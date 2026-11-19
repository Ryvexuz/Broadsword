//! The triangle rasterizer: near-plane clipping, exact fixed-point edge functions with a top-left fill rule,
//! a depth buffer, back-face culling, Lambert shading with one directional light, and a shadow map.
//! See README.md for the pipeline and for the shadow bias.

use crate::camera::Camera;
use crate::framebuffer::Framebuffer;
use crate::math::{Mat4, Vec3};
use crate::mesh::Mesh;
use std::fmt;
use std::time::Instant;

/// Screen coordinates are rounded to 1/16 of a pixel before filling, so the edge functions are exact integers
/// and a shared edge is evaluated identically by both triangles that own it.
const SUBPIXEL: i64 = 16;

/// Screen coordinates beyond this many pixels are clamped. It keeps every edge-function product inside `i64`
/// (2^24 pixels * 16 = 2^28; a difference is 2^29; a product 2^58), so it is a limit of the integer maths,
/// not a tuning knob. This is NOT clipping: a clamped vertex is moved, so the triangle's shape changes and it can
/// cover different pixels than the true triangle. Only triangles with a vertex beyond 2^24 pixels are affected.
const MAX_COORD: f32 = 16_777_216.0;

/// Which triangles `fill_triangle` draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    /// Only triangles whose vertices p[0] -> p[1] -> p[2] run counter-clockwise as seen on the screen (the edge
    /// function area is negative in pixel coordinates, y down). Triangles running clockwise on screen are not drawn.
    FrontOnly,
    /// Both. The weights handed to the callback are correct either way.
    Both,
}

/// Edge function of the edge s->t at point p, in 1/16 pixel units. Positive on the inside of a triangle
/// whose three edges are taken in the order that makes its area positive.
fn edge(s: (i64, i64), t: (i64, i64), p: (i64, i64)) -> i64 {
    (t.0 - s.0) * (p.1 - s.1) - (t.1 - s.1) * (p.0 - s.0)
}

/// A pixel exactly on an edge belongs to the triangle only if the edge is a top edge (horizontal, with the
/// triangle below it) or a left edge (the triangle to its right). Written for pixel coordinates, y down, and
/// edges in positive-area order. The two triangles sharing an edge walk it in opposite directions, so exactly
/// one of them owns it.
fn top_left(s: (i64, i64), t: (i64, i64)) -> bool {
    let (dx, dy) = (t.0 - s.0, t.1 - s.1);
    dy < 0 || (dy == 0 && dx > 0)
}

/// Fills one screen-space triangle on a `width` x `height` pixel grid. Pixel (x, y) is sampled at the screen
/// coordinate (x, y) itself, the same convention the line drawing uses. For each covered pixel it calls
/// `f(x, y, weights)`, where `weights` are the barycentric weights of `p[0]`, `p[1]`, `p[2]` (they sum to 1).
///
/// Returns `false` if the triangle was not drawn: a non-finite vertex, zero area (including edge-on), or
/// back-facing while `sides` is `FrontOnly`. Returns `true` otherwise, even if it covers no pixel.
pub fn fill_triangle(
    width: usize,
    height: usize,
    p: [(f32, f32); 3],
    sides: Sides,
    mut f: impl FnMut(usize, usize, [f32; 3]),
) -> bool {
    let mut q = [(0i64, 0i64); 3];
    for (i, &(x, y)) in p.iter().enumerate() {
        if !(x.is_finite() && y.is_finite()) {
            return false;
        }
        let s = SUBPIXEL as f32;
        q[i] = ((x.clamp(-MAX_COORD, MAX_COORD) * s).round() as i64, (y.clamp(-MAX_COORD, MAX_COORD) * s).round() as i64);
    }
    let area0 = edge(q[0], q[1], q[2]);
    if area0 == 0 || (area0 > 0 && sides == Sides::FrontOnly) {
        return false;
    }
    // Work with positive area: swap the last two vertices of a front-facing triangle.
    let order = if area0 > 0 { [0usize, 1, 2] } else { [0, 2, 1] };
    let (a, b, c) = (q[order[0]], q[order[1]], q[order[2]]);
    let inv_area = 1.0 / area0.abs() as f32;
    let tl = [top_left(b, c), top_left(c, a), top_left(a, b)];

    let ceil = |v: i64| (v + SUBPIXEL - 1).div_euclid(SUBPIXEL);
    let floor = |v: i64| v.div_euclid(SUBPIXEL);
    let x0 = ceil(a.0.min(b.0).min(c.0)).max(0);
    let x1 = floor(a.0.max(b.0).max(c.0)).min(width as i64 - 1);
    let y0 = ceil(a.1.min(b.1).min(c.1)).max(0);
    let y1 = floor(a.1.max(b.1).max(c.1)).min(height as i64 - 1);
    if x0 > x1 || y0 > y1 {
        return true;
    }

    let start = (x0 * SUBPIXEL, y0 * SUBPIXEL);
    let mut row = [edge(b, c, start), edge(c, a, start), edge(a, b, start)];
    let step_x = [-(c.1 - b.1) * SUBPIXEL, -(a.1 - c.1) * SUBPIXEL, -(b.1 - a.1) * SUBPIXEL];
    let step_y = [(c.0 - b.0) * SUBPIXEL, (a.0 - c.0) * SUBPIXEL, (b.0 - a.0) * SUBPIXEL];
    for y in y0..=y1 {
        let mut e = row;
        for x in x0..=x1 {
            if (e[0] > 0 || (e[0] == 0 && tl[0])) && (e[1] > 0 || (e[1] == 0 && tl[1])) && (e[2] > 0 || (e[2] == 0 && tl[2])) {
                let mut l = [0.0f32; 3];
                for k in 0..3 {
                    l[order[k]] = e[k] as f32 * inv_area;
                }
                f(x as usize, y as usize, l);
            }
            for k in 0..3 {
                e[k] += step_x[k];
            }
        }
        for k in 0..3 {
            row[k] += step_y[k];
        }
    }
    true
}

/// The directional light. `to_light` points from the surface toward the light (it is normalised for you).
/// A pixel's brightness is `ambient + diffuse * max(N.L, 0)`, times the base colour, per channel, with no gamma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub to_light: Vec3,
    pub ambient: f32,
    pub diffuse: f32,
}

/// Shadow-map settings. The map is an orthographic depth image of the casters seen from the light, looking at a
/// sphere of `focus_radius` around `focus_center`; anything outside the sphere's cross-section is not shadowed.
///
/// A pixel is shadowed if its distance from the light, minus a bias, is greater than the map's value. The bias
/// is in world units: `texel * (bias_const_texels + bias_slope * min(tan(angle), max_slope))`, where `texel` is
/// the world size of one map texel (`2 * focus_radius / map_size`) and `angle` is between the triangle's true
/// normal and the light. See README.md, "Shadow bias".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    pub map_size: usize,
    pub focus_center: Vec3,
    pub focus_radius: f32,
    pub bias_const_texels: f32,
    pub bias_slope: f32,
    pub max_slope: f32,
}

/// Chosen bias values: see README.md for why.
pub const BIAS_CONST_TEXELS: f32 = 1.0;
pub const BIAS_SLOPE: f32 = 1.0;
pub const BIAS_MAX_SLOPE: f32 = 8.0;

impl ShadowSettings {
    /// A map of `map_size` x `map_size` texels around a sphere, with the documented bias values.
    pub fn new(map_size: usize, focus_center: Vec3, focus_radius: f32) -> ShadowSettings {
        ShadowSettings {
            map_size,
            focus_center,
            focus_radius,
            bias_const_texels: BIAS_CONST_TEXELS,
            bias_slope: BIAS_SLOPE,
            max_slope: BIAS_MAX_SLOPE,
        }
    }
}

/// One mesh placed in the world. `model` must be a rotation and translation, with at most a uniform scale
/// (normals are rotated by it and renormalised).
#[derive(Clone, Copy, Debug)]
pub struct Object<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    /// Base colour `0x00RRGGBB`.
    pub color: u32,
    /// Whether the object is drawn into the shadow map. Every object receives shadows.
    pub casts_shadow: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Scene<'a> {
    pub objects: &'a [Object<'a>],
    pub light: Light,
    /// `None` turns shadows off (and skips the shadow pass).
    pub shadow: Option<ShadowSettings>,
    pub background: u32,
}

/// The centre and radius of a sphere around every vertex of the objects that cast shadows (`None` if there are none).
/// The centre is the middle of the bounding box, which is not the tightest sphere but is simple and stable.
pub fn caster_sphere(objects: &[Object]) -> Option<(Vec3, f32)> {
    let pts: Vec<Vec3> = objects
        .iter()
        .filter(|o| o.casts_shadow)
        .flat_map(|o| o.mesh.positions.iter().map(move |&p| o.model.transform_point3(p)))
        .collect();
    let first = *pts.first()?;
    let (mut lo, mut hi) = (first, first);
    for p in &pts {
        lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    let centre = (lo + hi) * 0.5;
    let radius = pts.iter().map(|&p| (p - centre).length()).fold(0.0f32, f32::max);
    Some((centre, radius))
}

/// Why a scene could not be rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    BadLight(String),
    BadShadow(String),
    BadMesh { object: usize, reason: String },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::BadLight(s) => write!(f, "bad light: {s}"),
            RenderError::BadShadow(s) => write!(f, "bad shadow settings: {s}"),
            RenderError::BadMesh { object, reason } => write!(f, "object {object} has a bad mesh: {reason}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// What the last `render` did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Triangles in all objects.
    pub triangles_submitted: usize,
    /// Triangles drawn into the shadow map (shadow casters, not degenerate).
    pub triangles_shadow: usize,
    /// Triangles (or pieces left after near-plane clipping) that survived culling in the main pass.
    pub triangles_main: usize,
    pub shadow_ms: f64,
    pub main_ms: f64,
}

#[derive(Clone, Copy)]
struct Vtx {
    clip: [f32; 4],
    /// Diffuse term, then the shadow-map position (u, v) in texels and the distance from the light.
    attr: [f32; 4],
}

#[derive(Clone, Copy)]
struct Screen {
    x: f32,
    y: f32,
    z: f32,
    inv_w: f32,
}

/// The light's view of the world: an orthographic projection along `-to_light`.
#[derive(Clone, Copy)]
struct LightSpace {
    centre: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    inv_radius: f32,
    size: f32,
}

impl LightSpace {
    fn new(s: &ShadowSettings, to_light: Vec3) -> LightSpace {
        let forward = -to_light;
        // Any axis that is not nearly parallel to the light direction: so a light pointing straight down works.
        let helper = if forward.y.abs() < 0.99 { Vec3::new(0.0, 1.0, 0.0) } else { Vec3::new(0.0, 0.0, 1.0) };
        let right = forward.cross(helper).normalized();
        let up = right.cross(forward);
        LightSpace { centre: s.focus_center, right, up, forward, inv_radius: 1.0 / s.focus_radius, size: s.map_size as f32 }
    }

    /// (u, v, distance): u and v in texels (a texel's centre is at its integer coordinate), distance from the
    /// focus centre along the light direction.
    fn map(&self, p: Vec3) -> (f32, f32, f32) {
        let r = p - self.centre;
        (
            (r.dot(self.right) * self.inv_radius * 0.5 + 0.5) * self.size,
            (0.5 - r.dot(self.up) * self.inv_radius * 0.5) * self.size,
            r.dot(self.forward),
        )
    }
}

/// Orders vertices by clip coordinates (x, y, z, w), then by attributes: a total order for finite numbers.
fn clip_key_less(a: &Vtx, b: &Vtx) -> bool {
    for k in 0..4 {
        if a.clip[k] != b.clip[k] {
            return a.clip[k] < b.clip[k];
        }
    }
    for k in 0..4 {
        if a.attr[k] != b.attr[k] {
            return a.attr[k] < b.attr[k];
        }
    }
    false
}

/// Clips a triangle against the near plane (z >= -w) in clip space, before the perspective divide.
/// Returns the number of vertices of the resulting convex polygon (0, 3 or 4).
fn clip_near(tri: &[Vtx; 3], out: &mut [Vtx; 4]) -> usize {
    let dist = |v: &Vtx| v.clip[2] + v.clip[3];
    let mut n = 0;
    for i in 0..3 {
        let (a, b) = (&tri[i], &tri[(i + 1) % 3]);
        let (da, db) = (dist(a), dist(b));
        if da >= 0.0 {
            out[n] = *a;
            n += 1;
        }
        if (da >= 0.0) != (db >= 0.0) {
            // Canonical order: interpolate from the endpoint that sorts first by clip coordinates, so a neighbour
            // that walks the shared edge the other way computes bit-for-bit the same crossing.
            let (p, q, dp, dq) = if clip_key_less(b, a) { (b, a, db, da) } else { (a, b, da, db) };
            let t = dp / (dp - dq);
            let mut v = *p;
            for k in 0..4 {
                v.clip[k] = p.clip[k] + (q.clip[k] - p.clip[k]) * t;
                v.attr[k] = p.attr[k] + (q.attr[k] - p.attr[k]) * t;
            }
            out[n] = v;
            n += 1;
        }
    }
    n
}

struct Target<'a> {
    pixels: &'a mut [u32],
    depth: &'a mut [f32],
    width: usize,
    height: usize,
}

struct ShadowView<'a> {
    map: &'a [f32],
    size: i64,
}

struct Material {
    base: [f32; 3],
    ambient: f32,
    diffuse: f32,
}

/// Clips, projects, culls and fills one triangle into the target. Returns how many pieces were drawn.
fn draw_triangle(target: &mut Target, tri: &[Vtx; 3], mat: &Material, shadow: Option<&ShadowView>, bias: f32) -> usize {
    let mut poly = [tri[0]; 4];
    let n = clip_near(tri, &mut poly);
    if n < 3 {
        return 0;
    }
    let (w, h) = (target.width, target.height);
    let mut s = [Screen { x: 0.0, y: 0.0, z: 0.0, inv_w: 0.0 }; 4];
    for i in 0..n {
        let c = poly[i].clip;
        if !(c[3] > 0.0) {
            return 0;
        }
        let inv_w = 1.0 / c[3];
        s[i] = Screen {
            x: (c[0] * inv_w * 0.5 + 0.5) * w as f32,
            y: (0.5 - c[1] * inv_w * 0.5) * h as f32,
            z: c[2] * inv_w,
            inv_w,
        };
    }
    let mut drawn = 0;
    for i in 1..n - 1 {
        let (sa, sb, sc) = (s[0], s[i], s[i + 1]);
        let (va, vb, vc) = (&poly[0], &poly[i], &poly[i + 1]);
        let Target { pixels, depth, .. } = target;
        let ok = fill_triangle(w, h, [(sa.x, sa.y), (sb.x, sb.y), (sc.x, sc.y)], Sides::FrontOnly, |x, y, l| {
            let idx = y * w + x;
            let z = l[0] * sa.z + l[1] * sb.z + l[2] * sc.z;
            // Depth test first, so a hidden pixel costs almost nothing. NaN fails it. Past the far plane is hidden.
            if !(z < depth[idx]) || z > 1.0 {
                return;
            }
            // Perspective-correct weights: screen-space weights divided by w, renormalised.
            let q = [l[0] * sa.inv_w, l[1] * sb.inv_w, l[2] * sc.inv_w];
            let inv = 1.0 / (q[0] + q[1] + q[2]);
            let attr = |k: usize| (q[0] * va.attr[k] + q[1] * vb.attr[k] + q[2] * vc.attr[k]) * inv;
            let mut lit = 1.0f32;
            if let Some(sv) = shadow {
                let (iu, iv) = ((attr(1) + 0.5).floor() as i64, (attr(2) + 0.5).floor() as i64);
                if iu >= 0 && iu < sv.size && iv >= 0 && iv < sv.size && attr(3) - bias > sv.map[(iv * sv.size + iu) as usize] {
                    lit = 0.0;
                }
            }
            let k = mat.ambient + attr(0) * mat.diffuse * lit;
            let ch = |b: f32| ((b * k) + 0.5).clamp(0.0, 255.0) as u32;
            depth[idx] = z;
            pixels[idx] = ch(mat.base[0]) << 16 | ch(mat.base[1]) << 8 | ch(mat.base[2]);
        });
        if ok {
            drawn += 1;
        }
    }
    drawn
}

/// Reusable renderer state: the depth buffer, the shadow map and scratch vectors. Rendering is a pure function
/// of the scene and camera: the same input gives the same pixels, whatever the renderer held before.
#[derive(Default)]
pub struct Renderer {
    depth: Vec<f32>,
    shadow_map: Vec<f32>,
    world: Vec<Vec3>,
    world_normals: Vec<Vec3>,
    vertices: Vec<Vtx>,
    stats: FrameStats,
}

fn validate(scene: &Scene) -> Result<(), RenderError> {
    let l = &scene.light;
    let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if !finite(l.to_light) || l.to_light.length() == 0.0 || !l.to_light.length().is_finite() {
        return Err(RenderError::BadLight("the light direction must be a finite, non-zero vector".into()));
    }
    if !l.ambient.is_finite() || !l.diffuse.is_finite() {
        return Err(RenderError::BadLight("ambient and diffuse must be finite".into()));
    }
    if let Some(s) = &scene.shadow {
        if s.map_size == 0 || s.map_size.checked_mul(s.map_size).map_or(true, |n| n > isize::MAX as usize / 4) {
            return Err(RenderError::BadShadow(format!("map size {} is invalid", s.map_size)));
        }
        if !(s.focus_radius > 0.0 && s.focus_radius.is_finite()) || !finite(s.focus_center) {
            return Err(RenderError::BadShadow("the focus needs a finite centre and a positive radius".into()));
        }
        if !(s.bias_const_texels.is_finite() && s.bias_slope.is_finite() && s.max_slope >= 0.0 && s.max_slope.is_finite()) {
            return Err(RenderError::BadShadow("bias values must be finite and max_slope not negative".into()));
        }
    }
    for (i, o) in scene.objects.iter().enumerate() {
        o.mesh.validate().map_err(|reason| RenderError::BadMesh { object: i, reason })?;
    }
    Ok(())
}

fn transform_object(obj: &Object, world: &mut Vec<Vec3>, normals: &mut Vec<Vec3>) {
    world.clear();
    world.extend(obj.mesh.positions.iter().map(|&p| obj.model.transform_point3(p)));
    normals.clear();
    normals.extend(obj.mesh.normals.iter().map(|&n| obj.model.transform_dir(n).normalized()));
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer::default()
    }

    /// What the last `render` did.
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    /// Clears `fb` to the scene background, then draws the scene seen by `cam`. The camera's aspect should be
    /// the framebuffer's width over height.
    pub fn render(&mut self, fb: &mut Framebuffer, cam: &Camera, scene: &Scene) -> Result<FrameStats, RenderError> {
        validate(scene)?;
        let (w, h) = (fb.width(), fb.height());
        let to_light = scene.light.to_light.normalized();
        let Renderer { depth, shadow_map, world, world_normals, vertices, stats } = self;
        *stats = FrameStats::default();
        stats.triangles_submitted = scene.objects.iter().map(|o| o.mesh.triangles.len()).sum();
        depth.clear();
        depth.resize(w * h, f32::INFINITY);
        fb.clear(scene.background);

        // Shadow pass: nearest distance from the light, per texel.
        let started = Instant::now();
        let light_space = scene.shadow.as_ref().map(|s| LightSpace::new(s, to_light));
        if let (Some(s), Some(ls)) = (&scene.shadow, &light_space) {
            let size = s.map_size;
            shadow_map.clear();
            shadow_map.resize(size * size, f32::INFINITY);
            for obj in scene.objects.iter().filter(|o| o.casts_shadow) {
                transform_object(obj, world, world_normals);
                let mapped: Vec<(f32, f32, f32)> = world.iter().map(|&p| ls.map(p)).collect();
                for t in &obj.mesh.triangles {
                    let m = [mapped[t[0] as usize], mapped[t[1] as usize], mapped[t[2] as usize]];
                    let drawn = fill_triangle(size, size, [(m[0].0, m[0].1), (m[1].0, m[1].1), (m[2].0, m[2].1)], Sides::Both, |x, y, l| {
                        let d = l[0] * m[0].2 + l[1] * m[1].2 + l[2] * m[2].2;
                        let cell = &mut shadow_map[y * size + x];
                        if d < *cell {
                            *cell = d;
                        }
                    });
                    if drawn {
                        stats.triangles_shadow += 1;
                    }
                }
            }
        }
        stats.shadow_ms = started.elapsed().as_secs_f64() * 1000.0;

        // Main pass.
        let started = Instant::now();
        let view_proj = cam.view_proj();
        let shadow_view = scene.shadow.as_ref().map(|s| ShadowView { map: shadow_map.as_slice(), size: s.map_size as i64 });
        let mut target = Target { pixels: fb.pixels_mut(), depth: depth.as_mut_slice(), width: w, height: h };
        for obj in scene.objects {
            transform_object(obj, world, world_normals);
            let smooth = !obj.mesh.normals.is_empty();
            let material = Material {
                base: [(obj.color >> 16 & 0xFF) as f32, (obj.color >> 8 & 0xFF) as f32, (obj.color & 0xFF) as f32],
                ambient: scene.light.ambient,
                diffuse: scene.light.diffuse,
            };
            vertices.clear();
            for (i, &p) in world.iter().enumerate() {
                let diffuse = if smooth { world_normals[i].dot(to_light).max(0.0) } else { 0.0 };
                let (u, v, d) = light_space.map_or((0.0, 0.0, 0.0), |ls| ls.map(p));
                vertices.push(Vtx { clip: view_proj.transform_point(p), attr: [diffuse, u, v, d] });
            }
            for t in &obj.mesh.triangles {
                let ids = [t[0] as usize, t[1] as usize, t[2] as usize];
                let normal = (world[ids[1]] - world[ids[0]]).cross(world[ids[2]] - world[ids[0]]);
                let len = normal.length();
                if !(len > 0.0) {
                    continue; // zero area in the world
                }
                let geometric = normal * (1.0 / len);
                let mut tri = [vertices[ids[0]], vertices[ids[1]], vertices[ids[2]]];
                if !smooth {
                    let diffuse = geometric.dot(to_light).max(0.0);
                    for v in &mut tri {
                        v.attr[0] = diffuse;
                    }
                }
                let bias = match &scene.shadow {
                    Some(s) => {
                        let cos = geometric.dot(to_light).abs();
                        let sin = (1.0 - cos * cos).max(0.0).sqrt();
                        let slope = (sin / cos).min(s.max_slope);
                        2.0 * s.focus_radius / s.map_size as f32 * (s.bias_const_texels + s.bias_slope * slope)
                    }
                    None => 0.0,
                };
                stats.triangles_main += draw_triangle(&mut target, &tri, &material, shadow_view.as_ref(), bias);
            }
        }
        stats.main_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(*stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(v: &Vtx) -> [u32; 8] {
        let mut o = [0u32; 8];
        for k in 0..4 {
            o[k] = v.clip[k].to_bits();
            o[4 + k] = v.attr[k].to_bits();
        }
        o
    }

    /// The vertices of the clipped polygon that are not one of the three inputs: the near-plane crossings.
    fn crossings(tri: &[Vtx; 3]) -> Vec<[u32; 8]> {
        let mut out = [tri[0]; 4];
        let n = clip_near(tri, &mut out);
        out[..n].iter().filter(|v| !tri.iter().any(|t| bits(t) == bits(v))).map(bits).collect()
    }

    #[test]
    fn clip_near_gives_a_shared_edge_the_same_crossing_from_either_end() {
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut next = move |lo: f32, hi: f32| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lo + (hi - lo) * ((state >> 40) as f32 / (1u64 << 24) as f32)
        };
        let mut vtx = |inside: bool| {
            let w = next(0.2, 5.0);
            let z = if inside { next(-0.9, 1.0) * w } else { next(-3.0, -1.01) * w };
            Vtx { clip: [next(-2.0, 2.0), next(-2.0, 2.0), z, w], attr: [next(0.0, 1.0), next(0.0, 64.0), next(0.0, 64.0), next(0.0, 9.0)] }
        };
        let mut compared = 0;
        for _ in 0..20_000 {
            let (p, q) = (vtx(true), vtx(false));
            // Two triangles on the edge p-q, which walk it in opposite directions (as neighbours do).
            let third_a = vtx(true);
            let third_b = vtx(true);
            let ca = crossings(&[p, q, third_a]);
            let cb = crossings(&[q, p, third_b]);
            // Each triangle has one other crossing (on the edge to its third vertex); the shared one is in both lists.
            assert!(ca.iter().any(|c| cb.contains(c)), "p-q crossing differs by direction: {ca:?} vs {cb:?}");
            compared += 1;
        }
        assert_eq!(compared, 20_000);
    }
}

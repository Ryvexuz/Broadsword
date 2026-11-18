//! Shared by the rasterizer tests: golden-image checking (same rules as tests/golden.rs) and an independent
//! model of the rasterizer written in plain f64 and exact integers, so a golden image is never the only evidence.
#![allow(dead_code)]

use render::{Camera, Framebuffer};
use std::fs;
use std::path::PathBuf;

pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

/// Compares `fb` byte for byte with `tests/golden/<name>.ppm`. On a mismatch or a missing file it writes
/// `<name>.actual.ppm` (git-ignored) and fails; accepting is always by hand.
pub fn check(name: &str, fb: &Framebuffer) {
    let golden = golden_dir().join(format!("{name}.ppm"));
    let actual_path = golden_dir().join(format!("{name}.actual.ppm"));
    let actual = fb.to_ppm();
    let Ok(expected_bytes) = fs::read(&golden) else {
        fs::create_dir_all(golden_dir()).unwrap();
        fs::write(&actual_path, &actual).unwrap();
        panic!(
            "golden image {name}.ppm is missing. The rendered image was written to {}. After checking it, accept it \
             by hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
            actual_path.display()
        );
    };
    if expected_bytes == actual {
        let _ = fs::remove_file(&actual_path);
        return;
    }
    fs::write(&actual_path, &actual).unwrap();
    let detail = match Framebuffer::from_ppm(&expected_bytes) {
        Ok(e) if e.width() == fb.width() && e.height() == fb.height() => {
            let n = e.pixels().iter().zip(fb.pixels()).filter(|(a, b)| a != b).count();
            format!("{n} pixels differ")
        }
        Ok(e) => format!("size differs: golden is {}x{}, actual is {}x{}", e.width(), e.height(), fb.width(), fb.height()),
        Err(e) => format!("the golden file cannot be read: {e}"),
    };
    panic!(
        "golden mismatch for {name}: {detail}. Actual image written to {}. If the change is intended, accept it by \
         hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
        actual_path.display()
    );
}

pub fn camera(position: render::Vec3, yaw: f32, pitch: f32, w: usize, h: usize) -> Camera {
    Camera {
        position,
        yaw,
        pitch,
        fov_y: 60f32.to_radians(),
        aspect: w as f32 / h as f32,
        near: 0.1,
        far: 100.0,
        move_speed: 1.0,
        turn_speed: 1.0,
    }
}

/// A camera at `position` looking at `target`.
pub fn look_at(position: render::Vec3, target: render::Vec3, w: usize, h: usize) -> Camera {
    let f = (target - position).normalized();
    camera(position, f.x.atan2(-f.z), f.y.asin(), w, h)
}

// ---------------------------------------------------------------------------------------------------------
// The independent model. Nothing below uses the crate's matrices, clipping, or fill code.

/// 1/16-pixel fixed point, rounding half away from zero: the same snapping the rasterizer documents.
pub fn snap(v: f64) -> i64 {
    (v * 16.0).round() as i64
}

/// Is pixel (x, y) inside the convex polygon `poly` (1/16-pixel integer vertices, either winding), under the top-left
/// rule? Done by the "infinitesimal shift" definition instead of the rasterizer's bias form: the sample point is
/// moved by (+d, +d^2) with d -> 0, and must then be strictly inside every edge. An edge value is therefore
/// compared as the triple (E, -dy, dx), lexicographically.
pub fn inside_ref(poly: &[(i64, i64)], x: i64, y: i64) -> bool {
    let n = poly.len();
    let area2: i128 = (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.0 as i128 * b.1 as i128 - b.0 as i128 * a.1 as i128
        })
        .sum();
    if area2 == 0 {
        return false;
    }
    let sign: i128 = if area2 > 0 { 1 } else { -1 };
    // `area2 > 0` means counter-clockwise in the (x right, y up) reading of these numbers. The interior is on
    // the left of each edge there, so the edge value is the cross product (t - s) x (p - s).
    let p = (x as i128 * 16, y as i128 * 16);
    (0..n).all(|i| {
        let (s, t) = (poly[i], poly[(i + 1) % n]);
        let (dx, dy) = ((t.0 - s.0) as i128, (t.1 - s.1) as i128);
        let e = dx * (p.1 - s.1 as i128) - dy * (p.0 - s.0 as i128);
        // The point p + (d, d^2): e changes by -dy*d + dx*d^2.
        let key = [sign * e, sign * -dy, sign * dx];
        key.iter().find(|&&k| k != 0).map_or(false, |&k| k > 0)
    })
}

/// Number of pixels of a `w` x `h` grid inside the polygon (given in pixel coordinates).
pub fn count_ref(w: usize, h: usize, poly_px: &[(f64, f64)]) -> usize {
    let poly: Vec<(i64, i64)> = poly_px.iter().map(|&(x, y)| (snap(x), snap(y))).collect();
    let mut n = 0;
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if inside_ref(&poly, x, y) {
                n += 1;
            }
        }
    }
    n
}

pub fn pixels_ref(w: usize, h: usize, poly_px: &[(f64, f64)]) -> Vec<bool> {
    let poly: Vec<(i64, i64)> = poly_px.iter().map(|&(x, y)| (snap(x), snap(y))).collect();
    let mut out = vec![false; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            out[y as usize * w + x as usize] = inside_ref(&poly, x, y);
        }
    }
    out
}

/// A camera as plain f64 maths: world -> (screen x, screen y, depth along the view direction).
pub struct RefCamera {
    pos: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    fwd: [f64; 3],
    tan_half: f64,
    aspect: f64,
    pub near: f64,
    pub w: f64,
    pub h: f64,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: [f64; 3]) -> [f64; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

impl RefCamera {
    pub fn new(c: &Camera, w: usize, h: usize) -> RefCamera {
        let (yaw, pitch) = (c.yaw as f64, c.pitch as f64);
        let fwd = [yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()];
        let right = [yaw.cos(), 0.0, yaw.sin()];
        let up = cross(right, fwd);
        RefCamera {
            pos: [c.position.x as f64, c.position.y as f64, c.position.z as f64],
            right,
            up,
            fwd,
            tan_half: (c.fov_y as f64 / 2.0).tan(),
            aspect: c.aspect as f64,
            near: c.near as f64,
            w: w as f64,
            h: h as f64,
        }
    }

    /// (view x, view y, depth) of a world point.
    pub fn to_view(&self, p: [f64; 3]) -> [f64; 3] {
        let d = sub(p, self.pos);
        [dot(d, self.right), dot(d, self.up), dot(d, self.fwd)]
    }

    /// Screen position of a point given in view space (depth must be positive).
    pub fn view_to_screen(&self, v: [f64; 3]) -> (f64, f64) {
        let nx = v[0] / (v[2] * self.tan_half * self.aspect);
        let ny = v[1] / (v[2] * self.tan_half);
        ((nx * 0.5 + 0.5) * self.w, (0.5 - ny * 0.5) * self.h)
    }

    pub fn project(&self, p: [f64; 3]) -> (f64, f64) {
        self.view_to_screen(self.to_view(p))
    }

    /// The world direction of the ray through pixel (x, y), sampled at the pixel's integer coordinate.
    pub fn ray(&self, x: usize, y: usize) -> [f64; 3] {
        let nx = x as f64 / self.w * 2.0 - 1.0;
        let ny = 1.0 - y as f64 / self.h * 2.0;
        let (vx, vy) = (nx * self.tan_half * self.aspect, ny * self.tan_half);
        [
            self.right[0] * vx + self.up[0] * vy + self.fwd[0],
            self.right[1] * vx + self.up[1] * vy + self.fwd[1],
            self.right[2] * vx + self.up[2] * vy + self.fwd[2],
        ]
    }

    pub fn position(&self) -> [f64; 3] {
        self.pos
    }

    /// Clips a world-space triangle against depth >= near (Sutherland-Hodgman in view space) and returns the
    /// screen-space polygon. Empty if nothing is in front.
    pub fn clipped_polygon(&self, tri: [[f64; 3]; 3]) -> Vec<(f64, f64)> {
        let v: Vec<[f64; 3]> = tri.iter().map(|&p| self.to_view(p)).collect();
        let mut out: Vec<[f64; 3]> = Vec::new();
        for i in 0..3 {
            let (a, b) = (v[i], v[(i + 1) % 3]);
            let (ina, inb) = (a[2] >= self.near, b[2] >= self.near);
            if ina {
                out.push(a);
            }
            if ina != inb {
                let t = (self.near - a[2]) / (b[2] - a[2]);
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, self.near]);
            }
        }
        out.iter().map(|&p| self.view_to_screen(p)).collect()
    }
}

/// Decodes a `0x00RRGGBB` pixel.
pub fn rgb(p: u32) -> [i32; 3] {
    [(p >> 16 & 0xFF) as i32, (p >> 8 & 0xFF) as i32, (p & 0xFF) as i32]
}

/// `base * k` per channel, rounded, as the shading formula says.
pub fn shade(base: u32, k: f64) -> [i32; 3] {
    let b = rgb(base);
    [0, 1, 2].map(|i| ((b[i] as f64 * k).round() as i32).clamp(0, 255))
}

pub fn near(a: [i32; 3], b: [i32; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

// ---------------------------------------------------------------------------------------------------------
// Scene helpers.

use render::{FrameStats, Light, Mat4, Mesh, Object, Renderer, Scene, ShadowSettings, Vec3};

pub fn render_scene(
    w: usize,
    h: usize,
    cam: &Camera,
    objects: &[Object],
    light: Light,
    shadow: Option<ShadowSettings>,
    background: u32,
) -> (Framebuffer, FrameStats) {
    let mut fb = Framebuffer::new(w, h).unwrap();
    let mut r = Renderer::new();
    let scene = Scene { objects, light, shadow, background };
    let stats = r.render(&mut fb, cam, &scene).expect("the scene renders");
    (fb, stats)
}

/// One triangle with no normals.
pub fn tri_mesh(p: [[f32; 3]; 3]) -> Mesh {
    Mesh { positions: p.iter().map(|q| Vec3::new(q[0], q[1], q[2])).collect(), normals: vec![], triangles: vec![[0, 1, 2]] }
}

/// Light that gives every surface exactly its base colour.
pub fn ambient_only() -> Light {
    Light { to_light: Vec3::new(0.0, 1.0, 0.0), ambient: 1.0, diffuse: 0.0 }
}

pub fn object(mesh: &Mesh, color: u32) -> Object<'_> {
    Object { mesh, model: Mat4::IDENTITY, color, casts_shadow: false }
}

/// Convex hull (counter-clockwise in the usual x right, y up reading) by the monotone chain.
pub fn hull(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut p = points.to_vec();
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Signed distance from `p` to the hull's boundary: positive inside, negative outside (to the nearest edge line).
pub fn hull_margin(h: &[(f64, f64)], p: (f64, f64)) -> f64 {
    let n = h.len();
    (0..n)
        .map(|i| {
            let (a, b) = (h[i], h[(i + 1) % n]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            ((b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)) / len
        })
        .fold(f64::INFINITY, f64::min)
}

/// What `check_ground` found.
pub struct GroundReport {
    pub shadow_checked: usize,
    pub lit_checked: usize,
    pub skipped: usize,
    pub bad: Vec<String>,
}

/// For every pixel whose view ray meets the ground plane y = 0 inside `half`, asks `expect(x, z)` whether that
/// ground point must be in shadow (`Some(true)`), lit (`Some(false)`) or is not to be judged (`None`), and
/// compares the pixel's colour with the lit or shadow colour. `skip(px, py, ray, t)` drops pixels where
/// something other than the ground is in front.
pub fn check_ground(
    fb: &Framebuffer,
    rc: &RefCamera,
    half: f64,
    lit: [i32; 3],
    shadow: [i32; 3],
    expect: &dyn Fn(f64, f64) -> Option<bool>,
    skip: &dyn Fn(usize, usize, [f64; 3], f64) -> bool,
) -> GroundReport {
    let mut r = GroundReport { shadow_checked: 0, lit_checked: 0, skipped: 0, bad: Vec::new() };
    let pos = rc.position();
    for y in 0..fb.height() {
        for x in 0..fb.width() {
            let ray = rc.ray(x, y);
            if ray[1] >= -1e-9 {
                continue;
            }
            let t = -pos[1] / ray[1];
            let g = (pos[0] + ray[0] * t, pos[2] + ray[2] * t);
            if g.0.abs() > half - 0.05 || g.1.abs() > half - 0.05 {
                continue;
            }
            let want = expect(g.0, g.1);
            if want.is_none() || skip(x, y, ray, t) {
                r.skipped += 1;
                continue;
            }
            let want = want.unwrap();
            let got = rgb(fb.get(x as i32, y as i32).unwrap());
            if want {
                r.shadow_checked += 1;
            } else {
                r.lit_checked += 1;
            }
            if !near(got, if want { shadow } else { lit }, 1) {
                r.bad.push(format!(
                    "pixel ({x},{y}) ground ({:.3},{:.3}): must be {} but is {got:?}",
                    g.0,
                    g.1,
                    if want { "shadow" } else { "lit" }
                ));
            }
        }
    }
    r
}

//! Rasterizer tests: the fill rule, depth, near-plane clipping, shading and the car with its shadow.
//! Every golden image is preceded by checks against the independent model in `common` (plain f64 maths and
//! exact integers), so the golden file only locks in a picture that has already been shown to be right.

mod common;

use common::*;
use render::{
    car_body_mesh, car_wheel_centres, caster_sphere, fill_triangle, ground_plane_mesh, wheel_mesh, Camera, Framebuffer,
    Light, Mat4, Mesh, Object, ShadowSettings, Sides, Vec3,
};

type P = (f32, f32);

/// How many times each pixel of a `w` x `h` grid is covered by these screen triangles.
fn coverage(w: usize, h: usize, tris: &[[P; 3]]) -> Vec<u8> {
    let mut c = vec![0u8; w * h];
    for t in tris {
        fill_triangle(w, h, *t, Sides::Both, |x, y, _| c[y * w + x] += 1);
    }
    c
}

fn as_f64(t: [P; 3]) -> Vec<(f64, f64)> {
    t.iter().map(|&(x, y)| (x as f64, y as f64)).collect()
}

// ------------------------------------------------------------------------------------------ the fill rule

#[test]
fn two_triangles_sharing_an_edge_cover_every_pixel_exactly_once() {
    let (w, h) = (96, 72);
    let (a, b, c, d) = ((10.0, 8.0), (70.0, 14.0), (82.0, 60.0), (14.0, 52.0));
    let quad = vec![(10.0, 8.0), (70.0, 14.0), (82.0, 60.0), (14.0, 52.0)];
    let want_quad = count_ref(w, h, &quad);
    for (name, t1, t2, on_edge) in [
        // The diagonal a-c passes exactly through these pixel centres.
        ("a-c", [a, b, c], [a, c, d], vec![(28usize, 21usize), (46, 34), (64, 47)]),
        // The diagonal b-d passes exactly through (42, 33).
        ("b-d", [a, b, d], [b, c, d], vec![(42, 33)]),
    ] {
        let cov = coverage(w, h, &[t1, t2]);
        let doubled = cov.iter().filter(|&&n| n > 1).count();
        let got: usize = cov.iter().map(|&n| n as usize).sum();
        let (r1, r2) = (count_ref(w, h, &as_f64(t1)), count_ref(w, h, &as_f64(t2)));
        println!("shared edge {name}: triangle 1 = {r1} px, triangle 2 = {r2} px, drawn total = {got}, quad = {want_quad}, doubled = {doubled}");
        assert_eq!(doubled, 0, "{name}: a pixel was drawn twice");
        assert_eq!(got, r1 + r2, "{name}: drawn pixels must equal the sum of the exact counts");
        assert_eq!(got, want_quad, "{name}: and the exact count of the whole quad");
        // Each triangle alone matches its own exact count, pixel for pixel.
        for t in [t1, t2] {
            let want = pixels_ref(w, h, &as_f64(t));
            let own = coverage(w, h, &[t]);
            assert!(own.iter().zip(&want).all(|(&c, &m)| (c == 1) == m), "{name}: a single triangle differs from the model");
        }
        for (x, y) in on_edge {
            assert_eq!(cov[y * w + x], 1, "{name}: pixel ({x},{y}) lies exactly on the shared edge and must be drawn once");
        }
    }
}

#[test]
fn right_triangles_on_a_square_have_the_closed_form_pixel_counts() {
    // The square [0,n) x [0,n) cut along its diagonal. With the top-left rule the upper-left triangle owns its top
    // and left edges: n(n+1)/2 pixels. The lower-right one gets n(n-1)/2. Together n*n, with none doubled.
    let n = 40usize;
    let nf = n as f32;
    let upper_left = [(0.0, 0.0), (nf, 0.0), (0.0, nf)];
    let lower_right = [(nf, 0.0), (nf, nf), (0.0, nf)];
    let (w, h) = (n + 8, n + 8);
    let c1: usize = coverage(w, h, &[upper_left]).iter().map(|&v| v as usize).sum();
    let c2: usize = coverage(w, h, &[lower_right]).iter().map(|&v| v as usize).sum();
    println!("right triangles n={n}: {c1} + {c2} = {}", c1 + c2);
    assert_eq!(c1, n * (n + 1) / 2);
    assert_eq!(c2, n * (n - 1) / 2);
    assert_eq!(c1 + c2, n * n);
}

#[test]
fn winding_decides_culling_but_not_coverage() {
    let (w, h) = (32, 32);
    let t = [(3.0, 3.0), (3.0, 25.0), (28.0, 6.5)]; // clockwise as drawn on screen (y down): a front face
    let rev = [t[0], t[2], t[1]];
    assert!(fill_triangle(w, h, t, Sides::FrontOnly, |_, _, _| {}), "the front face is drawn");
    assert!(!fill_triangle(w, h, rev, Sides::FrontOnly, |_, _, _| panic!("a back face must not be drawn")));
    // With Both, either winding covers the same pixels, with weights that still sum to 1 and are not negative.
    assert_eq!(coverage(w, h, &[t]), coverage(w, h, &[rev]));
    fill_triangle(w, h, rev, Sides::Both, |_, _, l| {
        assert!((l[0] + l[1] + l[2] - 1.0).abs() < 1e-5 && l.iter().all(|&v| v >= 0.0), "{l:?}");
    });
}

#[test]
fn degenerate_and_extreme_triangles_do_not_draw_or_panic() {
    let (w, h) = (16, 16);
    let mut hits = 0;
    let mut count = |_: usize, _: usize, _: [f32; 3]| hits += 1;
    assert!(!fill_triangle(w, h, [(1.0, 1.0), (5.0, 5.0), (9.0, 9.0)], Sides::Both, &mut count), "collinear");
    assert!(!fill_triangle(w, h, [(4.0, 4.0), (4.0, 4.0), (9.0, 2.0)], Sides::Both, &mut count), "a repeated vertex");
    assert!(!fill_triangle(w, h, [(2.0, 2.0), (f32::NAN, 5.0), (9.0, 9.0)], Sides::Both, &mut count), "NaN");
    assert!(!fill_triangle(w, h, [(2.0, 2.0), (f32::INFINITY, 5.0), (9.0, 9.0)], Sides::Both, &mut count), "infinity");
    assert!(fill_triangle(w, h, [(100.0, 100.0), (120.0, 100.0), (100.0, 130.0)], Sides::Both, &mut count), "wholly off screen");
    assert_eq!(hits, 0);
    // A triangle whose corners are far outside the grid still covers exactly what is inside it.
    let big = [(-1.0e6, -1.0e6), (1.0e6, -1.0e6), (0.0, 1.0e6)];
    let cov = coverage(w, h, &[big]);
    assert!(cov.iter().all(|&c| c == 1), "a huge triangle over the whole grid covers every pixel once");
    // Coordinates beyond the integer limit are clamped, not a panic.
    fill_triangle(w, h, [(-1.0e30, -1.0e30), (1.0e30, -1.0e30), (0.0, 1.0e30)], Sides::Both, |_, _, _| {});
    // A 1x1 grid.
    let cov = coverage(1, 1, &[[(-3.0, -3.0), (4.0, -3.0), (-3.0, 4.0)]]);
    assert_eq!(cov, vec![1]);
}

// -------------------------------------------------------------------------------------------- depth order

#[test]
fn golden_depth_order_near_triangle_hides_far_one() {
    let (w, h) = (120, 90);
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, w, h);
    let far_pts = [[-3.0, -2.5, -6.0], [3.0, -2.5, -6.0], [0.0, 3.5, -6.0]];
    let near_pts = [[-1.0, -1.2, -3.0], [2.5, -1.2, -3.0], [0.8, 1.4, -3.0]];
    let (far, near_m) = (tri_mesh(far_pts), tri_mesh(near_pts));
    let (far_c, near_c, bg) = (0x00C0_3020u32, 0x0020_60D0u32, 0x0010_1018u32);
    let draw = |order: &[usize]| {
        let objs = [object(&far, far_c), object(&near_m, near_c)];
        let list: Vec<Object> = order.iter().map(|&i| objs[i]).collect();
        render_scene(w, h, &cam, &list, ambient_only(), None, bg).0
    };
    let fb = draw(&[0, 1]);
    assert_eq!(fb, draw(&[1, 0]), "draw order must not change the picture");

    // The independent model: which pixels each triangle owns, and so what must be visible.
    let rc = RefCamera::new(&cam, w, h);
    let project = |pts: [[f64; 3]; 3]| -> Vec<(f64, f64)> { pts.iter().map(|&p| rc.project(p)).collect() };
    let f64s = |p: [[f32; 3]; 3]| p.map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]);
    let (in_far, in_near) = (pixels_ref(w, h, &project(f64s(far_pts))), pixels_ref(w, h, &project(f64s(near_pts))));
    let (mut n_near, mut n_far, mut n_hidden) = (0, 0, 0);
    for i in 0..w * h {
        let want = if in_near[i] {
            n_near += 1;
            near_c
        } else if in_far[i] {
            n_far += 1;
            far_c
        } else {
            bg
        };
        if in_near[i] && in_far[i] {
            n_hidden += 1;
        }
        assert_eq!(fb.pixels()[i], want, "pixel {} {}", i % w, i / w);
    }
    println!("depth order: near {n_near} px, far visible {n_far} px, far hidden behind near {n_hidden} px");
    assert!(n_hidden > 100 && n_far > 100 && n_near > 100, "the scene must exercise overlap and both triangles");
    check("depth_order", &fb);
}

// ---------------------------------------------------------------------------------------- near-plane clip

#[test]
fn golden_triangle_clipped_by_the_near_plane() {
    let (w, h) = (120, 90);
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, w, h);
    // A floor triangle 0.3 below the camera that starts in front of it and ends behind it. Counter-clockwise seen
    // from above, so it faces the camera.
    let pts = [[-0.6f32, -0.3, -1.0], [0.0, -0.3, 3.0], [0.6, -0.3, -1.0]];
    let mesh = tri_mesh(pts);
    let (fb, stats) = render_scene(w, h, &cam, &[object(&mesh, 0x0040_C060)], ambient_only(), None, 0x0010_1018);

    let rc = RefCamera::new(&cam, w, h);
    let poly = rc.clipped_polygon(pts.map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]));
    assert!(poly.len() >= 3, "the model says part of the triangle is in front");
    let want = pixels_ref(w, h, &poly);
    let n_want = want.iter().filter(|&&m| m).count();
    let n_got = fb.pixels().iter().filter(|&&p| p == 0x0040_C060).count();
    let differing: Vec<usize> = (0..w * h).filter(|&i| (fb.pixels()[i] == 0x0040_C060) != want[i]).collect();
    println!(
        "near clip: model polygon {:?}\n  exact pixels {n_want}, drawn {n_got}, differing {} ({} drawn pieces)",
        poly.iter().map(|&(x, y)| (x.round(), y.round())).collect::<Vec<_>>(),
        differing.len(),
        stats.triangles_main
    );
    assert!(differing.is_empty(), "pixels differ from the model at {:?}", &differing[..differing.len().min(8)]);
    assert!(n_want > 300, "enough of the triangle is visible to mean something");
    // The clipped edge lies on the near plane, so no pixel is drawn above the horizon (y = h/2) for a floor.
    assert!((0..w * h / 2 - w).all(|i| fb.pixels()[i] != 0x0040_C060), "nothing of the floor may appear above the horizon");
    check("near_clip", &fb);
}

#[test]
fn triangles_behind_the_camera_or_edge_on_draw_nothing() {
    let (w, h) = (40, 30);
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, w, h);
    let bg = 0x0010_1018;
    let cases: [(&str, [[f32; 3]; 3]); 5] = [
        ("wholly behind the camera", [[-1.0, -1.0, 3.0], [1.0, -1.0, 3.0], [0.0, 1.0, 5.0]]),
        ("behind, and facing away", [[-1.0, -1.0, 3.0], [0.0, 1.0, 5.0], [1.0, -1.0, 3.0]]),
        ("zero area", [[-1.0, 0.0, -3.0], [0.0, 0.0, -3.0], [1.0, 0.0, -3.0]]),
        ("a repeated vertex", [[-1.0, 0.0, -3.0], [-1.0, 0.0, -3.0], [1.0, 1.0, -3.0]]),
        // Exactly edge-on: the triangle lies in the plane y = 0 that contains the camera, so it projects to a line.
        ("edge-on", [[-1.0, 0.0, -2.0], [1.0, 0.0, -2.0], [0.0, 0.0, -4.0]]),
    ];
    for (name, pts) in cases {
        let mesh = tri_mesh(pts);
        let (fb, stats) = render_scene(w, h, &cam, &[object(&mesh, 0x00FF_FFFF)], ambient_only(), None, bg);
        assert!(fb.pixels().iter().all(|&p| p == bg), "{name}: something was drawn");
        assert_eq!(stats.triangles_main, 0, "{name}");
    }
    // A back face (clockwise seen from the camera) in front of it is culled too.
    let mesh = tri_mesh([[-1.0, -1.0, -3.0], [0.0, 1.0, -3.0], [1.0, -1.0, -3.0]]);
    let (fb, _) = render_scene(w, h, &cam, &[object(&mesh, 0x00FF_FFFF)], ambient_only(), None, bg);
    assert!(fb.pixels().iter().all(|&p| p == bg), "a back face must be culled");
    let mesh = tri_mesh([[-1.0, -1.0, -3.0], [1.0, -1.0, -3.0], [0.0, 1.0, -3.0]]);
    let (fb, _) = render_scene(w, h, &cam, &[object(&mesh, 0x00FF_FFFF)], ambient_only(), None, bg);
    assert!(fb.pixels().iter().any(|&p| p == 0x00FF_FFFF), "the same triangle, front facing, must draw");
}

// ----------------------------------------------------------------------------------------------- shading

fn uv_sphere(rings: usize, segs: usize) -> Mesh {
    let mut m = Mesh::default();
    for i in 0..=rings {
        let theta = std::f32::consts::PI * i as f32 / rings as f32;
        for j in 0..=segs {
            let phi = std::f32::consts::TAU * j as f32 / segs as f32;
            let p = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
            m.positions.push(p);
            m.normals.push(p);
        }
    }
    let at = |i: usize, j: usize| (i * (segs + 1) + j) as u32;
    for i in 0..rings {
        for j in 0..segs {
            m.triangles.push([at(i, j), at(i + 1, j + 1), at(i + 1, j)]);
            m.triangles.push([at(i, j), at(i, j + 1), at(i + 1, j + 1)]);
        }
    }
    m
}

#[test]
fn golden_shaded_sphere_brightness_follows_n_dot_l() {
    let (w, h) = (128, 128);
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, w, h);
    let sphere = uv_sphere(40, 64);
    sphere.validate().unwrap();
    let centre = [0.0, 0.0, -2.6];
    let model = Mat4::translation(Vec3::new(0.0, 0.0, -2.6));
    let to_light = Vec3::new(0.5, 0.6, 0.62).normalized();
    let light = Light { to_light, ambient: 0.2, diffuse: 0.8 };
    let obj = Object { mesh: &sphere, model, color: 0x00C8_C8C8, casts_shadow: false };
    let (fb, _) = render_scene(w, h, &cam, &[obj], light, None, 0x0010_1018);

    let rc = RefCamera::new(&cam, w, h);
    let l = norm([to_light.x as f64, to_light.y as f64, to_light.z as f64]);
    let (mut compared, mut max_err, mut max_err_lit) = (0, 0, 0);
    let (mut brightest, mut brightest_nl) = (0, -1.0);
    for y in 0..h {
        for x in 0..w {
            let d = norm(rc.ray(x, y));
            let oc = [-centre[0], -centre[1], -centre[2]];
            let b = dot(oc, d);
            let disc = b * b - (dot(oc, oc) - 1.0);
            // The disc of radius 0.93 of the sphere's outline: away from the faceted silhouette.
            if disc <= 1.0 - 0.93 * 0.93 {
                continue;
            }
            let t = -b - disc.sqrt();
            let hit = [d[0] * t, d[1] * t, d[2] * t];
            let n = norm([hit[0] - centre[0], hit[1] - centre[1], hit[2] - centre[2]]);
            let nl = dot(n, l);
            let want = shade(0x00C8_C8C8, 0.2 + 0.8 * nl.max(0.0));
            let got = rgb(fb.get(x as i32, y as i32).unwrap());
            // Gouraud on a 40 x 64 sphere is within a couple of levels; near the terminator the clamp at zero
            // is interpolated across a facet, which allows more.
            let tol = if nl.abs() < 0.12 { 10 } else { 3 };
            let err = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap();
            assert!(err <= tol, "pixel ({x},{y}): N.L = {nl:.3}, expected {want:?}, got {got:?}");
            if nl.abs() >= 0.12 {
                max_err_lit = max_err_lit.max(err);
            }
            max_err = max_err.max(err);
            compared += 1;
            if nl > brightest_nl && nl <= 1.0 {
                (brightest, brightest_nl) = (got[0], nl);
            }
            if nl < -0.12 {
                assert_eq!(got, shade(0x00C8_C8C8, 0.2), "a pixel facing away from the light shows exactly the ambient term");
            }
        }
    }
    println!("sphere: {compared} pixels compared; max error {max_err} levels ({max_err_lit} away from the terminator); brightest N.L {brightest_nl:.3} -> {brightest}");
    assert!(compared > 4000);
    assert!(brightest >= 195 && brightest_nl > 0.99, "the pixel facing the light is the brightest");
    check("sphere_shaded", &fb);
}

#[test]
fn a_mesh_without_normals_is_shaded_flat_per_face() {
    let (w, h) = (96, 96);
    let cam = look_at(Vec3::new(3.0, 2.5, 4.0), Vec3::new(0.0, 0.0, 0.0), w, h);
    // The fixture cube, with its normals thrown away: every face must still be lit by its own normal.
    let mut cube = Mesh::from_obj(include_str!("fixtures/cube.obj")).unwrap();
    cube.normals.clear();
    let to_light = Vec3::new(0.3, 0.8, 0.5).normalized();
    let light = Light { to_light, ambient: 0.25, diffuse: 0.75 };
    let (fb, _) = render_scene(w, h, &cam, &[object(&cube, 0x00C0_C0C0)], light, None, 0x0010_1018);
    let rc = RefCamera::new(&cam, w, h);
    let l = norm([to_light.x as f64, to_light.y as f64, to_light.z as f64]);
    let mut seen = Vec::new();
    for (name, centre, n) in [
        ("+X", [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ("+Y", [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]),
        ("+Z", [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
    ] {
        let (sx, sy) = rc.project(centre);
        let got = rgb(fb.get(sx.round() as i32, sy.round() as i32).unwrap());
        let want = shade(0x00C0_C0C0, 0.25 + 0.75 * dot(n, l).max(0.0));
        assert!(near(got, want, 1), "face {name}: expected {want:?}, got {got:?}");
        seen.push(got[0]);
    }
    assert!(seen[0] != seen[1] && seen[1] != seen[2], "the three faces have three different brightnesses: {seen:?}");
}

// ----------------------------------------------------------------------------------------- the car scene

const CAR_YAW: f32 = 0.436; // about 25 degrees
const CAR_POS: [f32; 3] = [0.3, 0.0, -0.2];
const GROUND_HALF: f32 = 12.0;
const GROUND: u32 = 0x0070_A060;
const AMBIENT: f32 = 0.25;
const DIFFUSE: f32 = 0.75;

fn car_light() -> Light {
    // From +X, above and from -Z: the shadow falls toward -X and +Z, which is toward the camera.
    Light { to_light: Vec3::new(0.5, 1.0, -0.6), ambient: AMBIENT, diffuse: DIFFUSE }
}

/// Boxes of the car in its own frame: (centre, half extents). The body and the cabin are solid boxes; the wheels
/// are cylinders, so for them the box is an upper bound.
fn car_boxes() -> (([f64; 3], [f64; 3]), ([f64; 3], [f64; 3]), Vec<([f64; 3], [f64; 3])>) {
    let body = ([0.0, 0.575, 0.0], [2.0, 0.275, 0.9]);
    let cabin = ([0.05, 1.1, 0.0], [0.85, 0.25, 0.75]);
    let wheels = car_wheel_centres().iter().map(|c| ([c.x as f64, c.y as f64, c.z as f64], [0.35, 0.35, 0.15])).collect();
    (body, cabin, wheels)
}

/// World corners of a box in the car's frame: rotated about Y by the car's yaw, then moved.
fn corners(b: ([f64; 3], [f64; 3])) -> Vec<[f64; 3]> {
    let (s, c) = (CAR_YAW as f64).sin_cos();
    let mut out = Vec::new();
    for sx in [-1.0, 1.0] {
        for sy in [-1.0, 1.0] {
            for sz in [-1.0, 1.0] {
                let (x, y, z) = (b.0[0] + sx * b.1[0], b.0[1] + sy * b.1[1], b.0[2] + sz * b.1[2]);
                out.push([c * x + s * z + CAR_POS[0] as f64, y, -s * x + c * z + CAR_POS[2] as f64]);
            }
        }
    }
    out
}

struct CarScene {
    body: Mesh,
    wheel: Mesh,
    ground: Mesh,
}

impl CarScene {
    fn new() -> CarScene {
        CarScene { body: car_body_mesh(), wheel: wheel_mesh(24).unwrap(), ground: ground_plane_mesh(GROUND_HALF) }
    }
    fn objects(&self) -> Vec<Object<'_>> {
        let car = Mat4::translation(Vec3::new(CAR_POS[0], CAR_POS[1], CAR_POS[2])).mul(&Mat4::rotation_y(CAR_YAW));
        let mut v = vec![
            Object { mesh: &self.ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: false },
            Object { mesh: &self.body, model: car, color: 0x00B0_2020, casts_shadow: true },
        ];
        for c in car_wheel_centres() {
            v.push(Object { mesh: &self.wheel, model: car.mul(&Mat4::translation(c)), color: 0x0020_2020, casts_shadow: true });
        }
        v
    }
}

fn car_camera(w: usize, h: usize) -> Camera {
    look_at(Vec3::new(-4.2, 3.0, 6.0), Vec3::new(0.3, 0.4, -0.2), w, h)
}

#[test]
fn golden_car_on_a_ground_plane_casts_its_shadow_away_from_the_light() {
    let (w, h) = (256, 192);
    let scene = CarScene::new();
    let objects = scene.objects();
    let (centre, radius) = caster_sphere(&objects).expect("the car casts shadows");
    let shadow = ShadowSettings::new(1024, centre, radius * 1.05);
    let cam = car_camera(w, h);
    let light = car_light();
    let bg = 0x0010_1018;
    let (fb, stats) = render_scene(w, h, &cam, &objects, light, Some(shadow), bg);
    println!("car: {} triangles submitted, {} in the shadow pass, {} in the main pass", stats.triangles_submitted, stats.triangles_shadow, stats.triangles_main);

    // --- independent maths: where must the ground be in shadow, and where lit?
    let l = norm([light.to_light.x as f64, light.to_light.y as f64, light.to_light.z as f64]);
    let lit = shade(GROUND, AMBIENT as f64 + DIFFUSE as f64 * l[1]);
    let dark = shade(GROUND, AMBIENT as f64);
    assert_ne!(lit, dark);
    let (body, cabin, wheels) = car_boxes();
    // The shadow of a solid box on y = 0 is the convex hull of its corners slid along the light to the ground.
    let on_ground = |pts: &[[f64; 3]]| -> Vec<(f64, f64)> {
        let p: Vec<(f64, f64)> = pts.iter().map(|q| (q[0] - l[0] / l[1] * q[1], q[2] - l[2] / l[1] * q[1])).collect();
        hull(&p)
    };
    let (body_shadow, cabin_shadow) = (on_ground(&corners(body)), on_ground(&corners(cabin)));
    let mut all = corners(body);
    all.extend(corners(cabin));
    for wb in &wheels {
        all.extend(corners(*wb));
    }
    let all_shadow = on_ground(&all);
    // The car's own outline on screen, so pixels showing the car are not judged as ground.
    let rc = RefCamera::new(&cam, w, h);
    let mut screen_hulls = vec![];
    for b in [body, cabin].iter().chain(wheels.iter()) {
        screen_hulls.push(hull(&corners(*b).iter().map(|&p| rc.project(p)).collect::<Vec<_>>()));
    }
    const M: f64 = 0.04; // metres of slack around a shadow edge; a map texel is about 5 mm
    let expect = |x: f64, z: f64| -> Option<bool> {
        if hull_margin(&body_shadow, (x, z)) > M || hull_margin(&cabin_shadow, (x, z)) > M {
            Some(true)
        } else if hull_margin(&all_shadow, (x, z)) < -M {
            Some(false)
        } else {
            None
        }
    };
    let skip = |px: usize, py: usize, _ray: [f64; 3], _t: f64| screen_hulls.iter().any(|h| hull_margin(h, (px as f64, py as f64)) > -1.5);
    let report = check_ground(&fb, &rc, GROUND_HALF as f64, lit, dark, &expect, &skip);
    println!(
        "car shadow: {} ground pixels must be shadow, {} must be lit, {} not judged, {} wrong",
        report.shadow_checked,
        report.lit_checked,
        report.skipped,
        report.bad.len()
    );
    assert!(report.bad.is_empty(), "{} wrong, first: {:?}", report.bad.len(), &report.bad[..report.bad.len().min(5)]);
    assert!(report.shadow_checked > 500 && report.lit_checked > 15000, "the check must cover a lot of both");

    // Control: the same check against a shadow slid 0.7 m toward the camera must fail on many pixels, or it
    // would prove nothing.
    let slid = |h: &[(f64, f64)]| -> Vec<(f64, f64)> { h.iter().map(|&(x, z)| (x - 0.7, z + 0.7)).collect() };
    let (bw, cw) = (slid(&body_shadow), slid(&cabin_shadow));
    let expect_wrong = |x: f64, z: f64| -> Option<bool> {
        if hull_margin(&bw, (x, z)) > M || hull_margin(&cw, (x, z)) > M {
            Some(true)
        } else {
            None
        }
    };
    let control = check_ground(&fb, &rc, GROUND_HALF as f64, lit, dark, &expect_wrong, &skip);
    println!("control with the shadow slid 0.7 m: {} of {} judged pixels disagree", control.bad.len(), control.shadow_checked + control.lit_checked);
    assert!(control.bad.len() > 100, "the check must be able to tell a wrong shadow from a right one");

    // The shadow falls on the side away from the light: the mean ground position of the shadow-coloured pixels,
    // measured from the car, points along -L (horizontal part), and no shadow reaches the light's side beyond the car.
    let away = norm([-l[0], 0.0, -l[2]]);
    let (mut sum, mut n) = ([0.0f64; 2], 0);
    let pos = rc.position();
    for y in 0..h {
        for x in 0..w {
            let ray = rc.ray(x, y);
            if ray[1] >= -1e-9 || !near(rgb(fb.get(x as i32, y as i32).unwrap()), dark, 0) {
                continue;
            }
            let t = -pos[1] / ray[1];
            let (gx, gz) = (pos[0] + ray[0] * t - CAR_POS[0] as f64, pos[2] + ray[2] * t - CAR_POS[2] as f64);
            sum[0] += gx;
            sum[1] += gz;
            n += 1;
        }
    }
    let mean = [sum[0] / n as f64, sum[1] / n as f64];
    let along = mean[0] * away[0] + mean[1] * away[2];
    println!("car shadow: {n} shadow-coloured pixels, mean offset from the car ({:.2}, {:.2}), {:.2} m along the away-from-light direction", mean[0], mean[1], along);
    assert!(n > 400 && along > 0.3, "shadow must be on the far side from the light (mean {along:.2} m along -L)");

    check("car_shadow", &fb);
}

#[test]
fn rendering_is_deterministic_and_independent_of_what_the_renderer_held_before() {
    let (w, h) = (96, 72);
    let scene = CarScene::new();
    let objects = scene.objects();
    let (centre, radius) = caster_sphere(&objects).unwrap();
    let shadow = ShadowSettings::new(256, centre, radius);
    let cam = car_camera(w, h);
    let mut r = render::Renderer::new();
    let (mut a, mut b) = (Framebuffer::new(w, h).unwrap(), Framebuffer::new(w, h).unwrap());
    let sc = render::Scene { objects: &objects, light: car_light(), shadow: Some(shadow), background: 0 };
    r.render(&mut a, &cam, &sc).unwrap();
    // The same renderer again, after drawing something else at another size and with other settings.
    let mut other = Framebuffer::new(10, 7).unwrap();
    let small = render::Scene { objects: &objects[..2], light: car_light(), shadow: None, background: 5 };
    r.render(&mut other, &cam, &small).unwrap();
    r.render(&mut b, &cam, &sc).unwrap();
    assert_eq!(a, b);
    let mut c = Framebuffer::new(w, h).unwrap();
    render::Renderer::new().render(&mut c, &cam, &sc).unwrap();
    assert_eq!(a, c);
}

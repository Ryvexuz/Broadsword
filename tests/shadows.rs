//! Shadow-map tests: acne, peter-panning, a light straight down, and the error and hard cases of `render`.

mod common;

use common::*;
use render::{
    car_body_mesh, caster_sphere, ground_plane_mesh, wheel_mesh, Light, Mat4, Mesh, Object, RenderError, Renderer, Scene,
    ShadowSettings, Vec3,
};

const GROUND: u32 = 0x0070_A060;
const CUBE: u32 = 0x00B0_2020;
const BG: u32 = 0x0010_1018;
const AMBIENT: f64 = 0.25;
const DIFFUSE: f64 = 0.75;

fn light(to_light: Vec3) -> Light {
    Light { to_light, ambient: AMBIENT as f32, diffuse: DIFFUSE as f32 }
}

fn colours(l: Vec3) -> ([i32; 3], [i32; 3]) {
    let n = l.normalized();
    (shade(GROUND, AMBIENT + DIFFUSE * n.y as f64), shade(GROUND, AMBIENT))
}

/// Does the ray o + t d, for 0 < t < t_max, enter the axis-aligned box? (The slab test.)
fn ray_hits_box(o: [f64; 3], d: [f64; 3], lo: [f64; 3], hi: [f64; 3], t_max: f64) -> bool {
    let (mut t0, mut t1) = (0.0f64, t_max);
    for k in 0..3 {
        if d[k].abs() < 1e-12 {
            if o[k] < lo[k] || o[k] > hi[k] {
                return false;
            }
        } else {
            let (a, b) = ((lo[k] - o[k]) / d[k], (hi[k] - o[k]) / d[k]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    t0 < t1
}

fn bias_in_metres(s: &ShadowSettings, to_light: Vec3, normal: Vec3) -> f32 {
    let cos = normal.normalized().dot(to_light.normalized()).abs();
    let slope = ((1.0 - cos * cos).sqrt() / cos).min(s.max_slope);
    2.0 * s.focus_radius / s.map_size as f32 * (s.bias_const_texels + s.bias_slope * slope)
}

#[test]
fn a_lit_ground_plane_does_not_shadow_itself_at_any_sun_height() {
    let (w, h) = (192, 144);
    let ground = ground_plane_mesh(6.0);
    // The plane both casts and receives: the worst case for acne.
    let objects = [Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: true }];
    let (centre, radius) = caster_sphere(&objects).unwrap();
    let cam = look_at(Vec3::new(0.0, 5.0, 6.0), Vec3::new(0.0, 0.0, 0.0), w, h);
    let rc = RefCamera::new(&cam, w, h);
    let mut without_bias = 0;
    for elevation_deg in [75.0f32, 45.0, 20.0, 10.0] {
        let e = elevation_deg.to_radians();
        let to_light = Vec3::new(e.cos() * 0.8, e.sin(), e.cos() * 0.6);
        let (lit, dark) = colours(to_light);
        let judge = |shadow: ShadowSettings| {
            let (fb, _) = render_scene(w, h, &cam, &objects, light(to_light), Some(shadow), BG);
            check_ground(&fb, &rc, 6.0, lit, dark, &|_, _| Some(false), &|_, _, _, _| false)
        };
        let good = judge(ShadowSettings::new(512, centre, radius));
        let bias = bias_in_metres(&ShadowSettings::new(512, centre, radius), to_light, Vec3::new(0.0, 1.0, 0.0));
        println!(
            "acne at {elevation_deg} deg: {} ground pixels checked, {} shadowed wrongly (bias {:.1} cm)",
            good.lit_checked,
            good.bad.len(),
            bias * 100.0
        );
        assert!(good.lit_checked > 10000);
        assert!(good.bad.is_empty(), "acne at {elevation_deg} degrees: {:?}", &good.bad[..good.bad.len().min(3)]);
        // And the check has teeth: with no bias at all the same scene is full of acne.
        let mut none = ShadowSettings::new(512, centre, radius);
        none.bias_const_texels = 0.0;
        none.bias_slope = 0.0;
        without_bias += judge(none).bad.len();
    }
    println!("acne with the bias switched off: {without_bias} wrong pixels over the four sun heights");
    assert!(without_bias > 200, "with bias 0 this test must see acne, or it proves nothing");
}

#[test]
fn a_cube_on_the_ground_shadows_the_ground_right_up_to_its_foot_and_no_further() {
    let (w, h) = (256, 192);
    let cube = Mesh::from_obj(include_str!("fixtures/cube.obj")).unwrap(); // 2 x 2 x 2, centred on the origin
    let ground = ground_plane_mesh(8.0);
    let cube_model = Mat4::translation(Vec3::new(0.0, 1.0, 0.0)); // sits on y = 0, top at y = 2
    let objects = [
        Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: false },
        Object { mesh: &cube, model: cube_model, color: CUBE, casts_shadow: true },
    ];
    let to_light = Vec3::new(0.5, 1.0, 0.0); // from +X, 63 degrees up: the shadow runs 1.0 toward -X
    let (centre, radius) = caster_sphere(&objects).unwrap();
    let shadow = ShadowSettings::new(1024, centre, radius);
    let cam = look_at(Vec3::new(-7.0, 7.0, 5.0), Vec3::new(-1.5, 0.0, 0.0), w, h);
    let (fb, _) = render_scene(w, h, &cam, &objects, light(to_light), Some(shadow), BG);
    let rc = RefCamera::new(&cam, w, h);
    let (lit, dark) = colours(to_light);
    // The shadow is the rectangle x in [-2, 1], z in [-1, 1] (the cube's foot plus a strip of 2 * 0.5 = 1 m).
    let expect = |x: f64, z: f64| {
        if (-1.95..0.95).contains(&x) && z.abs() < 0.95 {
            Some(true)
        } else if x < -2.05 || x > 1.05 || z.abs() > 1.05 {
            Some(false)
        } else {
            None
        }
    };
    // Pixels whose ray passes within 5 cm of the cube are not judged: its silhouette is not the point here.
    let skip = |_: usize, _: usize, ray: [f64; 3], t: f64| ray_hits_box(rc.position(), ray, [-1.05, 0.0, -1.05], [1.05, 2.05, 1.05], t - 1e-6);
    let r = check_ground(&fb, &rc, 8.0, lit, dark, &expect, &skip);
    let bias = bias_in_metres(&shadow, to_light, Vec3::new(0.0, 1.0, 0.0));
    println!(
        "cube shadow: {} shadow + {} lit ground pixels judged, {} wrong; floor bias {:.2} cm, texel {:.2} cm",
        r.shadow_checked,
        r.lit_checked,
        r.bad.len(),
        bias * 100.0,
        2.0 * shadow.focus_radius / 1024.0 * 100.0
    );
    assert!(r.bad.is_empty(), "{:?}", &r.bad[..r.bad.len().min(5)]);
    assert!(r.shadow_checked > 250 && r.lit_checked > 15000);
    // Peter-panning is a gap between the object and its shadow, as large as the bias. Here the bias is far below one
    // pixel of this picture, so a wrong gap could not hide: it would also need to be a mistake of the position.
    assert!(bias < 0.02, "the bias on the floor should be about a centimetre, not {bias}");
}

#[test]
fn a_light_pointing_straight_down_still_makes_a_shadow_under_the_object() {
    let (w, h) = (256, 192);
    let cube = Mesh::from_obj(include_str!("fixtures/cube.obj")).unwrap();
    let ground = ground_plane_mesh(8.0);
    // A cube floating with its centre 3.5 up: y from 2.5 to 4.5. The sun is exactly overhead.
    let cube_model = Mat4::translation(Vec3::new(0.0, 3.5, 0.0));
    let objects = [
        Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: false },
        Object { mesh: &cube, model: cube_model, color: CUBE, casts_shadow: true },
    ];
    let to_light = Vec3::new(0.0, 1.0, 0.0);
    let (centre, radius) = caster_sphere(&objects).unwrap();
    let shadow = ShadowSettings::new(1024, centre, radius);
    let cam = look_at(Vec3::new(-6.0, 5.0, 7.0), Vec3::new(0.0, 0.5, 0.0), w, h);
    let (fb, _) = render_scene(w, h, &cam, &objects, light(to_light), Some(shadow), BG);
    let rc = RefCamera::new(&cam, w, h);
    let (lit, dark) = colours(to_light);
    let expect = |x: f64, z: f64| {
        if x.abs() < 0.93 && z.abs() < 0.93 {
            Some(true)
        } else if x.abs() > 1.07 || z.abs() > 1.07 {
            Some(false)
        } else {
            None
        }
    };
    // Pixels whose ray passes within 5 cm of the cube are not judged: its silhouette is not the point here.
    let skip = |_: usize, _: usize, ray: [f64; 3], t: f64| ray_hits_box(rc.position(), ray, [-1.05, 2.45, -1.05], [1.05, 4.55, 1.05], t - 1e-6);
    let r = check_ground(&fb, &rc, 8.0, lit, dark, &expect, &skip);
    println!("overhead sun: {} shadow + {} lit ground pixels judged, {} wrong", r.shadow_checked, r.lit_checked, r.bad.len());
    assert!(r.bad.is_empty(), "{:?}", &r.bad[..r.bad.len().min(5)]);
    assert!(r.shadow_checked > 300 && r.lit_checked > 15000);
    // The cube's top face looks straight at the light: N.L = 1, so ambient + diffuse.
    let (sx, sy) = rc.project([0.0, 4.5, 0.0]);
    let top = rgb(fb.get(sx.round() as i32, sy.round() as i32).unwrap());
    assert!(near(top, shade(CUBE, AMBIENT + DIFFUSE), 1), "top face {top:?}");
}

#[test]
fn the_car_scene_has_the_expected_triangle_counts_and_turning_shadows_off_removes_them() {
    let (w, h) = (96, 72);
    let (body, wheel, ground) = (car_body_mesh(), wheel_mesh(24).unwrap(), ground_plane_mesh(12.0));
    let mut objects = vec![Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: false }];
    objects.push(Object { mesh: &body, model: Mat4::IDENTITY, color: CUBE, casts_shadow: true });
    for c in render::car_wheel_centres() {
        objects.push(Object { mesh: &wheel, model: Mat4::translation(c), color: 0x0020_2020, casts_shadow: true });
    }
    let cam = look_at(Vec3::new(-4.0, 3.0, 6.0), Vec3::new(0.0, 0.4, 0.0), w, h);
    let to_light = Vec3::new(0.5, 1.0, -0.6);
    let (centre, radius) = caster_sphere(&objects).unwrap();
    let (with, stats) = render_scene(w, h, &cam, &objects, light(to_light), Some(ShadowSettings::new(512, centre, radius)), BG);
    // 2 ground + 24 body and cabin + 4 wheels of (24 sides * 2 + 24 * 2 cap) triangles.
    assert_eq!(stats.triangles_submitted, 2 + 24 + 4 * 96);
    assert_eq!(stats.triangles_shadow, 24 + 4 * 96, "every caster triangle is rasterised into the map");
    assert!(stats.triangles_main > 0 && stats.triangles_main < stats.triangles_submitted, "back faces are culled");
    let (without, stats2) = render_scene(w, h, &cam, &objects, light(to_light), None, BG);
    assert_eq!(stats2.triangles_shadow, 0);
    assert_ne!(with, without, "shadows change the picture");
    // With shadows off the ground is a single colour wherever the car does not cover it.
    let (lit, _) = colours(to_light);
    let rc = RefCamera::new(&cam, w, h);
    let skip = |_: usize, _: usize, ray: [f64; 3], t: f64| {
        ray_hits_box(rc.position(), ray, [-2.0, 0.3, -0.9], [2.0, 1.35, 0.9], t - 1e-6)
            || ray_hits_box(rc.position(), ray, [-1.6, 0.0, -1.0], [1.6, 0.7, 1.0], t - 1e-6)
    };
    let r = check_ground(&without, &rc, 12.0, lit, lit, &|x, z| if x.abs() > 4.0 || z.abs() > 4.0 { Some(false) } else { None }, &skip);
    assert!(r.bad.is_empty() && r.lit_checked > 1000, "{:?}", r.bad.first());
}

#[test]
fn a_one_by_one_framebuffer_renders_without_panicking() {
    let (body, wheel, ground) = (car_body_mesh(), wheel_mesh(8).unwrap(), ground_plane_mesh(12.0));
    let objects = [
        Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: false },
        Object { mesh: &body, model: Mat4::IDENTITY, color: CUBE, casts_shadow: true },
        Object { mesh: &wheel, model: Mat4::translation(Vec3::new(1.0, 0.35, 0.8)), color: 0x0020_2020, casts_shadow: true },
    ];
    let (centre, radius) = caster_sphere(&objects).unwrap();
    // Looking straight at the middle of the car's side, so the single pixel is on the car.
    let cam = look_at(Vec3::new(0.0, 0.6, 6.0), Vec3::new(0.0, 0.6, 0.0), 1, 1);
    let (fb, stats) = render_scene(1, 1, &cam, &objects, light(Vec3::new(0.5, 1.0, 0.6)), Some(ShadowSettings::new(64, centre, radius)), BG);
    assert_eq!((fb.width(), fb.height()), (1, 1));
    assert!(stats.triangles_main > 0);
    // (The single pixel is sampled at the view's top-left corner, so what it shows is not asserted: the point is
    // that nothing panics and the triangles are processed.)
    // And one with a 1x1 shadow map, and no shadows at all.
    render_scene(1, 1, &cam, &objects, light(Vec3::new(0.0, 1.0, 0.0)), Some(ShadowSettings::new(1, centre, radius)), BG);
    render_scene(1, 1, &cam, &objects, light(Vec3::new(0.0, 1.0, 0.0)), None, BG);
}

#[test]
fn bad_input_is_refused_with_a_clear_error_not_a_panic() {
    let ground = ground_plane_mesh(1.0);
    let objects = [Object { mesh: &ground, model: Mat4::IDENTITY, color: GROUND, casts_shadow: true }];
    let cam = look_at(Vec3::new(0.0, 2.0, 2.0), Vec3::new(0.0, 0.0, 0.0), 8, 8);
    let try_render = |objects: &[Object], l: Light, shadow: Option<ShadowSettings>| {
        let mut fb = render::Framebuffer::new(8, 8).unwrap();
        Renderer::new().render(&mut fb, &cam, &Scene { objects, light: l, shadow, background: 0 })
    };
    let ok = light(Vec3::new(0.0, 1.0, 0.0));
    let s = ShadowSettings::new(16, Vec3::new(0.0, 0.0, 0.0), 2.0);
    assert!(try_render(&objects, ok, Some(s)).is_ok());
    assert!(matches!(try_render(&objects, light(Vec3::new(0.0, 0.0, 0.0)), None), Err(RenderError::BadLight(_))));
    assert!(matches!(try_render(&objects, light(Vec3::new(f32::NAN, 1.0, 0.0)), None), Err(RenderError::BadLight(_))));
    assert!(matches!(try_render(&objects, ok, Some(ShadowSettings { map_size: 0, ..s })), Err(RenderError::BadShadow(_))));
    assert!(matches!(try_render(&objects, ok, Some(ShadowSettings { focus_radius: 0.0, ..s })), Err(RenderError::BadShadow(_))));
    assert!(matches!(try_render(&objects, ok, Some(ShadowSettings { map_size: usize::MAX, ..s })), Err(RenderError::BadShadow(_))));
    let mut bad = ground.clone();
    bad.triangles.push([0, 1, 99]);
    let bad_objects = [Object { mesh: &bad, ..objects[0] }];
    match try_render(&bad_objects, ok, None) {
        Err(RenderError::BadMesh { object: 0, reason }) => assert!(reason.contains("99"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let mut bad = ground.clone();
    bad.normals.pop();
    let bad_objects = [Object { mesh: &bad, ..objects[0] }];
    assert!(matches!(try_render(&bad_objects, ok, None), Err(RenderError::BadMesh { .. })));
    // No caster: no sphere.
    let no_caster = [Object { casts_shadow: false, ..objects[0] }];
    assert!(caster_sphere(&no_caster).is_none());
}

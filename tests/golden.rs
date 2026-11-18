//! Golden-image tests: each scene is rendered and compared byte for byte with `tests/golden/<name>.ppm`.
//!
//! - Mismatch: the actual image is written to `tests/golden/<name>.actual.ppm` (git-ignored) and the test
//!   fails, saying how many pixels differ and their bounding box.
//! - Missing golden file: the test fails, writes the `.actual.ppm`, and says how to accept it. Accepting is
//!   always done by hand (copy the actual file over the golden one), never automatically.
//! - A passing test removes any stale `.actual.ppm` it finds.

use render::{draw_line_3d, draw_point_3d, wire_cube, Camera, Framebuffer, Vec3};
use std::fs;
use std::path::PathBuf;

const BG: u32 = 0x0010_1018;
const WHITE: u32 = 0x00FF_FFFF;
const AMBER: u32 = 0x00FF_C040;
const RED: u32 = 0x00FF_3030;
const GREEN: u32 = 0x0040_E060;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

/// Describes how two same-sized framebuffers differ: count and bounding box (x0, y0, x1, y1).
fn diff(expected: &Framebuffer, actual: &Framebuffer) -> Option<(usize, (usize, usize, usize, usize))> {
    let w = expected.width();
    let mut count = 0;
    let mut bb = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, (e, a)) in expected.pixels().iter().zip(actual.pixels()).enumerate() {
        if e != a {
            let (x, y) = (i % w, i / w);
            count += 1;
            bb = (bb.0.min(x), bb.1.min(y), bb.2.max(x), bb.3.max(y));
        }
    }
    (count > 0).then_some((count, bb))
}

fn check(name: &str, fb: &Framebuffer) {
    let golden = golden_dir().join(format!("{name}.ppm"));
    let actual_path = golden_dir().join(format!("{name}.actual.ppm"));
    let actual = fb.to_ppm();
    let expected_bytes = match fs::read(&golden) {
        Ok(b) => b,
        Err(_) => {
            fs::create_dir_all(golden_dir()).unwrap();
            fs::write(&actual_path, &actual).unwrap();
            panic!(
                "golden image {name}.ppm is missing. The rendered image was written to {}. After checking it, \
                 accept it by hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
                actual_path.display()
            );
        }
    };
    if expected_bytes == actual {
        let _ = fs::remove_file(&actual_path);
        return;
    }
    fs::write(&actual_path, &actual).unwrap();
    let detail = match Framebuffer::from_ppm(&expected_bytes) {
        Ok(expected) if expected.width() == fb.width() && expected.height() == fb.height() => {
            match diff(&expected, fb) {
                Some((n, (x0, y0, x1, y1))) => {
                    format!("{n} pixels differ, bounding box x {x0}..={x1}, y {y0}..={y1}")
                }
                None => "pixels are identical but the file bytes differ (header or length)".to_string(),
            }
        }
        Ok(expected) => format!(
            "size differs: golden is {}x{}, actual is {}x{}",
            expected.width(),
            expected.height(),
            fb.width(),
            fb.height()
        ),
        Err(e) => format!("the golden file cannot be read: {e}"),
    };
    panic!(
        "golden mismatch for {name}: {detail}. Actual image written to {}. If the change is intended, accept \
         it by hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
        actual_path.display()
    );
}

fn camera(position: Vec3, yaw: f32, pitch: f32, aspect: f32) -> Camera {
    Camera {
        position,
        yaw,
        pitch,
        fov_y: 60f32.to_radians(),
        aspect,
        near: 0.1,
        far: 100.0,
        move_speed: 1.0,
        turn_speed: 1.0,
    }
}

/// A camera at `position` looking at `target`.
fn look_toward(position: Vec3, target: Vec3, aspect: f32) -> Camera {
    let f = (target - position).normalized();
    camera(position, f.x.atan2(-f.z), f.y.asin(), aspect)
}

fn render_cube(fb: &mut Framebuffer, cam: &Camera, rotation_y_deg: f32) {
    for (a, b) in wire_cube(Vec3::new(0.0, 0.0, 0.0), 2.0, rotation_y_deg.to_radians()) {
        draw_line_3d(fb, cam, a, b, AMBER);
    }
}

#[test]
fn golden_cube_front() {
    let mut fb = Framebuffer::new(160, 120).unwrap();
    fb.clear(BG);
    let cam = camera(Vec3::new(0.0, 0.0, 5.0), 0.0, 0.0, 160.0 / 120.0);
    render_cube(&mut fb, &cam, 0.0);
    check("cube_front", &fb);
}

#[test]
fn golden_cube_rotated_from_above() {
    let mut fb = Framebuffer::new(160, 120).unwrap();
    fb.clear(BG);
    let cam = look_toward(Vec3::new(3.0, 4.0, 6.0), Vec3::new(0.0, 0.0, 0.0), 160.0 / 120.0);
    render_cube(&mut fb, &cam, 30.0);
    check("cube_rotated_from_above", &fb);
}

#[test]
fn golden_line_crossing_near_plane() {
    let mut fb = Framebuffer::new(160, 120).unwrap();
    fb.clear(BG);
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, 160.0 / 120.0);
    // Horizontal-looking line that starts in front of the camera and ends behind it.
    draw_line_3d(&mut fb, &cam, Vec3::new(-1.0, 0.0, -2.0), Vec3::new(-1.0, 0.0, 2.0), RED);
    // A diagonal one, crossing the near plane to the right and above.
    draw_line_3d(&mut fb, &cam, Vec3::new(0.5, 0.2, -1.5), Vec3::new(1.5, 0.9, 1.0), GREEN);
    // A control line wholly in front.
    draw_line_3d(&mut fb, &cam, Vec3::new(-0.5, -0.5, -3.0), Vec3::new(0.5, -0.5, -3.0), WHITE);
    check("line_near_plane", &fb);
}

#[test]
fn golden_point_grid() {
    let mut fb = Framebuffer::new(160, 120).unwrap();
    fb.clear(BG);
    let cam = look_toward(Vec3::new(0.0, 3.0, 6.0), Vec3::new(0.0, 0.0, 0.0), 160.0 / 120.0);
    for ix in -3i32..=3 {
        for iz in -3i32..=3 {
            // Sizes 1, 2 and 3 in a repeating pattern so all three are covered.
            let size = ((ix + iz).rem_euclid(3) + 1) as u32;
            draw_point_3d(&mut fb, &cam, Vec3::new(ix as f32, 0.0, iz as f32), size, WHITE);
        }
    }
    check("point_grid", &fb);
}

#[test]
fn golden_framebuffer_1x1() {
    let mut fb = Framebuffer::new(1, 1).unwrap();
    fb.clear(BG);
    fb.line(-5, -5, 5, 5, RED);
    fb.point(0, 0, 3, GREEN);
    fb.point(4, 4, 3, WHITE); // off the buffer: ignored
    check("fb_1x1", &fb);
}

#[test]
fn golden_framebuffer_tiny() {
    let mut fb = Framebuffer::new(9, 6).unwrap();
    fb.clear(BG);
    fb.line(0, 0, 8, 5, AMBER); // shallow diagonal, corner to corner
    fb.line(8, 0, 0, 0, RED); // horizontal, drawn right to left
    fb.line(2, 0, 2, 5, GREEN); // vertical
    fb.line(-10, 3, 20, 1, WHITE); // runs off both sides
    fb.line(4, 4, 4, 4, WHITE); // zero length: one pixel
    fb.line(-3, -3, -1, 9, RED); // wholly outside: nothing
    fb.point(7, 4, 2, GREEN);
    check("fb_tiny", &fb);
}

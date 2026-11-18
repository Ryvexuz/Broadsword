//! Behaviour tests that do not use golden files: the hard cases, and hand-derived expectations that give
//! the golden images an independent check.

use render::{draw_line_3d, draw_point_3d, wire_cube, Camera, FlyInput, Framebuffer, FramebufferError, Mat4, Vec3};

const BG: u32 = 0x0010_1018;
const FG: u32 = 0x00FF_FFFF;

fn cam(position: Vec3, yaw: f32, pitch: f32) -> Camera {
    Camera {
        position,
        yaw,
        pitch,
        fov_y: 60f32.to_radians(),
        aspect: 160.0 / 120.0,
        near: 0.1,
        far: 100.0,
        move_speed: 2.0,
        turn_speed: 1.0,
    }
}

fn lit(fb: &Framebuffer) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in 0..fb.height() as i32 {
        for x in 0..fb.width() as i32 {
            if fb.get(x, y) != Some(BG) {
                v.push((x, y));
            }
        }
    }
    v
}

fn fresh(w: usize, h: usize) -> Framebuffer {
    let mut fb = Framebuffer::new(w, h).unwrap();
    fb.clear(BG);
    fb
}

#[test]
fn zero_size_is_an_error_not_a_panic() {
    assert_eq!(Framebuffer::new(0, 5), Err(FramebufferError::ZeroSize { width: 0, height: 5 }));
    assert_eq!(Framebuffer::new(5, 0), Err(FramebufferError::ZeroSize { width: 5, height: 0 }));
    assert!(Framebuffer::new(1, 1).is_ok());
}

#[test]
fn set_out_of_range_is_ignored() {
    let mut fb = fresh(4, 3);
    for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 3), (i32::MIN, i32::MAX), (i32::MAX, i32::MIN)] {
        fb.set(x, y, FG);
    }
    assert!(lit(&fb).is_empty());
    fb.set(3, 2, FG);
    assert_eq!(lit(&fb), vec![(3, 2)]);
}

#[test]
fn ppm_format_and_round_trip() {
    let mut fb = fresh(3, 2);
    fb.set(1, 0, 0x0012_3456);
    let ppm = fb.to_ppm();
    assert!(ppm.starts_with(b"P6\n3 2\n255\n"));
    assert_eq!(ppm.len(), 11 + 3 * 2 * 3);
    // Pixel (1, 0) is bytes 3..6 of the data: R, G, B.
    assert_eq!(&ppm[11 + 3..11 + 6], &[0x12, 0x34, 0x56]);
    assert_eq!(Framebuffer::from_ppm(&ppm).unwrap(), fb);
}

#[test]
fn line_includes_both_endpoints_in_every_direction() {
    for (x0, y0, x1, y1) in [(1, 1, 8, 4), (8, 4, 1, 1), (1, 7, 8, 2), (3, 0, 3, 9), (0, 5, 9, 5), (2, 2, 2, 2)] {
        let mut fb = fresh(10, 10);
        fb.line(x0, y0, x1, y1, FG);
        assert_eq!(fb.get(x0, y0), Some(FG), "start of {x0},{y0} to {x1},{y1}");
        assert_eq!(fb.get(x1, y1), Some(FG), "end of {x0},{y0} to {x1},{y1}");
        // A Bresenham line has exactly max(dx, dy) + 1 pixels.
        let n = ((x1 - x0).abs().max((y1 - y0).abs()) + 1) as usize;
        assert_eq!(lit(&fb).len(), n);
    }
}

#[test]
fn line_is_the_same_drawn_either_way_round_for_axis_lines() {
    let mut a = fresh(10, 10);
    let mut b = fresh(10, 10);
    a.line(1, 3, 8, 3, FG);
    b.line(8, 3, 1, 3, FG);
    assert_eq!(a, b);
}

#[test]
fn clipped_line_keeps_the_pixels_of_the_unclipped_line() {
    // The same line drawn into a small buffer must equal the matching window of a big buffer.
    let mut big = fresh(40, 40);
    big.line(-10, 5, 30, 17, FG);
    let mut small = fresh(20, 20);
    small.line(-10, 5, 30, 17, FG);
    for y in 0..20 {
        for x in 0..20 {
            assert_eq!(small.get(x, y), big.get(x, y), "pixel {x},{y}");
        }
    }
}

#[test]
fn line_fully_outside_draws_nothing_and_partly_outside_draws_the_inside() {
    let mut fb = fresh(8, 8);
    fb.line(-20, -20, -1, 30, FG);
    fb.line(9, 0, 40, 7, FG);
    fb.line(-5, 20, 30, 20, FG);
    assert!(lit(&fb).is_empty());
    fb.line(-4, 3, 12, 3, FG); // crosses the whole buffer
    assert_eq!(lit(&fb).len(), 8);
}

#[test]
fn zero_length_line_is_one_pixel() {
    let mut fb = fresh(5, 5);
    fb.line(2, 3, 2, 3, FG);
    assert_eq!(lit(&fb), vec![(2, 3)]);
}

#[test]
fn point_is_a_filled_square_clipped_at_the_edges() {
    let mut fb = fresh(10, 10);
    fb.point(5, 5, 3, FG);
    assert_eq!(lit(&fb).len(), 9);
    assert!(fb.get(4, 4) == Some(FG) && fb.get(6, 6) == Some(FG) && fb.get(3, 5) == Some(BG));
    let mut fb = fresh(10, 10);
    fb.point(0, 0, 3, FG);
    assert_eq!(lit(&fb).len(), 4);
    let mut fb = fresh(10, 10);
    fb.point(5, 5, 0, FG);
    fb.point(-50, 5, 3, FG);
    assert!(lit(&fb).is_empty());
    fb.point(5, 5, u32::MAX, FG); // covers everything, without walking billions of pixels
    assert_eq!(lit(&fb).len(), 100);
}

#[test]
fn wire_cube_has_12_distinct_edges_of_the_right_length() {
    for rot in [0.0f32, 0.5236, 1.0] {
        let edges = wire_cube(Vec3::new(1.0, 2.0, 3.0), 2.0, rot);
        assert_eq!(edges.len(), 12);
        for (a, b) in &edges {
            assert!(((*b - *a).length() - 2.0).abs() < 1e-5);
        }
        for i in 0..edges.len() {
            for j in i + 1..edges.len() {
                let same = |p: Vec3, q: Vec3| (p - q).length() < 1e-5;
                let dup = (same(edges[i].0, edges[j].0) && same(edges[i].1, edges[j].1))
                    || (same(edges[i].0, edges[j].1) && same(edges[i].1, edges[j].0));
                assert!(!dup, "edges {i} and {j} are the same");
            }
        }
    }
}

#[test]
fn projection_matches_hand_maths() {
    // Camera at z=5 looking down -Z, 60 degree vertical fov, aspect 4:3, image 160x120.
    // Front-face corner (1, 1, 1): depth 4, ndc x = (1/tan30)/(4/3)/4 = 0.32476, ndc y = 0.43301.
    // Screen x = (0.32476/2 + 0.5) * 160 = 105.98, y = (0.5 - 0.43301/2) * 120 = 34.02.
    let c = cam(Vec3::new(0.0, 0.0, 5.0), 0.0, 0.0);
    let (x, y) = c.project(Vec3::new(1.0, 1.0, 1.0), 160, 120).unwrap();
    assert!((x - 105.98).abs() < 0.05 && (y - 34.02).abs() < 0.05, "got {x},{y}");
    // Back-face corner (1, 1, -1): depth 6, screen (97.3, 42.7).
    let (x, y) = c.project(Vec3::new(1.0, 1.0, -1.0), 160, 120).unwrap();
    assert!((x - 97.30).abs() < 0.05 && (y - 42.70).abs() < 0.05, "got {x},{y}");
    // The point straight ahead lands in the middle.
    let (x, y) = c.project(Vec3::new(0.0, 0.0, 0.0), 160, 120).unwrap();
    assert!((x - 80.0).abs() < 1e-3 && (y - 60.0).abs() < 1e-3);
}

#[test]
fn project_refuses_points_behind_the_camera_or_past_far() {
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    assert!(c.project(Vec3::new(0.0, 0.0, 1.0), 160, 120).is_none());
    assert!(c.project(Vec3::new(0.0, 0.0, 0.0), 160, 120).is_none());
    assert!(c.project(Vec3::new(0.0, 0.0, -0.05), 160, 120).is_none());
    assert!(c.project(Vec3::new(0.0, 0.0, -101.0), 160, 120).is_none());
    assert!(c.project(Vec3::new(0.0, 0.0, -5.0), 160, 120).is_some());
}

#[test]
fn front_cube_corners_are_lit_where_the_maths_says() {
    let mut fb = fresh(160, 120);
    let c = cam(Vec3::new(0.0, 0.0, 5.0), 0.0, 0.0);
    for (a, b) in wire_cube(Vec3::new(0.0, 0.0, 0.0), 2.0, 0.0) {
        draw_line_3d(&mut fb, &c, a, b, FG);
    }
    // Hand-derived: front corners (+-105.98 mirrored -> 54.02 and 105.98, 34.02 and 85.98), back corners
    // (62.7 and 97.3, 42.7 and 77.3). Rounded to pixels:
    for (x, y) in [(54, 34), (106, 34), (54, 86), (106, 86), (63, 43), (97, 43), (63, 77), (97, 77)] {
        assert_eq!(fb.get(x, y), Some(FG), "corner {x},{y}");
    }
    // Front face edges are straight, so the top edge row 34 is lit from x=54 to 106: 53 pixels.
    for x in 54..=106 {
        assert_eq!(fb.get(x, 34), Some(FG), "top edge at {x}");
        assert_eq!(fb.get(x, 86), Some(FG), "bottom edge at {x}");
    }
    // The picture is mirror-symmetric left/right. Screen coordinate s lights pixel round(s), so the image
    // centre is x = 80.0 and pixel x mirrors to 160 - x (column 0 has no partner).
    for y in 0..120 {
        for x in 1..=80 {
            assert_eq!(fb.get(x, y), fb.get(160 - x, y), "mirror at {x},{y}");
        }
    }
    // Looking straight on, the front and back faces are nested squares joined by 4 diagonals, so the whole
    // image is 2 squares (53 per side -> 4*52 = 208 pixels) + (35 -> 4*34 = 136) + 4 connectors of 8 new
    // pixels each (the 4 corners of each square are already counted): 208 + 136 + 32 = 376.
    assert_eq!(lit(&fb).len(), 208 + 136 + 32);
}

#[test]
fn line_crossing_the_near_plane_ends_at_the_crossing_not_wrapping_round() {
    let mut fb = fresh(160, 120);
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    // x = -1 for all of it; z from -2 (in front) to +2 (behind). In front, screen x = 28.0 at y = 60 and the
    // line runs left to the screen edge as it nears the camera. A wrap-around bug would draw to the right.
    draw_line_3d(&mut fb, &c, Vec3::new(-1.0, 0.0, -2.0), Vec3::new(-1.0, 0.0, 2.0), FG);
    let pixels = lit(&fb);
    assert!(pixels.iter().all(|&(x, y)| y == 60 && x <= 28), "unexpected pixels: {pixels:?}");
    assert_eq!(pixels.len(), 29); // x = 0..=28
}

#[test]
fn lines_wholly_behind_or_through_the_camera_are_handled() {
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, Vec3::new(0.0, 0.0, 1.0), Vec3::new(2.0, 1.0, 5.0), FG); // behind
    assert!(lit(&fb).is_empty());
    draw_line_3d(&mut fb, &c, Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0), FG); // zero length, behind
    draw_line_3d(&mut fb, &c, Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 0.0, -1.0), FG); // zero length, ahead
    assert_eq!(lit(&fb), vec![(80, 60)]);
    // Straight through the camera: only the part in front is drawn (a single centre pixel on the axis).
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, Vec3::new(0.0, 0.0, -3.0), Vec3::new(0.0, 0.0, 3.0), FG);
    assert_eq!(lit(&fb), vec![(80, 60)]);
    // Beyond the far plane.
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, Vec3::new(0.0, 0.0, -150.0), Vec3::new(5.0, 0.0, -200.0), FG);
    assert!(lit(&fb).is_empty());
    // Far-plane clipping of a line that starts in range.
    draw_line_3d(&mut fb, &c, Vec3::new(1.0, 0.0, -50.0), Vec3::new(1.0, 0.0, -500.0), FG);
    assert!(!lit(&fb).is_empty());
}

#[test]
fn points_behind_the_camera_are_skipped() {
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let mut fb = fresh(160, 120);
    draw_point_3d(&mut fb, &c, Vec3::new(0.0, 0.0, 3.0), 3, FG);
    assert!(lit(&fb).is_empty());
    draw_point_3d(&mut fb, &c, Vec3::new(0.0, 0.0, -3.0), 3, FG);
    assert_eq!(lit(&fb).len(), 9);
}

#[test]
fn camera_inside_the_cube_draws_without_panic_and_stays_in_the_buffer() {
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.3, -0.2);
    let mut fb = fresh(160, 120);
    for (a, b) in wire_cube(Vec3::new(0.0, 0.0, 0.0), 2.0, 0.4) {
        draw_line_3d(&mut fb, &c, a, b, FG);
    }
    // Inside the cube every wall edge is around or behind us; the ones in front must show.
    assert!(!lit(&fb).is_empty());
}

#[test]
fn rendering_twice_gives_identical_bytes() {
    let render = || {
        let mut fb = fresh(160, 120);
        let c = cam(Vec3::new(3.0, 4.0, 6.0), -0.4, -0.5);
        for (a, b) in wire_cube(Vec3::new(0.0, 0.0, 0.0), 2.0, 0.5236) {
            draw_line_3d(&mut fb, &c, a, b, FG);
        }
        fb.to_ppm()
    };
    assert_eq!(render(), render());
}

#[test]
fn huge_coordinates_do_not_hang_or_panic() {
    let mut fb = fresh(16, 16);
    fb.line(-1_000_000, 8, 1_000_000, 8, FG);
    assert_eq!(lit(&fb).len(), 16);
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    draw_line_3d(&mut fb, &c, Vec3::new(1e30, 0.0, -1.0), Vec3::new(-1e30, 0.0, -1.0), FG);
    draw_line_3d(&mut fb, &c, Vec3::new(f32::NAN, 0.0, -1.0), Vec3::new(0.0, 0.0, -2.0), FG);
}

#[test]
fn fly_moves_and_turns_and_is_pure() {
    let mut c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    // No input, no change.
    let before = c;
    c.fly(&FlyInput::default(), 1.0);
    assert_eq!(c, before);
    // Forward for 0.5 s at speed 2: 1 unit along -Z.
    c.fly(&FlyInput { forward: true, ..Default::default() }, 0.5);
    assert!((c.position - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5, "{:?}", c.position);
    // Right moves +X; left cancels; opposite keys cancel.
    c.fly(&FlyInput { right: true, ..Default::default() }, 0.5);
    assert!((c.position.x - 1.0).abs() < 1e-5);
    c.fly(&FlyInput { forward: true, back: true, left: true, right: true, ..Default::default() }, 1.0);
    assert!((c.position - Vec3::new(1.0, 0.0, -1.0)).length() < 1e-5);
    // Up moves +Y.
    c.fly(&FlyInput { up: true, ..Default::default() }, 1.0);
    assert!((c.position.y - 2.0).abs() < 1e-5);
    // Turning right by 1 rad/s for pi/2 s makes the camera look down +X.
    c.fly(&FlyInput { turn_right: true, ..Default::default() }, std::f32::consts::FRAC_PI_2);
    assert!((c.forward() - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    // Pitch is clamped short of vertical.
    c.fly(&FlyInput { look_up: true, ..Default::default() }, 100.0);
    assert!(c.pitch < std::f32::consts::FRAC_PI_2 && c.pitch > 1.5);
    // Same input, same state, same result.
    let (mut a, mut b) = (before, before);
    let input = FlyInput { forward: true, turn_left: true, ..Default::default() };
    a.fly(&input, 0.25);
    b.fly(&input, 0.25);
    assert_eq!(a, b);
}

#[test]
fn view_proj_maps_near_and_far_to_clip_extremes() {
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let n = c.view_proj().transform_point(Vec3::new(0.0, 0.0, -0.1));
    let f = c.view_proj().transform_point(Vec3::new(0.0, 0.0, -100.0));
    assert!((n[2] / n[3] + 1.0).abs() < 1e-4, "near ndc z {}", n[2] / n[3]);
    assert!((f[2] / f[3] - 1.0).abs() < 1e-3, "far ndc z {}", f[2] / f[3]);
    assert_eq!(Mat4::IDENTITY.mul(&Mat4::IDENTITY), Mat4::IDENTITY);
}

// ---- review minors from part A: clipping of very long lines

#[test]
fn line_with_extreme_coordinates_finishes_at_once_and_draws_the_full_row() {
    let mut fb = fresh(16, 16);
    let start = std::time::Instant::now();
    fb.line(i32::MIN, 5, i32::MAX, 5, FG);
    fb.line(7, i32::MIN, 7, i32::MAX, FG);
    fb.line(i32::MIN, i32::MIN, i32::MAX, i32::MAX, FG);
    fb.line(i32::MAX, i32::MIN, i32::MIN, i32::MAX, FG);
    assert!(start.elapsed().as_secs_f64() < 1.0, "took {:?}", start.elapsed());
    let l = lit(&fb);
    for x in 0..16 {
        assert_eq!(fb.get(x, 5), Some(FG), "row 5 at x={x}");
    }
    for y in 0..16 {
        assert_eq!(fb.get(7, y), Some(FG), "column 7 at y={y}");
    }
    // The diagonal from (MIN, MIN) to (MAX, MAX) is exactly y = x, so it lights (0,0) to (15,15).
    for i in 0..16 {
        assert_eq!(fb.get(i, i), Some(FG), "diagonal at {i}");
    }
    assert!(!l.is_empty());
    // One-sided extremes and a line that misses the buffer entirely.
    let mut fb = fresh(16, 16);
    fb.line(3, 3, i32::MAX, 3, FG);
    assert_eq!(lit(&fb).len(), 13); // x = 3..=15
    let mut fb = fresh(16, 16);
    fb.line(i32::MIN, 100, i32::MAX, 100, FG);
    fb.line(-5, i32::MIN, -5, i32::MAX, FG);
    assert!(lit(&fb).is_empty());
}

/// The textbook walk from the first endpoint to the last, with no clipping: the reference for `line`.
fn reference_line(fb: &mut Framebuffer, x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
    let (mut x, mut y) = (x0, y0);
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        fb.set(x, y, c);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[test]
fn clipped_line_covers_exactly_the_pixels_of_the_unclipped_walk() {
    // Every pair of endpoints on a grid reaching well outside a 9x6 buffer, in every direction, including
    // the ties of 45 degree and half-pixel slopes: the fast clipped `line` must equal the plain walk.
    let (mut checked, mut lit_total) = (0u32, 0usize);
    for x0 in -6..=14 {
        for y0 in -5..=10 {
            for x1 in -6..=14 {
                for y1 in -5..=10 {
                    let (mut a, mut b) = (fresh(9, 6), fresh(9, 6));
                    a.line(x0, y0, x1, y1, FG);
                    reference_line(&mut b, x0, y0, x1, y1, FG);
                    assert_eq!(a, b, "line ({x0},{y0}) -> ({x1},{y1})");
                    checked += 1;
                    lit_total += lit(&a).len();
                }
            }
        }
    }
    assert!(checked > 50_000 && lit_total > 100_000, "{checked} lines, {lit_total} pixels");
    // Far-off starts that stay on the same line as a near one: a shallow line shifted by a whole number of
    // its own periods draws the same pixels in the buffer.
    let (mut a, mut b) = (fresh(40, 20), fresh(40, 20));
    a.line(-3, 2, 60, 17, FG);
    reference_line(&mut b, -3, 2, 60, 17, FG);
    assert_eq!(a, b);
}

/// Draws a..b with `draw_line_3d` and checks, against an independent f64 projection of the endpoints, that
/// the picture is the true straight line: every lit pixel within 1 pixel of it, and every column (or row,
/// for a steep line) that the line crosses inside the image has a lit pixel. Returns the lit pixels.
fn check_long_line(a: Vec3, b: Vec3) -> Vec<(i32, i32)> {
    // Camera at the origin looking down -Z, image 160x120, fov 60 degrees.
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let f = 1.0 / (30f64.to_radians().tan());
    let aspect = 160.0 / 120.0;
    let proj = |p: Vec3| -> (f64, f64) {
        let d = -(p.z as f64);
        ((p.x as f64 * f / (aspect * d) * 0.5 + 0.5) * 160.0, (0.5 - p.y as f64 * f / d * 0.5) * 120.0)
    };
    let (pa, pb) = (proj(a), proj(b));
    assert!(pa.0.abs().max(pa.1.abs()).max(pb.0.abs()).max(pb.1.abs()) > 1.0e6, "an end is far off screen: {pa:?} {pb:?}");
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, a, b, FG);
    let pixels = lit(&fb);
    let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
    let shallow = dx.abs() >= dy.abs();
    // Position of the true line along the minor axis at a given major coordinate.
    let at = |major: f64| if shallow { pa.1 + dy / dx * (major - pa.0) } else { pa.0 + dx / dy * (major - pa.1) };
    for &(x, y) in &pixels {
        let (major, minor) = if shallow { (x, y) } else { (y, x) };
        let want = at(major as f64);
        assert!((minor as f64 - want).abs() <= 1.0, "pixel ({x},{y}) is off the line (should be near {want:.2})");
    }
    let (major_len, minor_len) = if shallow { (160, 120) } else { (120, 160) };
    let mut crossing = 0;
    for major in 0..major_len {
        let want = at(major as f64);
        if want >= 1.0 && want < minor_len as f64 - 1.0 {
            crossing += 1;
            assert!(
                pixels.iter().any(|&(x, y)| {
                    let (ma, mi) = if shallow { (x, y) } else { (y, x) };
                    ma == major && (mi as f64 - want).abs() <= 1.0
                }),
                "no pixel at {major} (minor should be near {want:.2})"
            );
        }
    }
    assert!(crossing >= 50, "the line crosses {crossing} columns of the image; the test needs a real crossing");
    pixels
}

#[test]
fn a_very_long_ground_line_near_the_camera_keeps_its_slope() {
    // Along the ground (y = -1) from x = -6000, z = -0.5 to x = 4e5, z = -400. The near end is at screen
    // x = -1.25e6, past the old +-1e6 clamp: the clamp moved that end and the line drew 3.7 px too low
    // across the image. Now the picture must stay within 1 px of the true line.
    let a = Vec3::new(-6000.0, -1.0, -0.5);
    let b = Vec3::new(4.0e5, -1.0, -400.0);
    let pixels = check_long_line(a, b);
    // The same line drawn from the other end covers the same pixels, give or take rounding at the cut ends.
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let mut rev = fresh(160, 120);
    draw_line_3d(&mut rev, &c, b, a, FG);
    let diff = lit(&rev).iter().filter(|p| !pixels.contains(p)).count();
    assert!(diff <= pixels.len() / 10, "{diff} of {} pixels differ when reversed", pixels.len());
}

#[test]
fn a_steep_very_long_line_keeps_its_slope_too() {
    // Up a wall at z = -0.4 from (0.02, -5000) to (0.2, 2000): the low end is at screen y = 1.3e6, past the old
    // clamp, which made the line cross the middle of the image 2.6 px too far left.
    check_long_line(Vec3::new(0.02, -5000.0, -0.4), Vec3::new(0.2, 2000.0, -0.4));
}

#[test]
fn a_line_millions_of_pixels_long_is_clipped_in_2d_and_still_draws_the_whole_row() {
    // Screen x of 3.5e10 at both ends: beyond what an i32 pixel can hold, so the 2D clip does the work.
    // y = 0 maps to row 60 whatever x is, so the whole row 60 must be lit and nothing else.
    let c = cam(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0);
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, Vec3::new(-1.0e8, 0.0, -0.3), Vec3::new(1.0e8, 0.0, -0.3), FG);
    assert_eq!(lit(&fb), (0..160).map(|x| (x, 60)).collect::<Vec<_>>());
    // Same with both ends off the same side of the image: nothing is drawn.
    let mut fb = fresh(160, 120);
    draw_line_3d(&mut fb, &c, Vec3::new(1.0e8, 0.0, -0.3), Vec3::new(2.0e8, 0.0, -0.3), FG);
    assert!(lit(&fb).is_empty());
}

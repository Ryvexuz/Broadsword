//! Tests for the replay module: the parser and its refusals, the cube edges, the hard cases, and the golden
//! images of three frames of the real crash-sim trajectory in `tests/fixtures/trajectory.csv`.
//!
//! Each golden image is checked twice: byte for byte against `tests/golden/<name>.ppm`, and, independently
//! of the renderer's own matrices, against a second projection written here in f64 (`Probe`), a naive
//! Bresenham walk, and pixel statistics. The golden file only says "nothing changed"; these checks say the
//! picture is right.

use render::replay::{
    cube_edges, BACKGROUND, EDGE_COLOR, GROUND_COLOR, GROUND_HALF_EXTENT, GROUND_SPACING, NODES, NODE_COLOR,
    NODE_SIZE,
};
use render::{draw_frame, Camera, Framebuffer, Trajectory, Vec3};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

const WIDTH: usize = 240;
const HEIGHT: usize = 180;
/// A node counts as touching the ground when its y is below this (metres). Used to find first contact.
const CONTACT_Y: f32 = 0.01;

fn fixture_text() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trajectory.csv")).unwrap()
}

fn fixture() -> Trajectory {
    Trajectory::parse(&fixture_text()).unwrap()
}

// ---------------------------------------------------------------- parser

/// A well-formed CSV of `frames` frames, one second apart, with node i at (i, t, 0).
fn csv(frames: usize) -> String {
    let mut s = String::from("t,node,x,y,z\n");
    for f in 0..frames {
        for n in 0..8 {
            s += &format!("{f}.0,{n},{n}.0,{f}.5,0.0\n");
        }
    }
    s
}

fn refuses(text: &str, line: usize, words: &str) {
    let err = Trajectory::parse(text).expect_err("should be refused");
    assert!(err.starts_with(&format!("line {line}:")), "wrong line in `{err}` (wanted line {line})");
    assert!(err.contains(words), "`{err}` should mention `{words}`");
}

#[test]
fn parse_reads_the_real_fixture() {
    let t = fixture();
    assert_eq!(t.len(), 51);
    assert_eq!(t.frames()[0].0, 0.0);
    assert_eq!(t.duration(), 5.0);
    // First row of the file: t=0, node 0 at (-0.4585, 2.0868, -0.5417).
    assert_eq!(t.frames()[0].1[0], Vec3::new(-0.4585, 2.0868, -0.5417));
    // Last row: node 7 at (0.5206, 0.9997, 0.4550).
    assert_eq!(t.frames()[50].1[7], Vec3::new(0.5206, 0.9997, 0.4550));
    // Times are strictly increasing in this file (steps of 0.1 s).
    assert!(t.frames().windows(2).all(|w| w[1].0 > w[0].0));
}

#[test]
fn parse_accepts_crlf_line_ends_a_bom_and_a_missing_final_newline() {
    assert_eq!(Trajectory::parse(&csv(2).replace('\n', "\r\n")).unwrap().len(), 2);
    assert_eq!(Trajectory::parse(&format!("\u{feff}{}", csv(1))).unwrap().len(), 1);
    assert_eq!(Trajectory::parse(csv(3).trim_end()).unwrap().len(), 3);
}

#[test]
fn parse_refuses_an_empty_file() {
    refuses("", 1, "empty");
    // A header and nothing else has no frames either.
    refuses("t,node,x,y,z\n", 1, "no data rows");
}

#[test]
fn parse_refuses_a_wrong_header() {
    refuses("time,node,x,y,z\n0,0,0,0,0\n", 1, "wrong header");
    refuses("0.0,0,1,2,3\n", 1, "wrong header");
    refuses("t,node,x,y\n", 1, "wrong header");
}

#[test]
fn parse_refuses_a_non_number_and_names_the_line() {
    let good = csv(1);
    // x of node 2 (file line 4) is not a number.
    refuses(&good.replacen("2.0,0.5,0.0", "abc,0.5,0.0", 1), 4, "x `abc`");
    refuses(&good.replacen("0.0,0,0.0,0.5,0.0", "0.0,0,0.0,oops,0.0", 1), 2, "y `oops`");
    // Time, node and the NaN/inf spellings that Rust would otherwise accept.
    refuses("t,node,x,y,z\nx,0,0,0,0\n", 2, "time `x`");
    refuses("t,node,x,y,z\n0,zero,0,0,0\n", 2, "node `zero`");
    refuses("t,node,x,y,z\n0,0,NaN,0,0\n", 2, "not a number");
    refuses("t,node,x,y,z\n0,0,0,inf,0\n", 2, "not a number");
    refuses("t,node,x,y,z\n0,0,0,0,\n", 2, "z ``");
    // Wrong field count.
    refuses("t,node,x,y,z\n0,0,0,0\n", 2, "expected 5 fields");
    refuses("t,node,x,y,z\n0,0,0,0,0,0\n", 2, "expected 5 fields");
    // A fractional node number is not a whole number.
    refuses("t,node,x,y,z\n0,1.5,0,0,0\n", 2, "node `1.5`");
}

#[test]
fn parse_refuses_a_frame_with_other_than_eight_nodes() {
    // Last frame cut short: 7 nodes. Reported at the last line, naming where the frame began.
    let mut short = csv(2);
    let cut = short.rfind("1.0,7,7.0").unwrap();
    short.truncate(cut);
    refuses(&short, 16, "starting at line 10 has 7 nodes, expected 8");
    // A short frame in the middle is found when the next frame starts.
    let three = csv(3);
    let mid: Vec<&str> = three.lines().filter(|l| !l.starts_with("1.0,7,")).collect();
    refuses(&mid.join("\n"), 17, "starting at line 10 has 7 nodes, expected 8");
    // A ninth node.
    let mut nine = csv(1);
    nine += "0.0,8,8.0,0.5,0.0\n";
    refuses(&nine, 10, "more than 8 nodes");
    // A single row.
    refuses("t,node,x,y,z\n0,0,0,0,0\n", 2, "has 1 nodes");
}

#[test]
fn parse_refuses_nodes_out_of_order() {
    let swapped = csv(1).replacen("0.0,1,1.0", "0.0,2,1.0", 1);
    refuses(&swapped, 3, "found node 2, expected node 1");
    // A frame that starts at node 1.
    refuses("t,node,x,y,z\n0,1,0,0,0\n", 2, "found node 1, expected node 0");
    // A repeated node.
    let dup = csv(1).replacen("0.0,1,1.0", "0.0,0,1.0", 1);
    refuses(&dup, 3, "starting at line 2 has 1 nodes");
    // A node number after a complete frame that is neither 0 nor 8.
    let mut extra = csv(1);
    extra += "0.0,3,3.0,0.5,0.0\n";
    refuses(&extra, 10, "found node 3, expected node 0");
}

#[test]
fn parse_refuses_time_going_backwards_and_time_changing_inside_a_frame() {
    // Frame 1 (line 10) starts before frame 0 ends.
    let back = csv(2).replacen("1.0,0,", "-0.5,0,", 1);
    refuses(&back, 10, "time goes backwards");
    // A later frame earlier than the one before it.
    let back2 = csv(3).replacen("2.0,0,", "0.5,0,", 1);
    refuses(&back2, 18, "time goes backwards");
    // One node of a frame with its own time.
    let split = csv(1).replacen("0.0,4,", "0.1,4,", 1);
    refuses(&split, 6, "differs from the rest of its frame");
    // Equal times in consecutive frames are allowed (a paused moment), not backwards.
    let same = csv(2).replace("1.0,", "0.0,");
    // (the replace also changed node rows' x values only, not the structure)
    assert!(Trajectory::parse(&same).is_ok());
}

// ---------------------------------------------------------------- time and edges

#[test]
fn a_single_frame_trajectory_works() {
    let t = Trajectory::parse(&csv(1)).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t.duration(), 0.0);
    for time in [-1.0, 0.0, 0.5, 1e9, f32::INFINITY, f32::NAN] {
        assert_eq!(t.frame_index_at(time), 0, "time {time}");
    }
    let mut fb = Framebuffer::new(WIDTH, HEIGHT).unwrap();
    draw_frame(&mut fb, &golden_camera(), &t.frames()[0].1);
}

#[test]
fn replaying_past_the_last_frame_holds_it_and_never_panics() {
    let t = fixture();
    let last = t.len() - 1;
    assert_eq!(t.frame_index_at(t.duration()), last);
    for time in [5.0001, 5.1, 100.0, 1e30, f32::INFINITY, f32::MAX] {
        assert_eq!(t.frame_index_at(time), last, "time {time}");
    }
    // Inside the file: the last frame at or before t.
    assert_eq!(t.frame_index_at(0.0), 0);
    assert_eq!(t.frame_index_at(0.05), 0);
    assert_eq!(t.frame_index_at(0.1), 1);
    assert_eq!(t.frame_index_at(0.19), 1);
    assert_eq!(t.frame_index_at(-3.0), 0);
    assert_eq!(t.frame_index_at(f32::NEG_INFINITY), 0);
    assert_eq!(t.frame_index_at(f32::NAN), 0);
    // And drawing the held frame is the same picture however far past the end it is.
    let cam = golden_camera();
    let (mut a, mut b) = (Framebuffer::new(WIDTH, HEIGHT).unwrap(), Framebuffer::new(WIDTH, HEIGHT).unwrap());
    draw_frame(&mut a, &cam, &t.frames()[t.frame_index_at(5.0)].1);
    draw_frame(&mut b, &cam, &t.frames()[t.frame_index_at(1e9)].1);
    assert_eq!(a, b);
}

#[test]
fn cube_edges_are_the_node_pairs_differing_in_one_bit() {
    let edges = cube_edges();
    let mut expected = Vec::new();
    for a in 0..8usize {
        for b in (a + 1)..8 {
            if (a ^ b).count_ones() == 1 {
                expected.push((a, b));
            }
        }
    }
    assert_eq!(expected.len(), 12);
    let mut got = edges.to_vec();
    got.sort();
    assert_eq!(got, expected);
    // Hand list: the bottom face 0-1, 0-4, 1-5, 4-5 (y bit clear), the top face 2-3, 2-6, 3-7, 6-7, and the
    // four uprights 0-2, 1-3, 4-6, 5-7.
    let by_hand = [(0, 1), (0, 2), (0, 4), (1, 3), (1, 5), (2, 3), (2, 6), (3, 7), (4, 5), (4, 6), (5, 7), (6, 7)];
    assert_eq!(got, by_hand.to_vec());
    // In the real file, every one of these edges is the cube's edge length (about 1) in the first frame.
    let traj = fixture();
    let frame = &traj.frames()[0].1;
    for (a, b) in edges {
        let len = (frame[a] - frame[b]).length();
        assert!((len - 1.0).abs() < 0.01, "edge {a}-{b} is {len} long");
    }
}

// ---------------------------------------------------------------- an independent projection

/// A second, independent implementation of the camera maths in f64 (no matrices), used only to check images.
struct Probe {
    pos: [f64; 3],
    f: [f64; 3],
    r: [f64; 3],
    u: [f64; 3],
    tan_half_fov: f64,
    aspect: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Probe {
    fn new(cam: &Camera) -> Probe {
        let (yaw, pitch) = (cam.yaw as f64, cam.pitch as f64);
        let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
        Probe {
            pos: [cam.position.x as f64, cam.position.y as f64, cam.position.z as f64],
            f: [sy * cp, sp, -cy * cp],
            r: [cy, 0.0, sy],
            u: [-sy * sp, cp, cy * sp],
            tan_half_fov: (cam.fov_y as f64 / 2.0).tan(),
            aspect: cam.aspect as f64,
        }
    }

    /// Screen position of a world point (the image is `WIDTH` x `HEIGHT`), for a point in front of the camera.
    fn project(&self, p: Vec3) -> (f64, f64) {
        let d = [p.x as f64 - self.pos[0], p.y as f64 - self.pos[1], p.z as f64 - self.pos[2]];
        let depth = dot(d, self.f);
        assert!(depth > 0.1, "probe only handles points in front of the near plane");
        let nx = dot(d, self.r) / (depth * self.tan_half_fov * self.aspect);
        let ny = dot(d, self.u) / (depth * self.tan_half_fov);
        ((nx * 0.5 + 0.5) * WIDTH as f64, (0.5 - ny * 0.5) * HEIGHT as f64)
    }

    fn pixel(&self, p: Vec3) -> (i32, i32) {
        let (x, y) = self.project(p);
        ((x + 0.5).floor() as i32, (y + 0.5).floor() as i32)
    }
}

/// Plain Bresenham from the textbook (all four directions), no clipping: the set of pixels on the line.
fn naive_line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = (if a.0 < b.0 { 1 } else { -1 }, if a.1 < b.1 { 1 } else { -1 });
    let mut err = dx + dy;
    let mut out = vec![(x, y)];
    while (x, y) != b {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        out.push((x, y));
    }
    out
}

fn in_image(p: &(i32, i32)) -> bool {
    p.0 >= 0 && p.1 >= 0 && (p.0 as usize) < WIDTH && (p.1 as usize) < HEIGHT
}

fn pixels_of(fb: &Framebuffer, color: u32) -> HashSet<(i32, i32)> {
    let mut s = HashSet::new();
    for y in 0..fb.height() as i32 {
        for x in 0..fb.width() as i32 {
            if fb.get(x, y) == Some(color) {
                s.insert((x, y));
            }
        }
    }
    s
}

fn near(set: &HashSet<(i32, i32)>, p: (i32, i32)) -> bool {
    (-1..=1).any(|dx| (-1..=1).any(|dy| set.contains(&(p.0 + dx, p.1 + dy))))
}

fn grid_lines() -> Vec<(Vec3, Vec3)> {
    let e = GROUND_HALF_EXTENT as f32 * GROUND_SPACING;
    let mut v = Vec::new();
    for i in -GROUND_HALF_EXTENT..=GROUND_HALF_EXTENT {
        let t = i as f32 * GROUND_SPACING;
        v.push((Vec3::new(t, 0.0, -e), Vec3::new(t, 0.0, e)));
        v.push((Vec3::new(-e, 0.0, t), Vec3::new(e, 0.0, t)));
    }
    v
}

/// Checks a rendered frame against the independent model and returns (grid, edge, node) pixel counts.
/// Rule: an observed pixel of a colour must be within 1 pixel of the independent set for that colour (nothing
/// is drawn that should not be); an expected pixel must have an observed one within 1 pixel unless something
/// drawn later (edges over the grid, nodes over both) can cover it (nothing is missing). The 1 pixel slack
/// is for f32 versus f64 rounding at pixel centres.
fn verify_against_probe(fb: &Framebuffer, cam: &Camera, frame: &[Vec3; NODES]) -> (usize, usize, usize) {
    let probe = Probe::new(cam);
    let mut grid: HashSet<(i32, i32)> = HashSet::new();
    for (a, b) in grid_lines() {
        grid.extend(naive_line(probe.pixel(a), probe.pixel(b)).into_iter().filter(in_image));
    }
    let mut edges: HashSet<(i32, i32)> = HashSet::new();
    for (a, b) in cube_edges() {
        edges.extend(naive_line(probe.pixel(frame[a]), probe.pixel(frame[b])).into_iter().filter(in_image));
    }
    let mut nodes: HashSet<(i32, i32)> = HashSet::new();
    let half = NODE_SIZE as i32 / 2;
    for p in frame {
        let (cx, cy) = probe.pixel(*p);
        for dy in -half..=half {
            for dx in -half..=half {
                if in_image(&(cx + dx, cy + dy)) {
                    nodes.insert((cx + dx, cy + dy));
                }
            }
        }
    }
    let (og, oe, on) = (pixels_of(fb, GROUND_COLOR), pixels_of(fb, EDGE_COLOR), pixels_of(fb, NODE_COLOR));
    // Nothing is drawn that should not be.
    for &p in &og {
        assert!(near(&grid, p), "stray grid pixel {p:?}");
    }
    for &p in &oe {
        assert!(near(&edges, p), "stray edge pixel {p:?}");
    }
    for &p in &on {
        assert!(nodes.contains(&p), "stray node pixel {p:?}");
    }
    // Nothing is missing, unless covered by something drawn later.
    for &p in &edges {
        assert!(near(&oe, p) || near(&on, p), "edge pixel {p:?} missing");
    }
    for &p in &grid {
        assert!(near(&og, p) || near(&oe, p) || near(&on, p), "grid pixel {p:?} missing");
    }
    for &p in &nodes {
        assert!(on.contains(&p), "node pixel {p:?} missing");
    }
    // Everything else is background.
    let total = WIDTH * HEIGHT;
    let bg = fb.pixels().iter().filter(|&&c| c == BACKGROUND).count();
    assert_eq!(bg + og.len() + oe.len() + on.len(), total, "pixels of an unknown colour");
    // Statistics: with the 12 cube edges drawn over each other, the edge pixels can only be as many as
    // the independent walk counts, within the slack of the nodes covering some and 1-pixel shifts.
    let (ne, no) = (edges.len() as f64, nodes.len() as f64);
    assert!((oe.len() as f64) > ne * 0.6 - no && (oe.len() as f64) < ne * 1.3, "edge count {} vs {}", oe.len(), ne);
    (og.len(), oe.len(), on.len())
}

// ---------------------------------------------------------------- golden images

fn golden_camera() -> Camera {
    let position = Vec3::new(3.5, 2.2, 5.5);
    let f = (Vec3::new(0.0, 1.2, 0.0) - position).normalized();
    Camera {
        position,
        yaw: f.x.atan2(-f.z),
        pitch: f.y.asin(),
        fov_y: 60f32.to_radians(),
        aspect: WIDTH as f32 / HEIGHT as f32,
        near: 0.1,
        far: 100.0,
        move_speed: 1.0,
        turn_speed: 1.0,
    }
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden")
}

/// Byte-for-byte comparison with `tests/golden/<name>.ppm`; see `tests/golden.rs` for how to accept a change.
fn check(name: &str, fb: &Framebuffer) {
    let golden = golden_dir().join(format!("{name}.ppm"));
    let actual_path = golden_dir().join(format!("{name}.actual.ppm"));
    let actual = fb.to_ppm();
    let Ok(expected) = fs::read(&golden) else {
        fs::write(&actual_path, &actual).unwrap();
        panic!(
            "golden image {name}.ppm is missing. The rendered image was written to {}. After checking it, accept \
             it by hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
            actual_path.display()
        );
    };
    if expected == actual {
        let _ = fs::remove_file(&actual_path);
        return;
    }
    fs::write(&actual_path, &actual).unwrap();
    let differing = match Framebuffer::from_ppm(&expected) {
        Ok(e) if e.width() == fb.width() && e.height() == fb.height() => {
            e.pixels().iter().zip(fb.pixels()).filter(|(a, b)| a != b).count().to_string()
        }
        _ => "an unreadable or different-sized golden: some".to_string(),
    };
    panic!(
        "golden mismatch for {name}: {differing} pixels differ. Actual image written to {}. If the change is \
         intended, accept it by hand: copy {name}.actual.ppm over {name}.ppm in tests/golden.",
        actual_path.display()
    );
}

fn render(frame: &[Vec3; NODES]) -> Framebuffer {
    let mut fb = Framebuffer::new(WIDTH, HEIGHT).unwrap();
    draw_frame(&mut fb, &golden_camera(), frame);
    fb
}

fn lowest_y(frame: &[Vec3; NODES]) -> f32 {
    frame.iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
}

/// First frame whose lowest node is within `CONTACT_Y` of the ground.
fn first_contact_index(t: &Trajectory) -> usize {
    t.frames().iter().position(|f| lowest_y(&f.1) < CONTACT_Y).expect("the cube lands in the fixture")
}

#[test]
fn golden_replay_frame_0_cube_in_the_air_tilted() {
    let t = fixture();
    let frame = &t.frames()[0].1;
    assert!(lowest_y(frame) > 1.9, "frame 0 is in the air: lowest node at {}", lowest_y(frame));
    // Tilted: the 4 nodes of the y-bit-clear face are at different heights.
    assert!(frame[1].y - frame[0].y > 0.05 && frame[3].y - frame[2].y > 0.05);
    let fb = render(frame);
    let (g, e, n) = verify_against_probe(&fb, &golden_camera(), frame);
    eprintln!("frame 0: grid {g}, edge {e}, node {n} pixels");
    check("replay_frame_first", &fb);
}

#[test]
fn golden_replay_first_ground_contact() {
    let t = fixture();
    let i = first_contact_index(&t);
    assert_eq!(i, 7, "the cube's first touch in the fixture is frame 7 (t = 0.7 s)");
    assert_eq!(t.frames()[i].0, 0.7);
    // It is the closest frame to contact: the one before is clearly above ground.
    assert!(lowest_y(&t.frames()[i - 1].1) > 0.2);
    let frame = &t.frames()[i].1;
    let fb = render(frame);
    let (g, e, n) = verify_against_probe(&fb, &golden_camera(), frame);
    eprintln!("contact frame {i}: grid {g}, edge {e}, node {n} pixels");
    check("replay_frame_contact", &fb);
}

#[test]
fn golden_replay_last_frame_cube_settled_on_the_ground() {
    let t = fixture();
    let frame = &t.frames()[t.len() - 1].1;
    // Settled: four nodes on the ground (y = 0) and four one metre above.
    let mut ys: Vec<f32> = frame.iter().map(|p| p.y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(ys[..4].iter().all(|y| y.abs() < 1e-3), "{ys:?}");
    assert!(ys[4..].iter().all(|y| (y - 1.0).abs() < 1e-2), "{ys:?}");
    let cam = golden_camera();
    let fb = render(frame);
    let (g, e, n) = verify_against_probe(&fb, &cam, frame);
    eprintln!("last frame: grid {g}, edge {e}, node {n} pixels");

    // Done check 4: the cube's lowest corner lies on the ground line, within 1 pixel. The lowest corner on
    // screen is the bottom node nearest the camera (largest screen y). "On the ground" means the node
    // marker's centre sits where the ground-plane point directly below that corner projects, y = 0 (all
    // four bottom nodes are at y = 0.0000 in the file).
    let probe = Probe::new(&cam);
    let (idx, _) = (0..NODES)
        .filter(|&i| frame[i].y.abs() < 1e-3)
        .map(|i| (i, probe.project(frame[i]).1))
        .fold((usize::MAX, f64::MIN), |best, c| if c.1 > best.1 { c } else { best });
    let corner = frame[idx];
    let below = Vec3::new(corner.x, 0.0, corner.z);
    let (ex, ey) = probe.project(below);
    // The node marker found in the image: centroid of the node-coloured pixels within 3 pixels of the corner.
    let on = pixels_of(&fb, NODE_COLOR);
    let near_px: Vec<(i32, i32)> =
        on.iter().copied().filter(|p| (p.0 as f64 - ex).abs() <= 3.0 && (p.1 as f64 - ey).abs() <= 3.0).collect();
    assert_eq!(near_px.len(), (NODE_SIZE * NODE_SIZE) as usize, "one whole node marker near the corner");
    let cx = near_px.iter().map(|p| p.0 as f64).sum::<f64>() / near_px.len() as f64;
    let cy = near_px.iter().map(|p| p.1 as f64).sum::<f64>() / near_px.len() as f64;
    let dist = ((cx - ex).powi(2) + (cy - ey).powi(2)).sqrt();
    eprintln!(
        "lowest corner is node {idx} at world {corner:?}; ground point below it projects to ({ex:.2}, {ey:.2}); \
         node marker centre in the image ({cx}, {cy}); distance {dist:.2} px"
    );
    assert!(dist <= 1.0, "lowest corner is {dist} px from the ground point below it");
    check("replay_frame_last", &fb);
}

#[test]
fn camera_below_the_ground_draws_without_panic_and_matches_the_independent_model() {
    let t = fixture();
    let frame = &t.frames()[t.len() - 1].1;
    // Under the ground looking up at the cube; every point is still in front of the camera.
    let position = Vec3::new(0.0, -2.0, 7.0);
    let f = (Vec3::new(0.0, 1.0, 0.0) - position).normalized();
    let cam = Camera { position, yaw: f.x.atan2(-f.z), pitch: f.y.asin(), ..golden_camera() };
    let mut fb = Framebuffer::new(WIDTH, HEIGHT).unwrap();
    draw_frame(&mut fb, &cam, frame);
    let (g, e, n) = verify_against_probe(&fb, &cam, frame);
    assert!(g > 100 && e > 50 && n > 0, "grid {g}, edge {e}, node {n}");
    // Also under the ground and close to it, looking along it: lines run behind and through the camera.
    // It must simply not panic, and still draw the cube if it is ahead.
    let cam = Camera { position: Vec3::new(0.3, -0.5, 1.0), yaw: 0.2, pitch: 0.4, ..golden_camera() };
    draw_frame(&mut fb, &cam, frame);
    let cam = Camera { position: Vec3::new(0.0, -1e6, 0.0), yaw: 0.0, pitch: 0.0, ..golden_camera() };
    draw_frame(&mut fb, &cam, frame);
}

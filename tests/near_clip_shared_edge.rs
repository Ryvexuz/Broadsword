//! Two triangles that share an edge crossing the near plane must leave no uncovered pixel along it: the crossing
//! point is computed the same way whichever triangle walks the edge, and in whichever direction.
//!
//! The two crossings computed from opposite ends differ by a few f32 rounding steps, which opens a gap only when
//! one lands just across a 1/16-pixel snap boundary. So the test sweeps many pseudo-random quads (a fixed
//! generator, so the run is repeatable) on a small target and counts every pixel left at the clear colour on the
//! shared edge.

mod common;

use common::*;
use render::{Framebuffer, Vec3};

const W: usize = 48;
const H: usize = 36;
const BG: u32 = 0x0010_1018;
const QUADS: usize = 30_000;

type Pt = [f32; 3];

/// A small repeatable generator (64-bit LCG, high bits): uniform in [lo, hi).
struct Rng(u64);
impl Rng {
    fn next(&mut self, lo: f32, hi: f32) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        lo + (hi - lo) * ((self.0 >> 40) as f32 / (1u64 << 24) as f32)
    }
}

fn render_pair(cam: &render::Camera, t1: [Pt; 3], t2: [Pt; 3]) -> Framebuffer {
    let (m1, m2) = (tri_mesh(t1), tri_mesh(t2));
    let objs = [object(&m1, 0x00D0_3030), object(&m2, 0x0030_30D0)];
    render_scene(W, H, cam, &objs, ambient_only(), None, BG).0
}

/// Counts the pixels within one pixel of the on-screen shared edge b-d (from b to its near-plane crossing) that
/// are at the clear colour. Returns (pixels examined, uncovered ones).
fn gaps_on_edge(fb: &Framebuffer, rc: &RefCamera, near: f64, tris: [[Pt; 3]; 2], b: Pt, d: Pt) -> (usize, Vec<(usize, usize)>) {
    let (b, d) = (b.map(|v| v as f64), d.map(|v| v as f64));
    let t = (-near - b[2]) / (d[2] - b[2]);
    let x = [b[0] + (d[0] - b[0]) * t, b[1] + (d[1] - b[1]) * t, -near];
    let (p, q) = (rc.project(b), rc.project(x));
    let len2 = (q.0 - p.0).powi(2) + (q.1 - p.1).powi(2);
    // Only pixels the independent model (exact f64 clip, `common`) puts inside one of the triangles count, so
    // a pixel outside the quad's outline can never be mistaken for a gap.
    let to64 = |t: [Pt; 3]| t.map(|q| [q[0] as f64, q[1] as f64, q[2] as f64]);
    let masks = tris.map(|t| pixels_ref(W, H, &rc.clipped_polygon(to64(t))));
    let (mut seen, mut bad) = (0, Vec::new());
    for y in 0..H {
        for xx in 0..W {
            let (dx, dy) = (xx as f64 - p.0, y as f64 - p.1);
            // Stay clear of both ends of the segment, where the quad's outline (not the shared edge) decides.
            let u = (dx * (q.0 - p.0) + dy * (q.1 - p.1)) / len2;
            if !(0.05..0.95).contains(&u) {
                continue;
            }
            let dist = (dx * (q.1 - p.1) - dy * (q.0 - p.0)).abs() / len2.sqrt();
            if dist <= 1.0 && (masks[0][y * W + xx] || masks[1][y * W + xx]) {
                seen += 1;
                if fb.pixels()[y * W + xx] == BG {
                    bad.push((xx, y));
                }
            }
        }
    }
    (seen, bad)
}

#[test]
fn shared_edge_across_the_near_plane_leaves_no_gap() {
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, W, H);
    let rc = RefCamera::new(&cam, W, H);
    let near = cam.near as f64;
    let mut rng = Rng(0x5EED);
    let (mut examined, mut gaps, mut worst) = (0usize, 0usize, None);
    for _ in 0..QUADS {
        // A floor quad 0.3 below the camera, split along b-d; b is in front of the near plane, d is behind it.
        // Counter-clockwise seen from above: a, d, b and b, d, c (the shared edge is walked in opposite directions).
        let a = [rng.next(-1.0, -0.3), -0.3, rng.next(-2.0, -0.8)];
        let b = [rng.next(0.3, 1.0), -0.3, rng.next(-2.0, -0.8)];
        let c = [rng.next(0.2, 1.0), -0.3, rng.next(0.5, 4.0)];
        let d = [rng.next(-1.0, -0.2), -0.3, rng.next(0.5, 4.0)];
        let fb = render_pair(&cam, [a, d, b], [b, d, c]);
        let (n, bad) = gaps_on_edge(&fb, &rc, near, [[a, d, b], [b, d, c]], b, d);
        examined += n;
        gaps += bad.len();
        if worst.is_none() && !bad.is_empty() {
            worst = Some((a, b, c, d, bad));
        }
    }
    println!("near-clip shared edge: {QUADS} quads, {examined} pixels examined along the edge, {gaps} left at the clear colour");
    if let Some((a, b, c, d, bad)) = worst {
        panic!("a pixel along the shared edge was left uncovered, first case: a {a:?} b {b:?} c {c:?} d {d:?}: {bad:?}");
    }
    assert!(examined > QUADS * 10, "the edge must really be examined");
}

#[test]
fn golden_quad_split_across_the_near_plane() {
    let cam = camera(Vec3::new(0.0, 0.0, 0.0), 0.0, 0.0, 120, 90);
    let (a, b, c, d) = ([-0.8f32, -0.3, -1.2], [0.8, -0.3, -1.2], [1.4, -0.3, 1.5], [-1.4, -0.3, 1.5]);
    let (m1, m2) = (tri_mesh([a, d, b]), tri_mesh([b, d, c]));
    let objs = [object(&m1, 0x00D0_3030), object(&m2, 0x0030_30D0)];
    let (fb, _) = render_scene(120, 90, &cam, &objs, ambient_only(), None, BG);
    // The same quad with the triangles and their shared edge listed the other way round covers the same pixels.
    let (n1, n2) = (tri_mesh([d, b, a]), tri_mesh([c, b, d]));
    let objs2 = [object(&n1, 0x00D0_3030), object(&n2, 0x0030_30D0)];
    let (fb2, _) = render_scene(120, 90, &cam, &objs2, ambient_only(), None, BG);
    assert_eq!(fb.pixels(), fb2.pixels(), "vertex order of the shared edge changed the picture");
    let (red, blue) = (fb.pixels().iter().filter(|&&p| p == 0x00D0_3030).count(), fb.pixels().iter().filter(|&&p| p == 0x0030_30D0).count());
    assert!(red > 300 && blue > 300, "both triangles must be visible: red {red}, blue {blue}");
    check("near_clip_shared_edge", &fb);
}

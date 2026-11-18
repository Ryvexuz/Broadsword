//! Replay of a crash-sim trajectory: a parser for crash-sim's CSV and the drawing of one frame.
//!
//! The renderer and crash-sim are separate projects that meet only through this file format:
//!
//! ```text
//! t,node,x,y,z
//! 0.0000,0,-0.4585,2.0868,-0.5417
//! ...                                   (one row per node per frame, 8 nodes per frame)
//! ```
//!
//! crash-sim numbers node `i` by its bits: x is bit 0, y is bit 1, z is bit 2 (`Sim::new` builds the corner
//! as `(i & 1, i & 2, i & 4) * edge - edge / 2`). So the cube's 12 edges are the node pairs whose numbers
//! differ in exactly one bit.

use crate::camera::{draw_line_3d, draw_point_3d, Camera};
use crate::framebuffer::Framebuffer;
use crate::math::Vec3;

/// Nodes per frame: the 8 corners of the cube.
pub const NODES: usize = 8;
/// The header line crash-sim writes.
pub const HEADER: &str = "t,node,x,y,z";

/// Colours used by `draw_frame`, `0x00RRGGBB`. Public so tests can classify pixels.
pub const BACKGROUND: u32 = 0x0010_1018;
pub const GROUND_COLOR: u32 = 0x0040_4860;
pub const EDGE_COLOR: u32 = 0x00FF_C040;
pub const NODE_COLOR: u32 = 0x00FF_FFFF;
/// Side of the square drawn for each node, in pixels (odd, so the node pixel is its centre).
pub const NODE_SIZE: u32 = 3;
/// The ground grid covers x and z from `-GROUND_HALF_EXTENT` to `+GROUND_HALF_EXTENT` at y = 0, with a line
/// every `GROUND_SPACING` world units (the world unit is crash-sim's metre).
pub const GROUND_HALF_EXTENT: i32 = 4;
pub const GROUND_SPACING: f32 = 1.0;

/// One frame: its time in seconds and the 8 node positions.
pub type Frame = (f32, [Vec3; NODES]);

/// A parsed trajectory: at least one frame, times never going backwards.
#[derive(Clone, Debug, PartialEq)]
pub struct Trajectory {
    frames: Vec<Frame>,
}

impl Trajectory {
    /// Parses crash-sim's trajectory CSV. Refuses with a message that names the line (1 is the header):
    /// an empty file, a wrong header, a non-number, a frame with other than 8 nodes, nodes out of order,
    /// or time going backwards.
    pub fn parse(text: &str) -> Result<Trajectory, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        match lines.next() {
            None => return Err("line 1: the file is empty".to_string()),
            Some((n, header)) if header.trim() != HEADER => {
                return Err(format!("line {n}: wrong header `{}`, expected `{HEADER}`", header.trim()));
            }
            Some(_) => {}
        }
        let mut frames: Vec<Frame> = Vec::new();
        // The frame being filled: its first line, its time, and its nodes so far.
        let mut open: Option<(usize, f32, Vec<Vec3>)> = None;
        let mut last_line = 1;
        for (n, line) in lines {
            last_line = n;
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            if fields.len() != 5 {
                return Err(format!("line {n}: expected 5 fields (t,node,x,y,z), found {}", fields.len()));
            }
            let number = |idx: usize, what: &str| -> Result<f32, String> {
                match fields[idx].parse::<f32>() {
                    Ok(v) if v.is_finite() => Ok(v),
                    _ => Err(format!("line {n}: {what} `{}` is not a number", fields[idx])),
                }
            };
            let t = number(0, "time")?;
            let node: usize = fields[1]
                .parse()
                .map_err(|_| format!("line {n}: node `{}` is not a whole number", fields[1]))?;
            let p = Vec3::new(number(2, "x")?, number(3, "y")?, number(4, "z")?);

            let expected = open.as_ref().map_or(0, |(_, _, nodes)| nodes.len());
            if expected == NODES && node == NODES {
                return Err(format!("line {n}: frame has more than {NODES} nodes (found node {node})"));
            }
            if node == 0 && expected != 0 && expected != NODES {
                let (start, _, _) = open.as_ref().unwrap();
                return Err(format!(
                    "line {n}: the frame starting at line {start} has {expected} nodes, expected {NODES}"
                ));
            }
            if node != expected % NODES {
                return Err(format!("line {n}: nodes out of order: found node {node}, expected node {}", expected % NODES));
            }
            if node == 0 {
                // A new frame begins; the previous one (if any) was complete.
                if let Some((_, t_prev, nodes)) = open.take() {
                    frames.push((t_prev, nodes.try_into().expect("a complete frame has 8 nodes")));
                }
                if let Some(&(t_prev, _)) = frames.last() {
                    if t < t_prev {
                        return Err(format!("line {n}: time goes backwards: {t} after {t_prev}"));
                    }
                }
                open = Some((n, t, vec![p]));
            } else {
                let (_, t_frame, nodes) = open.as_mut().expect("a frame is open when node > 0");
                if t != *t_frame {
                    return Err(format!("line {n}: time {t} differs from the rest of its frame ({t_frame})"));
                }
                nodes.push(p);
            }
        }
        match open {
            None => Err(format!("line {last_line}: no data rows after the header")),
            Some((start, _, nodes)) if nodes.len() != NODES => Err(format!(
                "line {last_line}: the frame starting at line {start} has {} nodes, expected {NODES}",
                nodes.len()
            )),
            Some((_, t, nodes)) => {
                frames.push((t, nodes.try_into().expect("a complete frame has 8 nodes")));
                Ok(Trajectory { frames })
            }
        }
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Time of the last frame, in seconds.
    pub fn duration(&self) -> f32 {
        self.frames[self.frames.len() - 1].0
    }

    /// Index of the frame to show at time `t`: the last frame whose time is `<= t`. Before the first frame
    /// that is frame 0; past the last it is the last frame (the replay holds the end). A NaN `t` gives 0.
    pub fn frame_index_at(&self, t: f32) -> usize {
        self.frames.partition_point(|f| f.0 <= t).saturating_sub(1)
    }
}

/// The 12 cube edges as node-number pairs: every pair that differs in exactly one bit, lower number first.
pub fn cube_edges() -> [(usize, usize); 12] {
    let mut edges = [(0, 0); 12];
    let mut n = 0;
    for a in 0..NODES {
        for bit in [1usize, 2, 4] {
            let b = a ^ bit;
            if a < b {
                edges[n] = (a, b);
                n += 1;
            }
        }
    }
    debug_assert_eq!(n, 12);
    edges
}

/// Draws one frame: clears the framebuffer, then the ground grid at y = 0, the 12 cube edges and the 8
/// nodes as points. The viewer and the tests both use this function.
pub fn draw_frame(fb: &mut Framebuffer, cam: &Camera, frame: &[Vec3; NODES]) {
    fb.clear(BACKGROUND);
    let e = GROUND_HALF_EXTENT as f32 * GROUND_SPACING;
    for i in -GROUND_HALF_EXTENT..=GROUND_HALF_EXTENT {
        let t = i as f32 * GROUND_SPACING;
        draw_line_3d(fb, cam, Vec3::new(t, 0.0, -e), Vec3::new(t, 0.0, e), GROUND_COLOR);
        draw_line_3d(fb, cam, Vec3::new(-e, 0.0, t), Vec3::new(e, 0.0, t), GROUND_COLOR);
    }
    for (a, b) in cube_edges() {
        draw_line_3d(fb, cam, frame[a], frame[b], EDGE_COLOR);
    }
    for p in frame {
        draw_point_3d(fb, cam, *p, NODE_SIZE, NODE_COLOR);
    }
}

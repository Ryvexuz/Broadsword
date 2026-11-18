//! A perspective fly camera, and 3D line/point drawing through it.

use crate::framebuffer::Framebuffer;
use crate::math::{Mat4, Vec3};

/// Pitch is kept just short of straight up/down so the view never flips. This is a maths limit, not a tuning knob.
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

/// Which fly controls are held this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlyInput {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub look_up: bool,
    pub look_down: bool,
}

/// Yaw 0 looks down -Z; positive yaw turns toward +X. Positive pitch looks up. Angles are radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Width divided by height of the image this camera renders to.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    /// World units per second.
    pub move_speed: f32,
    /// Radians per second.
    pub turn_speed: f32,
}

fn axis(plus: bool, minus: bool) -> f32 {
    (plus as i32 - minus as i32) as f32
}

impl Camera {
    /// The unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    /// The unit vector pointing to the camera's right (always level).
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at(self.position, self.position + self.forward(), Vec3::new(0.0, 1.0, 0.0))
    }

    pub fn proj(&self) -> Mat4 {
        Mat4::perspective(self.fov_y, self.aspect, self.near, self.far)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj().mul(&self.view())
    }

    /// Moves and turns the camera for `dt` seconds. Forward/back follow the full view direction;
    /// left/right slide level; up/down move along world Y. Pure: no clock, no window.
    pub fn fly(&mut self, input: &FlyInput, dt: f32) {
        self.yaw += axis(input.turn_right, input.turn_left) * self.turn_speed * dt;
        self.pitch = (self.pitch + axis(input.look_up, input.look_down) * self.turn_speed * dt)
            .clamp(-MAX_PITCH, MAX_PITCH);
        let step = self.move_speed * dt;
        let mv = self.forward() * axis(input.forward, input.back)
            + self.right() * axis(input.right, input.left)
            + Vec3::new(0.0, 1.0, 0.0) * axis(input.up, input.down);
        self.position = self.position + mv * step;
    }

    /// Screen position (pixels, y down) of a world point in an image `width` x `height`, or `None` if the
    /// point is behind the near plane or beyond the far plane. The point may still be off screen.
    pub fn project(&self, world: Vec3, width: usize, height: usize) -> Option<(f32, f32)> {
        let depth = -self.view().transform_point(world)[2];
        if !(depth >= self.near && depth <= self.far) {
            return None;
        }
        self.project_visible(world, width, height)
    }

    fn project_visible(&self, world: Vec3, width: usize, height: usize) -> Option<(f32, f32)> {
        let c = self.view_proj().transform_point(world);
        if !(c[3] > 0.0) {
            return None;
        }
        let nx = c[0] / c[3];
        let ny = c[1] / c[3];
        let sx = (nx * 0.5 + 0.5) * width as f32;
        let sy = (0.5 - ny * 0.5) * height as f32;
        if sx.is_finite() && sy.is_finite() {
            Some((sx, sy))
        } else {
            None
        }
    }

    /// Clips the segment a-b to the depth range [near, far] (in the camera's view). Returns the clipped
    /// endpoints in world space, or `None` if nothing of the segment is in range.
    pub fn clip_line(&self, a: Vec3, b: Vec3) -> Option<(Vec3, Vec3)> {
        let view = self.view();
        let da = -view.transform_point(a)[2];
        let db = -view.transform_point(b)[2];
        if !(da.is_finite() && db.is_finite()) {
            return None;
        }
        let (mut ta, mut tb) = (0.0f32, 1.0f32);
        for bound in [self.near, self.far] {
            let is_near = bound == self.near;
            // `inside(d)` is d >= near for the near plane and d <= far for the far plane.
            let (ia, ib) = if is_near { (da >= bound, db >= bound) } else { (da <= bound, db <= bound) };
            if !ia && !ib {
                return None;
            }
            if ia && ib {
                continue;
            }
            // Parameter where the depth crosses the bound.
            let t = (bound - da) / (db - da);
            if ia {
                tb = tb.min(t);
            } else {
                ta = ta.max(t);
            }
        }
        if ta > tb {
            return None;
        }
        Some((a.lerp(b, ta), a.lerp(b, tb)))
    }
}

/// Pixel for a screen coordinate: `s` lights pixel round(s). Saturates at the `i32` range (NaN gives 0).
fn to_pixel(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

/// Screen coordinates up to this size convert to `i32` pixels exactly, and `Framebuffer::line` clips them
/// itself without changing a single pixel. It is 2^30, a safe margin inside `i32`; it is not a tuning knob.
const EXACT_PIXEL_RANGE: f64 = 1_073_741_824.0;

/// Margin, in pixels, kept around the buffer when a very long line is clipped in 2D: the cut ends then land
/// outside the visible area, so their rounding cannot be seen. A fixed part of the clipping, not a tuning knob.
const GUARD_BAND: f64 = 2.0;

/// Liang-Barsky clip of the segment p0-p1 against the rectangle [xmin, xmax] x [ymin, ymax]. The result
/// lies on the original line (so its slope is unchanged), or is `None` if no part is inside.
fn clip_segment_2d(p0: (f64, f64), p1: (f64, f64), min: (f64, f64), max: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    // For each edge: p * t <= q describes the side that is inside.
    for (p, q) in [(-dx, p0.0 - min.0), (dx, max.0 - p0.0), (-dy, p0.1 - min.1), (dy, max.1 - p0.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None; // parallel to this edge and outside it
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if !(t0 <= t1) {
        return None;
    }
    Some(((p0.0 + dx * t0, p0.1 + dy * t0), (p0.0 + dx * t1, p0.1 + dy * t1)))
}

/// Draws a world-space line, clipped against the camera's near and far planes first so a line passing
/// behind the camera draws correctly instead of wrapping around. A line whose screen ends are far outside
/// the buffer is then clipped in 2D (along the line, so its slope is kept) before it is rasterised.
pub fn draw_line_3d(fb: &mut Framebuffer, cam: &Camera, a: Vec3, b: Vec3, color: u32) {
    let Some((ca, cb)) = cam.clip_line(a, b) else { return };
    let (w, h) = (fb.width(), fb.height());
    let (Some(pa), Some(pb)) = (cam.project_visible(ca, w, h), cam.project_visible(cb, w, h)) else {
        return;
    };
    let (p0, p1) = ((pa.0 as f64, pa.1 as f64), (pb.0 as f64, pb.1 as f64));
    let near_enough = [p0.0, p0.1, p1.0, p1.1].iter().all(|v| v.abs() <= EXACT_PIXEL_RANGE);
    let (p0, p1) = if near_enough {
        (p0, p1)
    } else {
        let min = (-GUARD_BAND, -GUARD_BAND);
        let max = (w as f64 + GUARD_BAND, h as f64 + GUARD_BAND);
        match clip_segment_2d(p0, p1, min, max) {
            Some(c) => c,
            None => return,
        }
    };
    fb.line(to_pixel(p0.0 as f32), to_pixel(p0.1 as f32), to_pixel(p1.0 as f32), to_pixel(p1.1 as f32), color);
}

/// Draws a world-space point as a `size`-pixel square. Points outside the depth range are skipped.
pub fn draw_point_3d(fb: &mut Framebuffer, cam: &Camera, p: Vec3, size: u32, color: u32) {
    if let Some((sx, sy)) = cam.project(p, fb.width(), fb.height()) {
        fb.point(to_pixel(sx), to_pixel(sy), size, color);
    }
}

#[cfg(test)]
mod tests {
    use super::clip_segment_2d;

    #[test]
    fn clipping_keeps_the_line_and_its_slope_at_huge_coordinates() {
        // A line of slope 0.25 through (0, 10), ends a trillion pixels out.
        let (p0, p1) = ((-4.0e12, -1.0e12 + 10.0), (4.0e12, 1.0e12 + 10.0));
        let (a, b) = clip_segment_2d(p0, p1, (-2.0, -2.0), (162.0, 122.0)).expect("it crosses the box");
        for p in [a, b] {
            assert!(p.0 >= -2.0 - 1e-6 && p.0 <= 162.0 + 1e-6 && p.1 >= -2.0 - 1e-6 && p.1 <= 122.0 + 1e-6, "{p:?}");
            // On the original line: y = 10 + 0.25 x.
            assert!((p.1 - (10.0 + 0.25 * p.0)).abs() < 1e-3, "{p:?} is off the line");
        }
        assert!(((b.1 - a.1) / (b.0 - a.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn clipping_rejects_lines_that_miss_the_box_and_handles_axis_parallel_ones() {
        let (min, max) = ((0.0, 0.0), (160.0, 120.0));
        assert!(clip_segment_2d((-1e9, -5.0), (1e9, -5.0), min, max).is_none()); // parallel, outside
        assert!(clip_segment_2d((-1e9, -1e9), (-5.0, 1e9), min, max).is_none()); // runs past the left edge
        assert!(clip_segment_2d((5.0, 5.0), (5.0, 5.0), min, max).is_some()); // a point inside
        assert!(clip_segment_2d((500.0, 5.0), (500.0, 5.0), min, max).is_none()); // a point outside
        let (a, b) = clip_segment_2d((-1e9, 60.0), (1e9, 60.0), min, max).unwrap();
        // The cut ends are the box edges, to within the rounding of a parameter on a 2e9 pixel line.
        assert!(a.0.abs() < 1e-3 && (b.0 - 160.0).abs() < 1e-3 && a.1 == 60.0 && b.1 == 60.0, "{a:?} {b:?}");
    }
}

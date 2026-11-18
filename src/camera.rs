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

fn to_pixel(v: f32) -> i32 {
    // Far enough off screen to be rejected, small enough that walking the line stays cheap.
    const LIMIT: f32 = 1.0e6;
    (v + 0.5).floor().clamp(-LIMIT, LIMIT) as i32
}

/// Draws a world-space line, clipped against the camera's near and far planes first so a line passing
/// behind the camera draws correctly instead of wrapping around.
pub fn draw_line_3d(fb: &mut Framebuffer, cam: &Camera, a: Vec3, b: Vec3, color: u32) {
    let Some((ca, cb)) = cam.clip_line(a, b) else { return };
    let (w, h) = (fb.width(), fb.height());
    let (Some(pa), Some(pb)) = (cam.project_visible(ca, w, h), cam.project_visible(cb, w, h)) else {
        return;
    };
    fb.line(to_pixel(pa.0), to_pixel(pa.1), to_pixel(pb.0), to_pixel(pb.1), color);
}

/// Draws a world-space point as a `size`-pixel square. Points outside the depth range are skipped.
pub fn draw_point_3d(fb: &mut Framebuffer, cam: &Camera, p: Vec3, size: u32, color: u32) {
    if let Some((sx, sy)) = cam.project(p, fb.width(), fb.height()) {
        fb.point(to_pixel(sx), to_pixel(sy), size, color);
    }
}

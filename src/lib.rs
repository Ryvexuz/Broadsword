//! Broadsword's renderer (Stage 4). Std only: everything is drawn in software into a `Framebuffer`, which tests compare
//! against golden images and a Win32 window can show. See README.md.

pub mod camera;
pub mod framebuffer;
pub mod math;
pub mod mesh;
pub mod raster;
pub mod replay;
pub mod scene;
#[cfg(windows)]
pub mod window;

pub use camera::{draw_line_3d, draw_point_3d, Camera, FlyInput};
pub use framebuffer::{Framebuffer, FramebufferError};
pub use math::{Mat4, Vec3};
pub use mesh::{car_body_mesh, car_mesh, car_wheel_centres, ground_plane_mesh, wheel_mesh, Mesh, ObjError};
pub use raster::{
    caster_sphere, fill_triangle, FrameStats, Light, Object, RenderError, Renderer, Scene, ShadowSettings, Sides,
};
pub use replay::{draw_frame, Trajectory};
pub use scene::wire_cube;

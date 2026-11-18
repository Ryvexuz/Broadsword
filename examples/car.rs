//! A car on a turntable, drawn by the software rasterizer with one light and a shadow map.
//!
//!   car                       opens a window; the car turns by itself, W/S/A/D, E/Q and the arrows fly the camera;
//!                             Esc quits
//!   car --frames N            renders N frames with no window, prints the average frame time with its split
//!                             (shadow pass, main pass) and the triangle counts, exits 0
//!   car --present N           opens the window off screen without activation, renders and presents N frames through
//!                             the real pump / fly_input / present path, prints the split including the blit, exits 0
//! Options: --width W --height H (default 640x480), --shadow-size S (default 1024), --segments N (wheel sides, 24).

use render::mesh::car_wheel_centres;
use render::{
    car_body_mesh, caster_sphere, ground_plane_mesh, wheel_mesh, Camera, Framebuffer, Light, Mat4, Object, Renderer, Scene,
    ShadowSettings, Vec3,
};
use std::time::Instant;

/// Time step for the modes that do not follow the clock (`--frames`, `--present`): one 60 Hz frame.
const FIXED_DT: f32 = 1.0 / 60.0;
/// How fast the turntable turns, radians per second.
const TURN_RATE: f32 = 0.9;
const GROUND_HALF: f32 = 30.0;

struct Args {
    frames: Option<usize>,
    present: Option<usize>,
    width: usize,
    height: usize,
    shadow_size: usize,
    segments: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { frames: None, present: None, width: 640, height: 480, shadow_size: 1024, segments: 24 };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| -> Result<usize, String> {
            it.next()
                .ok_or_else(|| format!("{name} needs a value"))?
                .parse()
                .map_err(|_| format!("{name} needs a whole number"))
        };
        match arg.as_str() {
            "--frames" => a.frames = Some(value("--frames")?),
            "--present" => a.present = Some(value("--present")?),
            "--width" => a.width = value("--width")?,
            "--height" => a.height = value("--height")?,
            "--shadow-size" => a.shadow_size = value("--shadow-size")?,
            "--segments" => a.segments = value("--segments")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.frames.is_some() && a.present.is_some() {
        return Err("--frames and --present cannot be used together".to_string());
    }
    Ok(a)
}

fn fail(msg: &str, code: i32) -> ! {
    eprintln!("car: {msg}");
    std::process::exit(code);
}

/// Looks at the car from the front-left and above.
fn start_camera(aspect: f32) -> Camera {
    let position = Vec3::new(-4.6, 2.6, 5.4);
    let f = (Vec3::new(0.0, 0.5, 0.0) - position).normalized();
    Camera {
        position,
        yaw: f.x.atan2(-f.z),
        pitch: f.y.asin(),
        fov_y: 60f32.to_radians(),
        aspect,
        near: 0.1,
        far: 200.0,
        move_speed: 4.0,
        turn_speed: 1.5,
    }
}

/// The meshes and the settings that do not change from frame to frame.
struct World {
    body: render::Mesh,
    wheel: render::Mesh,
    ground: render::Mesh,
    light: Light,
    shadow: ShadowSettings,
}

impl World {
    fn new(shadow_size: usize, segments: usize) -> Result<World, String> {
        let (body, wheel, ground) = (car_body_mesh(), wheel_mesh(segments)?, ground_plane_mesh(GROUND_HALF));
        // The car turns about the vertical axis through its origin, so a sphere around it at the right height
        // always holds it: the map then does not move or change size as the car turns.
        let only_car: Vec<Object> = vec![Object { mesh: &body, model: Mat4::IDENTITY, color: 0, casts_shadow: true }];
        let (centre, radius) = caster_sphere(&only_car).ok_or("the car mesh is empty")?;
        let wheel_reach = car_wheel_centres().iter().map(|c| (Vec3::new(c.x, c.y, c.z) - centre).length() + 0.5).fold(0.0f32, f32::max);
        let shadow = ShadowSettings::new(shadow_size, Vec3::new(0.0, centre.y, 0.0), radius.max(wheel_reach));
        let light = Light { to_light: Vec3::new(-0.5, 1.0, 0.45), ambient: 0.25, diffuse: 0.75 };
        Ok(World { body, wheel, ground, light, shadow })
    }

    fn render(&self, renderer: &mut Renderer, fb: &mut Framebuffer, cam: &Camera, angle: f32) {
        let car = Mat4::rotation_y(angle);
        let mut objects = vec![
            Object { mesh: &self.ground, model: Mat4::IDENTITY, color: 0x0070_A060, casts_shadow: false },
            Object { mesh: &self.body, model: car, color: 0x00B0_2020, casts_shadow: true },
        ];
        for c in car_wheel_centres() {
            objects.push(Object { mesh: &self.wheel, model: car.mul(&Mat4::translation(c)), color: 0x0020_2020, casts_shadow: true });
        }
        let scene = Scene { objects: &objects, light: self.light, shadow: Some(self.shadow), background: 0x0070_90C0 };
        if let Err(e) = renderer.render(fb, cam, &scene) {
            fail(&e.to_string(), 1);
        }
    }
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| fail(&e, 2));
    let world = World::new(args.shadow_size, args.segments).unwrap_or_else(|e| fail(&e, 2));
    let mut fb = Framebuffer::new(args.width, args.height).unwrap_or_else(|e| fail(&e.to_string(), 2));
    let mut cam = start_camera(args.width as f32 / args.height as f32);
    let mut renderer = Renderer::new();

    if let Some(frames) = args.frames {
        if frames == 0 {
            fail("--frames must be at least 1", 2);
        }
        let (mut shadow_ms, mut main_ms) = (0.0, 0.0);
        let start = Instant::now();
        for i in 0..frames {
            world.render(&mut renderer, &mut fb, &cam, i as f32 * FIXED_DT * TURN_RATE);
            let s = renderer.stats();
            shadow_ms += s.shadow_ms;
            main_ms += s.main_ms;
        }
        let total = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let n = frames as f64;
        let s = renderer.stats();
        println!(
            "car: {frames} frames at {}x{}, shadow map {}x{}, {} triangles ({} cast shadows, {} drawn in the main pass)",
            args.width, args.height, args.shadow_size, args.shadow_size, s.triangles_submitted, s.triangles_shadow, s.triangles_main
        );
        println!(
            "car: average frame time {total:.3} ms = shadow pass {:.3} + main pass {:.3} + other {:.3}; blit: none, no window in this mode",
            shadow_ms / n,
            main_ms / n,
            total - (shadow_ms + main_ms) / n
        );
        return;
    }

    #[cfg(windows)]
    {
        use render::window::{Placement, Window};
        let present_n = args.present;
        if present_n == Some(0) {
            fail("--present must be at least 1", 2);
        }
        let placement = if present_n.is_some() { Placement::OffScreen } else { Placement::Normal };
        let mut win = Window::open("Broadsword render: car", args.width, args.height, placement).unwrap_or_else(|e| fail(&e, 1));
        let (mut angle, mut presented) = (0.0f32, 0usize);
        let (mut shadow_ms, mut main_ms, mut blit_ms) = (0.0, 0.0, 0.0);
        let mut last = Instant::now();
        let started = Instant::now();
        while win.pump() {
            let now = Instant::now();
            let dt = if present_n.is_some() { FIXED_DT } else { now.duration_since(last).as_secs_f32() };
            last = now;
            angle += dt * TURN_RATE;
            cam.fly(&win.fly_input(), dt);
            world.render(&mut renderer, &mut fb, &cam, angle);
            let blit = Instant::now();
            if let Err(e) = win.present(&fb) {
                fail(&e, 1);
            }
            blit_ms += blit.elapsed().as_secs_f64() * 1000.0;
            let s = renderer.stats();
            shadow_ms += s.shadow_ms;
            main_ms += s.main_ms;
            presented += 1;
            if present_n == Some(presented) {
                let n = presented as f64;
                let total = started.elapsed().as_secs_f64() * 1000.0 / n;
                println!("car: presented {presented} frames at {}x{}", args.width, args.height);
                println!(
                    "car: average {total:.3} ms per frame = shadow pass {:.3} + main pass {:.3} + blit {:.3} + other {:.3}",
                    shadow_ms / n,
                    main_ms / n,
                    blit_ms / n,
                    total - (shadow_ms + main_ms + blit_ms) / n
                );
                return; // dropping the window destroys it
            }
            if present_n.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        if present_n.is_some() {
            fail("the window closed before all frames were presented", 1);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&mut cam, &mut fb);
        fail("the window viewer is Windows-only; use --frames N", 1);
    }
}

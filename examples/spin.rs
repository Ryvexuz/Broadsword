//! A spinning wireframe cube with fly controls.
//!
//!   spin                      opens a window (W/S/A/D move, E/Q up/down, arrows turn and look, Esc quits)
//!   spin --frames N           renders N frames with no window, prints the average frame time, exits 0
//! Options: --width W --height H (default 640x480).

use render::{draw_line_3d, wire_cube, Camera, FlyInput, Framebuffer, Vec3};
use std::time::Instant;

const BACKGROUND: u32 = 0x0010_1018;
const CUBE_COLOR: u32 = 0x00FF_C040;
const GRID_COLOR: u32 = 0x0040_4860;

struct Args {
    frames: Option<usize>,
    width: usize,
    height: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { frames: None, width: 640, height: 480 };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |name: &str| -> Result<usize, String> {
            it.next()
                .ok_or_else(|| format!("{name} needs a value"))?
                .parse()
                .map_err(|_| format!("{name} needs a whole number"))
        };
        match flag.as_str() {
            "--frames" => a.frames = Some(value("--frames")?),
            "--width" => a.width = value("--width")?,
            "--height" => a.height = value("--height")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn draw_scene(fb: &mut Framebuffer, cam: &Camera, angle: f32) {
    fb.clear(BACKGROUND);
    // A ground grid of lines, so motion is easy to see.
    for i in -5..=5 {
        let t = i as f32;
        draw_line_3d(fb, cam, Vec3::new(t, 0.0, -5.0), Vec3::new(t, 0.0, 5.0), GRID_COLOR);
        draw_line_3d(fb, cam, Vec3::new(-5.0, 0.0, t), Vec3::new(5.0, 0.0, t), GRID_COLOR);
    }
    for (a, b) in wire_cube(Vec3::new(0.0, 1.0, 0.0), 2.0, angle) {
        draw_line_3d(fb, cam, a, b, CUBE_COLOR);
    }
}

fn start_camera(aspect: f32) -> Camera {
    Camera {
        position: Vec3::new(0.0, 2.5, 6.0),
        yaw: 0.0,
        pitch: -0.3,
        fov_y: 60f32.to_radians(),
        aspect,
        near: 0.1,
        far: 100.0,
        move_speed: 4.0,
        turn_speed: 1.5,
    }
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("spin: {e}");
            std::process::exit(2);
        }
    };
    let mut fb = match Framebuffer::new(args.width, args.height) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("spin: {e}");
            std::process::exit(2);
        }
    };
    let cam = start_camera(args.width as f32 / args.height as f32);

    if let Some(frames) = args.frames {
        if frames == 0 {
            eprintln!("spin: --frames must be at least 1");
            std::process::exit(2);
        }
        let start = Instant::now();
        for i in 0..frames {
            draw_scene(&mut fb, &cam, i as f32 * 0.02);
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("spin: {frames} frames at {}x{}, average frame time {ms:.3} ms", args.width, args.height);
        return;
    }

    #[cfg(windows)]
    {
        use render::window::{Placement, Window};
        let mut cam = cam;
        let mut win = match Window::open("Broadsword render: spin", args.width, args.height, Placement::Normal) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("spin: {e}");
                std::process::exit(1);
            }
        };
        let mut angle = 0.0f32;
        let mut last = Instant::now();
        while win.pump() {
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f32();
            last = now;
            angle += dt;
            let input: FlyInput = win.fly_input();
            cam.fly(&input, dt);
            draw_scene(&mut fb, &cam, angle);
            if let Err(e) = win.present(&fb) {
                eprintln!("spin: {e}");
                std::process::exit(1);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&cam, FlyInput::default());
        eprintln!("spin: the window viewer is Windows-only; use --frames N");
        std::process::exit(1);
    }
}

//! Replays a crash-sim trajectory (CSV from crash-sim's `examples/trajectory.rs`) as a wireframe cube.
//!
//!   replay FILE.csv                 opens a window and plays the file in real time, using its own `t` column.
//!                                   Space pauses, R restarts; W/S/A/D, E/Q and the arrows fly the camera; Esc quits.
//!   replay FILE.csv --frames N      renders N frames with no window, prints the average frame time, exits 0
//!   replay FILE.csv --present N     opens the window off screen without activation, presents N frames through
//!                                   the real pump / fly_input / present path, closes it and exits 0
//! Options: --width W --height H (default 640x480).
//!
//! After the last frame the replay holds the last frame until R is pressed.

use render::replay::Frame;
use render::{draw_frame, Camera, Framebuffer, Trajectory, Vec3};
use std::time::Instant;

/// Time step for the modes that do not follow the clock (`--frames`, `--present`): one 60 Hz frame.
const FIXED_DT: f32 = 1.0 / 60.0;

struct Args {
    path: String,
    frames: Option<usize>,
    present: Option<usize>,
    width: usize,
    height: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { path: String::new(), frames: None, present: None, width: 640, height: 480 };
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
            other if other.starts_with("--") => return Err(format!("unknown argument {other}")),
            other if a.path.is_empty() => a.path = other.to_string(),
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    if a.path.is_empty() {
        return Err("usage: replay FILE.csv [--frames N | --present N] [--width W] [--height H]".to_string());
    }
    if a.frames.is_some() && a.present.is_some() {
        return Err("--frames and --present cannot be used together".to_string());
    }
    Ok(a)
}

/// Looks at the cube from the side and above; everything the starter cube does is in view.
fn start_camera(aspect: f32) -> Camera {
    Camera {
        position: Vec3::new(3.5, 2.2, 5.5),
        yaw: -0.567,
        pitch: -0.15,
        fov_y: 60f32.to_radians(),
        aspect,
        near: 0.1,
        far: 100.0,
        move_speed: 4.0,
        turn_speed: 1.5,
    }
}

fn fail(msg: &str, code: i32) -> ! {
    eprintln!("replay: {msg}");
    std::process::exit(code);
}

fn frame_to_draw(traj: &Trajectory, t: f32) -> &Frame {
    &traj.frames()[traj.frame_index_at(t)]
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| fail(&e, 2));
    let text = std::fs::read_to_string(&args.path).unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", args.path), 2));
    let traj = Trajectory::parse(&text).unwrap_or_else(|e| fail(&format!("{}: {e}", args.path), 2));
    let mut fb = Framebuffer::new(args.width, args.height).unwrap_or_else(|e| fail(&e.to_string(), 2));
    let mut cam = start_camera(args.width as f32 / args.height as f32);

    if let Some(frames) = args.frames {
        if frames == 0 {
            fail("--frames must be at least 1", 2);
        }
        let start = Instant::now();
        for i in 0..frames {
            draw_frame(&mut fb, &cam, &frame_to_draw(&traj, i as f32 * FIXED_DT).1);
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!(
            "replay: {frames} frames of {} ({} trajectory frames) at {}x{}, average frame time {ms:.3} ms",
            args.path,
            traj.len(),
            args.width,
            args.height
        );
        return;
    }

    #[cfg(windows)]
    {
        use render::window::{Placement, Window, VK_SPACE};
        let present_n = args.present;
        if present_n == Some(0) {
            fail("--present must be at least 1", 2);
        }
        let placement = if present_n.is_some() { Placement::OffScreen } else { Placement::Normal };
        let mut win = Window::open("Broadsword render: replay", args.width, args.height, placement)
            .unwrap_or_else(|e| fail(&e, 1));
        let (mut t, mut paused) = (0.0f32, false);
        let (mut space_was, mut r_was) = (false, false);
        let mut last = Instant::now();
        let mut presented = 0usize;
        while win.pump() {
            let now = Instant::now();
            // The modes that stop by themselves step a fixed time so they do not depend on the clock.
            let dt = if present_n.is_some() { FIXED_DT } else { now.duration_since(last).as_secs_f32() };
            last = now;
            let (space, r) = (win.key_down(VK_SPACE), win.key_down(b'R' as i32));
            if space && !space_was {
                paused = !paused;
            }
            if r && !r_was {
                t = 0.0;
            }
            (space_was, r_was) = (space, r);
            if !paused {
                t += dt;
            }
            cam.fly(&win.fly_input(), dt);
            draw_frame(&mut fb, &cam, &frame_to_draw(&traj, t).1);
            if let Err(e) = win.present(&fb) {
                fail(&e, 1);
            }
            presented += 1;
            if present_n == Some(presented) {
                println!("replay: presented {presented} frames");
                return; // dropping the window destroys it
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
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

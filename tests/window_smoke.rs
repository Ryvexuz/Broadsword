//! Opens the Win32 viewer off every monitor and without activation (it cannot take focus or be seen),
//! presents 3 frames and closes it. Windows only.
#![cfg(windows)]

use render::window::{Placement, Window};
use render::Framebuffer;

#[test]
fn window_smoke_opens_presents_three_frames_and_closes() {
    let mut fb = Framebuffer::new(64, 48).unwrap();
    let mut win = Window::open("render smoke test", 64, 48, Placement::OffScreen).expect("window opens");
    for i in 0..3u32 {
        fb.clear(0x0010_1018 + i * 0x0010_0000);
        fb.line(0, 0, 63, 47, 0x00FF_FFFF);
        assert!(win.pump(), "window should stay open (frame {i})");
        win.present(&fb).expect("present");
    }
    // A mismatched framebuffer is refused, not blitted.
    let wrong = Framebuffer::new(10, 10).unwrap();
    assert!(win.present(&wrong).is_err());
    drop(win); // destroys the window
}

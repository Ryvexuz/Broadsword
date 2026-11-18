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

/// A window the user has minimised has nothing to draw into; presenting to it is skipped, not an error.
/// `Minimised` opens it that way without activation, so it cannot take focus.
#[test]
fn presenting_to_a_minimised_window_is_not_an_error() {
    let fb = Framebuffer::new(64, 48).unwrap();
    let mut win = Window::open("render minimised smoke test", 64, 48, Placement::Minimised).expect("window opens");
    for i in 0..3 {
        assert!(win.pump(), "window should stay open (frame {i})");
        win.present(&fb).expect("present to a minimised window is skipped, not an error");
    }
    // A mismatched framebuffer is still refused: the size check comes before the minimised skip.
    let wrong = Framebuffer::new(10, 10).unwrap();
    assert!(win.present(&wrong).is_err());
}

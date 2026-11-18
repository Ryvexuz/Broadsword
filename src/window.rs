//! A minimal Win32 viewer: shows a `Framebuffer` in a window. Win32 is declared by hand, no crates.
//! The window is only a viewer; the framebuffer is what tests check.

use crate::camera::FlyInput;
use crate::framebuffer::Framebuffer;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

type Hwnd = isize;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: isize,
}

#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Default)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels: i32,
    y_pels: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(class: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: isize,
        instance: isize,
        param: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn GetDC(hwnd: Hwnd) -> isize;
    fn ReleaseDC(hwnd: Hwnd, hdc: isize) -> i32;
    fn AdjustWindowRect(rect: *mut Rect, style: u32, menu: i32) -> i32;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetForegroundWindow() -> Hwnd;
}

#[link(name = "gdi32")]
extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn StretchDIBits(
        hdc: isize,
        x_dest: i32,
        y_dest: i32,
        w_dest: i32,
        h_dest: i32,
        x_src: i32,
        y_src: i32,
        w_src: i32,
        h_src: i32,
        bits: *const c_void,
        info: *const BitmapInfo,
        usage: u32,
        rop: u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
}

const WM_CLOSE: u32 = 0x0010;
const PM_REMOVE: u32 = 1;
const GWLP_USERDATA: i32 = -21;
const SW_SHOWNOACTIVATE: i32 = 4;
const SW_SHOWMINNOACTIVE: i32 = 7;
const SW_SHOWNORMAL: i32 = 1;
const WS_OVERLAPPED: u32 = 0;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const ERROR_CLASS_ALREADY_EXISTS: i32 = 1410;
const SRCCOPY: u32 = 0x00CC_0020;
const OFFSCREEN: i32 = -32000;

pub const VK_ESCAPE: i32 = 0x1B;
pub const VK_LEFT: i32 = 0x25;
pub const VK_UP: i32 = 0x26;
pub const VK_RIGHT: i32 = 0x27;
pub const VK_DOWN: i32 = 0x28;

/// How the window first appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Shown normally (takes focus, as any app window does).
    Normal,
    /// Shown far off every monitor and without activation: never steals focus and cannot be seen. For tests.
    OffScreen,
    /// Shown minimised without activation: never steals focus.
    Minimised,
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    if msg == WM_CLOSE {
        // Do not destroy here: the `Window` owns the handle. Just record the request.
        let flag = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const AtomicBool;
        if !flag.is_null() {
            (*flag).store(true, Ordering::SeqCst);
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Window {
    hwnd: Hwnd,
    width: usize,
    height: usize,
    close_requested: Box<AtomicBool>,
}

impl Window {
    /// Opens a window whose client area is `width` x `height` pixels.
    pub fn open(title: &str, width: usize, height: usize, placement: Placement) -> Result<Window, String> {
        if width == 0 || height == 0 || width > i32::MAX as usize || height > i32::MAX as usize {
            return Err(format!("window size {width}x{height} is invalid"));
        }
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class_name = wide("BroadswordRenderWindow");
            let class = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                wnd_proc: Some(wnd_proc),
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: 0,
                cursor: 0,
                background: 0,
                menu_name: std::ptr::null(),
                class_name: class_name.as_ptr(),
                icon_sm: 0,
            };
            if RegisterClassExW(&class) == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(ERROR_CLASS_ALREADY_EXISTS) {
                    return Err(format!("RegisterClassExW failed: {err}"));
                }
            }
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let mut rect = Rect { left: 0, top: 0, right: width as i32, bottom: height as i32 };
            AdjustWindowRect(&mut rect, style, 0);
            let (x, y) = match placement {
                Placement::OffScreen => (OFFSCREEN, OFFSCREEN),
                // CW_USEDEFAULT
                _ => (i32::MIN, i32::MIN),
            };
            let title_w = wide(title);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                style,
                x,
                y,
                rect.right - rect.left,
                rect.bottom - rect.top,
                0,
                0,
                instance,
                std::ptr::null_mut(),
            );
            if hwnd == 0 {
                return Err(format!("CreateWindowExW failed: {}", std::io::Error::last_os_error()));
            }
            let close_requested = Box::new(AtomicBool::new(false));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*close_requested as *const AtomicBool as isize);
            ShowWindow(
                hwnd,
                match placement {
                    Placement::Normal => SW_SHOWNORMAL,
                    Placement::OffScreen => SW_SHOWNOACTIVATE,
                    Placement::Minimised => SW_SHOWMINNOACTIVE,
                },
            );
            Ok(Window { hwnd, width, height, close_requested })
        }
    }

    /// Handles pending window messages. Returns false once the window was closed (or Escape pressed).
    pub fn pump(&mut self) -> bool {
        unsafe {
            let mut msg = Msg::default();
            while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        !(self.close_requested.load(Ordering::SeqCst) || self.key_down(VK_ESCAPE))
    }

    /// True while the key (a Win32 virtual-key code, or the ASCII code of a capital letter) is held and
    /// this window is the foreground window, so other programs' typing is ignored.
    pub fn key_down(&self, vk: i32) -> bool {
        unsafe { GetForegroundWindow() == self.hwnd && (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
    }

    /// The fly controls: W/S forward/back, A/D left/right, E/Q up/down, arrows turn and look.
    pub fn fly_input(&self) -> FlyInput {
        FlyInput {
            forward: self.key_down(b'W' as i32),
            back: self.key_down(b'S' as i32),
            left: self.key_down(b'A' as i32),
            right: self.key_down(b'D' as i32),
            up: self.key_down(b'E' as i32),
            down: self.key_down(b'Q' as i32),
            turn_left: self.key_down(VK_LEFT),
            turn_right: self.key_down(VK_RIGHT),
            look_up: self.key_down(VK_UP),
            look_down: self.key_down(VK_DOWN),
        }
    }

    /// Copies the framebuffer into the window. The framebuffer must be the size the window was opened with.
    pub fn present(&mut self, fb: &Framebuffer) -> Result<(), String> {
        if fb.width() != self.width || fb.height() != self.height {
            return Err(format!(
                "framebuffer is {}x{} but the window is {}x{}",
                fb.width(),
                fb.height(),
                self.width,
                self.height
            ));
        }
        let info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width: self.width as i32,
                height: -(self.height as i32), // negative: top row first
                planes: 1,
                bit_count: 32,
                compression: 0, // BI_RGB
                size_image: 0,
                x_pels: 0,
                y_pels: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        unsafe {
            let hdc = GetDC(self.hwnd);
            if hdc == 0 {
                return Err("GetDC failed".to_string());
            }
            let lines = StretchDIBits(
                hdc,
                0,
                0,
                self.width as i32,
                self.height as i32,
                0,
                0,
                self.width as i32,
                self.height as i32,
                fb.pixels().as_ptr() as *const c_void,
                &info,
                0, // DIB_RGB_COLORS
                SRCCOPY,
            );
            ReleaseDC(self.hwnd, hdc);
            if lines == 0 {
                return Err("StretchDIBits copied no scan lines".to_string());
            }
        }
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            // Detach the flag before it is freed, then destroy.
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
        }
    }
}

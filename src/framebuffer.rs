//! The in-memory framebuffer and 2D drawing.

use std::fmt;

/// Why a framebuffer or PPM could not be made.
#[derive(Debug, PartialEq, Eq)]
pub enum FramebufferError {
    /// Width or height is zero.
    ZeroSize { width: usize, height: usize },
    /// width * height does not fit in memory addressing.
    TooLarge { width: usize, height: usize },
    /// A PPM could not be parsed.
    BadPpm(String),
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FramebufferError::ZeroSize { width, height } => {
                write!(f, "framebuffer size {width}x{height} is invalid: both sides must be at least 1")
            }
            FramebufferError::TooLarge { width, height } => {
                write!(f, "framebuffer size {width}x{height} is too large")
            }
            FramebufferError::BadPpm(why) => write!(f, "bad PPM: {why}"),
        }
    }
}

impl std::error::Error for FramebufferError {}

/// Pixels are `0x00RRGGBB`, row 0 at the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Result<Framebuffer, FramebufferError> {
        if width == 0 || height == 0 {
            return Err(FramebufferError::ZeroSize { width, height });
        }
        let n = width
            .checked_mul(height)
            .ok_or(FramebufferError::TooLarge { width, height })?;
        Ok(Framebuffer { width, height, pixels: vec![0; n] })
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn clear(&mut self, color: u32) {
        self.pixels.fill(color & 0x00FF_FFFF);
    }

    /// Writes one pixel. Out-of-range coordinates are ignored.
    pub fn set(&mut self, x: i32, y: i32, color: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            self.pixels[y as usize * self.width + x as usize] = color & 0x00FF_FFFF;
        }
    }

    /// Reads one pixel, or `None` out of range.
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            Some(self.pixels[y as usize * self.width + x as usize])
        } else {
            None
        }
    }

    /// Bresenham line, both endpoints included. Exact integer maths; the pixels it covers do not depend on
    /// how much of the line is on screen. The part outside the buffer is never walked: the first and last
    /// on-screen steps are found directly, and the error term is computed in closed form for the first, so
    /// a line with huge coordinates costs only as much as its on-screen part.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u32) {
        let (x0, y0, x1, y1) = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
        let (w, h) = (self.width as i64, self.height as i64);
        let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1i64 } else { -1 }, if y0 < y1 { 1i64 } else { -1 });
        let color = color & 0x00FF_FFFF;
        if dx == 0 && dy == 0 {
            self.set_i64(x0, y0, color);
            return;
        }
        // The walk advances one pixel per step along the major axis (x if dx >= dy, else y). After k steps
        // the minor axis has advanced `minor_steps(k)` pixels; both are monotone in k.
        let x_major = dx >= dy;
        let (d_major, d_minor) = if x_major { (dx, dy) } else { (dy, dx) };
        let (a0, s_a, a_len) = if x_major { (x0, sx, w) } else { (y0, sy, h) };
        let (b0, s_b, b_len) = if x_major { (y0, sy, h) } else { (x0, sx, w) };
        // Steps k in 0..=d_major whose major coordinate is in the buffer.
        let (mut k_lo, mut k_hi) = if s_a > 0 { (-a0, a_len - 1 - a0) } else { (a0 - (a_len - 1), a0) };
        k_lo = k_lo.max(0);
        k_hi = k_hi.min(d_major);
        // Minor-axis advance that keeps the minor coordinate in the buffer.
        let (m_lo, m_hi) = if s_b > 0 { (-b0, b_len - 1 - b0) } else { (b0 - (b_len - 1), b0) };
        if k_lo > k_hi || m_hi < 0 || m_lo > d_minor {
            return;
        }
        // m(k) = floor((2*d_minor*k + d_major) / (2*d_major)): what the error-term walk below produces.
        let minor_steps = |k: i64| -> i64 { ((2 * d_minor as i128 * k as i128 + d_major as i128) / (2 * d_major as i128)) as i64 };
        // Smallest k in [k_lo, k_hi] with m(k) >= m_lo, and largest with m(k) <= m_hi (binary searches).
        if minor_steps(k_hi) < m_lo || minor_steps(k_lo) > m_hi {
            return;
        }
        let (mut lo, mut hi) = (k_lo, k_hi);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if minor_steps(mid) >= m_lo {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let k_first = lo;
        let (mut lo, mut hi) = (k_first, k_hi);
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            if minor_steps(mid) <= m_hi {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let k_last = lo;
        // State of the classic error-term walk (err = dx + dy with dy negative) after k_first steps.
        let m = minor_steps(k_first) as i128;
        let (dmaj, dmin, k) = (d_major as i128, d_minor as i128, k_first as i128);
        let mut err: i128 = if x_major { dmaj - dmin - dmin * k + dmaj * m } else { dmin - dmaj + dmin * k - dmaj * m };
        let (mut x, mut y) = if x_major {
            (x0 + sx * k_first, y0 + sy * m as i64)
        } else {
            (x0 + sx * m as i64, y0 + sy * k_first)
        };
        let (dx, dy) = (dx as i128, -(dy as i128));
        for step in 0..=(k_last - k_first) {
            self.set_i64(x, y, color);
            if step == k_last - k_first {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn set_i64(&mut self, x: i64, y: i64, color: u32) {
        if x >= 0 && y >= 0 && x < self.width as i64 && y < self.height as i64 {
            self.pixels[y as usize * self.width + x as usize] = color;
        }
    }

    /// A filled square of `size` x `size` pixels. For odd sizes (x, y) is the centre; for even sizes it is
    /// the pixel just below and right of the centre. Size 0 draws nothing.
    pub fn point(&mut self, x: i32, y: i32, size: u32, color: u32) {
        let size = size as i64;
        let start_x = x as i64 - size / 2;
        let start_y = y as i64 - size / 2;
        // Only walk the part that can be on screen.
        let x_lo = start_x.max(0);
        let x_hi = (start_x + size).min(self.width as i64);
        let y_lo = start_y.max(0);
        let y_hi = (start_y + size).min(self.height as i64);
        for py in y_lo..y_hi {
            for px in x_lo..x_hi {
                self.pixels[py as usize * self.width + px as usize] = color & 0x00FF_FFFF;
            }
        }
    }

    /// Binary PPM (P6), 8 bits per channel.
    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.reserve(self.pixels.len() * 3);
        for &p in &self.pixels {
            out.push((p >> 16) as u8);
            out.push((p >> 8) as u8);
            out.push(p as u8);
        }
        out
    }

    /// Reads back a P6 file written by `to_ppm` (no comments, maxval 255).
    pub fn from_ppm(bytes: &[u8]) -> Result<Framebuffer, FramebufferError> {
        let bad = |s: &str| FramebufferError::BadPpm(s.to_string());
        let mut pos = 0usize;
        let token = |pos: &mut usize| -> Option<String> {
            while *pos < bytes.len() && bytes[*pos].is_ascii_whitespace() {
                *pos += 1;
            }
            let start = *pos;
            while *pos < bytes.len() && !bytes[*pos].is_ascii_whitespace() {
                *pos += 1;
            }
            if start == *pos {
                None
            } else {
                Some(String::from_utf8_lossy(&bytes[start..*pos]).into_owned())
            }
        };
        if token(&mut pos).as_deref() != Some("P6") {
            return Err(bad("missing P6 magic"));
        }
        let w: usize = token(&mut pos).and_then(|t| t.parse().ok()).ok_or_else(|| bad("width"))?;
        let h: usize = token(&mut pos).and_then(|t| t.parse().ok()).ok_or_else(|| bad("height"))?;
        if token(&mut pos).as_deref() != Some("255") {
            return Err(bad("maxval must be 255"));
        }
        pos += 1; // the single whitespace byte after the header
        let mut fb = Framebuffer::new(w, h)?;
        let data = bytes.get(pos..).ok_or_else(|| bad("no pixel data"))?;
        if data.len() != w * h * 3 {
            return Err(bad("pixel data length does not match the header"));
        }
        for (i, c) in data.chunks_exact(3).enumerate() {
            fb.pixels[i] = (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
        }
        Ok(fb)
    }
}

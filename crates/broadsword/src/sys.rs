//! All OS calls live here so other platforms can be added later.
//! Windows implementation uses kernel32 directly (no external crates).

#[derive(Debug, Clone, Copy)]
pub struct Disk {
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Ram {
    pub avail: u64,
    pub total: u64,
}

#[cfg(windows)]
mod imp {
    use super::{Disk, Ram};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
        fn GlobalMemoryStatusEx(buf: *mut MemoryStatusEx) -> i32;
        // FILETIME is two u32 (low, high) = one little-endian u64.
        fn GetSystemTimes(idle: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    }

    pub fn disk(path: &Path) -> Result<Disk, String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: wide is NUL-terminated and the out pointers are valid u64s.
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok == 0 {
            return Err(format!("GetDiskFreeSpaceExW failed for {}", path.display()));
        }
        Ok(Disk { free: avail, total })
    }

    pub fn ram() -> Result<Ram, String> {
        let mut s = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: s is a properly sized, initialised MEMORYSTATUSEX.
        if unsafe { GlobalMemoryStatusEx(&mut s) } == 0 {
            return Err("GlobalMemoryStatusEx failed".into());
        }
        Ok(Ram { avail: s.avail_phys, total: s.total_phys })
    }

    /// (idle, busy) cumulative 100ns ticks. Kernel time includes idle time.
    pub fn cpu_times() -> Result<(u64, u64), String> {
        let (mut idle, mut kernel, mut user) = (0u64, 0u64, 0u64);
        // SAFETY: out pointers are valid u64s with FILETIME layout.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return Err("GetSystemTimes failed".into());
        }
        Ok((idle, kernel + user - idle))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Disk, Ram};
    use std::path::Path;
    pub fn disk(_: &Path) -> Result<Disk, String> {
        Err("disk query not implemented on this platform".into())
    }
    pub fn ram() -> Result<Ram, String> {
        Err("RAM query not implemented on this platform".into())
    }
    pub fn cpu_times() -> Result<(u64, u64), String> {
        Err("CPU query not implemented on this platform".into())
    }
}

pub use imp::{cpu_times, disk, ram};

/// CPU use in percent over `sample` (blocks for that long).
pub fn cpu_percent(sample: std::time::Duration) -> Result<f64, String> {
    let (i0, b0) = cpu_times()?;
    std::thread::sleep(sample);
    let (i1, b1) = cpu_times()?;
    Ok(cpu_percent_from(i1.saturating_sub(i0), b1.saturating_sub(b0)))
}

pub fn cpu_percent_from(idle_delta: u64, busy_delta: u64) -> f64 {
    let total = idle_delta + busy_delta;
    if total == 0 {
        0.0
    } else {
        busy_delta as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_math() {
        assert_eq!(cpu_percent_from(0, 0), 0.0);
        assert_eq!(cpu_percent_from(75, 25), 25.0);
    }

    #[cfg(windows)]
    #[test]
    fn real_calls_return_sane_values() {
        let d = disk(std::path::Path::new(".")).unwrap();
        assert!(d.total > 0 && d.free <= d.total);
        let r = ram().unwrap();
        assert!(r.total > 0 && r.avail <= r.total);
    }
}

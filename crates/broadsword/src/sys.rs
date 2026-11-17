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

/// What the OS says about a process id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    /// No such process (never existed, or it has exited).
    Gone,
    /// Running. `start` is its creation time (100ns ticks since 1601) when it could be read.
    Alive { start: Option<u64> },
}

#[cfg(windows)]
mod imp {
    use super::{Disk, ProcState, Ram};
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
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(h: isize) -> i32;
        fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
        fn GetProcessTimes(h: isize, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
        fn GetLastError() -> u32;
    }

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: u32 = 5;

    pub fn process_state(pid: u32) -> ProcState {
        // SAFETY: plain handle calls; the handle is closed before returning and the out pointers are valid.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h == 0 {
                // A process we may not open still exists: never call it dead on a guess.
                return if GetLastError() == ERROR_ACCESS_DENIED { ProcState::Alive { start: None } } else { ProcState::Gone };
            }
            let mut code = 0u32;
            let state = if GetExitCodeProcess(h, &mut code) != 0 && code != STILL_ACTIVE {
                ProcState::Gone
            } else {
                let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
                let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
                ProcState::Alive { start: ok.then_some(c) }
            };
            CloseHandle(h);
            state
        }
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
    use super::{Disk, ProcState, Ram};
    use std::path::Path;
    pub fn process_state(_: u32) -> ProcState {
        ProcState::Alive { start: None } // not implemented here: never claim a process is gone
    }
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

pub use imp::{cpu_times, disk, process_state, ram};

/// Does a recorded (pid, creation time) still name the same running process? A pid that now belongs
/// to a different process (pid reuse) has a different creation time and counts as gone.
pub fn same_process(recorded_start: Option<u64>, actual: ProcState) -> bool {
    match (recorded_start, actual) {
        (_, ProcState::Gone) => false,
        (Some(r), ProcState::Alive { start: Some(a) }) => r == a,
        _ => true,
    }
}

/// "Is it still alive?" Lenient: an unreadable creation time counts as the same process. Never use this to kill.
pub fn is_alive(pid: u32, recorded_start: Option<u64>) -> bool {
    same_process(recorded_start, process_state(pid))
}

/// Strict identity for killing: only a recorded creation time that equals the live process's counts.
pub fn exact_match(recorded_start: Option<u64>, actual: ProcState) -> bool {
    matches!((recorded_start, actual), (Some(r), ProcState::Alive { start: Some(a) }) if r == a)
}

/// Creation time of a live process, if readable.
pub fn start_time(pid: u32) -> Option<u64> {
    match process_state(pid) {
        ProcState::Alive { start } => start,
        ProcState::Gone => None,
    }
}

/// Kill a process and everything it started.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
}

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

    #[test]
    fn pid_reuse_guard() {
        let alive = |s| ProcState::Alive { start: s };
        assert!(same_process(Some(5), alive(Some(5))));
        assert!(!same_process(Some(5), alive(Some(6)))); // same pid, different process
        assert!(!same_process(Some(5), ProcState::Gone));
        assert!(same_process(None, alive(Some(6)))); // nothing recorded: trust the pid
        assert!(same_process(Some(5), alive(None))); // start unreadable: do not call it dead
        // killing needs an exact match
        assert!(exact_match(Some(5), alive(Some(5))));
        assert!(!exact_match(Some(5), alive(Some(6))));
        assert!(!exact_match(Some(5), alive(None)));
        assert!(!exact_match(None, alive(Some(5))));
        assert!(!exact_match(Some(5), ProcState::Gone));
    }

    #[cfg(windows)]
    #[test]
    fn liveness_of_real_processes() {
        let me = std::process::id();
        let start = start_time(me).expect("own start time");
        assert!(is_alive(me, Some(start)));
        assert!(!is_alive(me, Some(start + 1)));
        let mut c = std::process::Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        let pid = c.id();
        c.wait().unwrap();
        assert!(!is_alive(pid, None));
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

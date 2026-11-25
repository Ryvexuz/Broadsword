/// Kill the job first: taskkill blocks for seconds under load, but the job kill is one kernel call.
pub(super) fn everything(
    group: &super::group::Group,
    child: &mut std::process::Child,
    tree_kill: impl FnOnce(u32),
) {
    group.kill();
    tree_kill(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::group::{self, Group};
    use super::everything;
    use crate::config::Config;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    fn running_grandchild(test: &str) -> (Group, Child, PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = Config::load_from(&root).unwrap();
        let scratch = root.join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let dir = scratch.join(format!("dc-unit-kill-{test}-{}", std::process::id()));
        std::fs::create_dir(&dir).expect("a fresh test directory");
        let inner = format!(
            "echo up> {} & ping -n 3 127.0.0.1 >nul & echo late> {}",
            dir.join("up.txt").display(),
            dir.join("late.txt").display()
        );
        let mut cmd = crate::scrub::command(&cfg, "cmd.exe");
        cmd.args(["/C", "start", "/b", "cmd", "/C", &inner])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        group::prepare(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let g = Group::start(&child, &cfg).unwrap_or_else(|e| {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cannot start the kill group: {e}");
        });
        let start = Instant::now();
        while !dir.join("up.txt").exists() {
            assert!(start.elapsed() < Duration::from_secs(20), "grandchild did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        (g, child, dir)
    }

    fn cleanup(dir: &Path) {
        std::fs::remove_file(dir.join("up.txt")).unwrap();
        if dir.join("late.txt").exists() {
            std::fs::remove_file(dir.join("late.txt")).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn the_job_is_killed_before_a_slow_tree_kill() {
        let (g, mut child, dir) = running_grandchild("slow");
        everything(&g, &mut child, |_| std::thread::sleep(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_secs(3));
        let late = dir.join("late.txt").exists();
        cleanup(&dir);
        assert!(!late, "the grandchild ran on during the slow tree kill");
    }

    #[test]
    fn the_tree_kill_still_runs_with_the_child_pid() {
        let (g, mut child, dir) = running_grandchild("pid");
        let pid = child.id();
        let seen = Cell::new(None);
        everything(&g, &mut child, |pid| seen.set(Some(pid)));
        let waited = child.try_wait();
        cleanup(&dir);
        assert_eq!(seen.get(), Some(pid));
        assert!(matches!(waited, Ok(Some(_))), "child was not reaped: {waited:?}");
    }
}

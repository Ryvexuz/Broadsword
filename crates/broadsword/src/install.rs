//! `broadsword install`: copy the running binary to a stable place (`install_dir`), so the MCP server and the
//! pause hook run a file that a rebuild never has to overwrite.
//!
//! On Windows a running .exe cannot be overwritten or deleted, but it CAN be renamed. So the new copy is
//! written next to the target under a temporary name and then renamed into place (atomic: nobody ever sees a
//! half-written binary). If the target is locked, the old file is renamed aside first
//! (`broadsword.old-<stamp>.exe`) and the new one moved in. Old copies are deleted on later installs, once
//! nothing runs them any more.

use crate::config::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const STEM: &str = "broadsword";

fn exe_name() -> String {
    format!("{STEM}{}", std::env::consts::EXE_SUFFIX)
}

fn aside_prefix() -> String {
    format!("{STEM}.old-")
}

fn temp_prefix() -> String {
    format!("{STEM}.new-")
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub target: PathBuf,
    pub bytes: u64,
    /// The old copy was in use and was renamed to this file name.
    pub moved_aside: Option<String>,
    pub removed: Vec<String>,
    pub still_in_use: Vec<String>,
    /// The source already is the installed file: nothing was done.
    pub already_installed: bool,
}

/// Names of leftovers of earlier installs in `dir`: renamed-aside old copies and abandoned temp files.
fn leftovers(dir: &Path) -> Vec<String> {
    let (old, tmp) = (aside_prefix(), temp_prefix());
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with(&old) || n.starts_with(&tmp)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Install `src` into `dir` as `broadsword[.exe]`. `stamp` makes the temporary and renamed-aside names unique.
pub fn install(src: &Path, dir: &Path, stamp: &str) -> Result<Report, String> {
    let target = dir.join(exe_name());
    let mut report = Report { target: target.clone(), ..Report::default() };
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    if let (Ok(a), Ok(b)) = (src.canonicalize(), target.canonicalize()) {
        if a == b {
            report.already_installed = true;
            return Ok(report);
        }
    }
    // Earlier leftovers first: whatever is no longer locked goes now. (The file renamed aside below is by
    // definition still locked, so it is left for the next install.)
    for name in leftovers(dir) {
        match fs::remove_file(dir.join(&name)) {
            Ok(()) => report.removed.push(name),
            Err(_) => report.still_in_use.push(name),
        }
    }
    let temp = dir.join(format!("{}{stamp}{}", temp_prefix(), std::env::consts::EXE_SUFFIX));
    report.bytes = fs::copy(src, &temp).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("cannot copy {} to {}: {e}", src.display(), temp.display())
    })?;
    if fs::rename(&temp, &target).is_err() {
        // Locked (or otherwise not replaceable): move the old file out of the way, then the new one in.
        let aside_name = format!("{}{stamp}{}", aside_prefix(), std::env::consts::EXE_SUFFIX);
        let aside = dir.join(&aside_name);
        let moved = target.exists() && fs::rename(&target, &aside).is_ok();
        if let Err(e) = fs::rename(&temp, &target) {
            if moved {
                let _ = fs::rename(&aside, &target); // put the old binary back: never leave the install empty
            }
            let _ = fs::remove_file(&temp);
            return Err(format!("cannot move the new binary into place at {}: {e}", target.display()));
        }
        if moved {
            report.moved_aside = Some(aside_name);
        }
    }
    Ok(report)
}

pub fn print_report(r: &Report) {
    if r.already_installed {
        println!("already installed: this process is {}; nothing to do (run `install` from the freshly built binary to update it)", r.target.display());
        return;
    }
    println!("installed: {} ({} bytes)", r.target.display(), r.bytes);
    if let Some(n) = &r.moved_aside {
        println!("the old copy is in use, so it was renamed to {n}; it is deleted by a later install once nothing runs it");
    }
    if !r.removed.is_empty() {
        println!("removed old copies: {}", r.removed.join(", "));
    }
    if !r.still_in_use.is_empty() {
        println!("old copies still in use (left in place): {}", r.still_in_use.join(", "));
    }
}

pub fn cmd_install(cfg: &Config) -> Result<ExitCode, String> {
    let src = std::env::current_exe().map_err(|e| format!("cannot find the running binary: {e}"))?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| (d.as_secs(), d.subsec_millis())).unwrap_or((0, 0));
    let stamp = format!("{}{:03}-{}", crate::job::file_stamp(secs.0), secs.1, std::process::id());
    let report = install(&src, &cfg.install_dir, &stamp)?;
    print_report(&report);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("inst-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn source(d: &Dir, content: &str) -> PathBuf {
        let p = d.0.join(format!("src-bin{}", std::env::consts::EXE_SUFFIX));
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn installs_into_a_new_folder_and_replaces_an_unlocked_copy() {
        let d = Dir::new("new");
        let bin = d.0.join("deep/bin");
        let r = install(&source(&d, "one"), &bin, "s1").unwrap();
        assert_eq!((r.bytes, r.moved_aside, r.already_installed), (3, None, false));
        assert_eq!(fs::read_to_string(&r.target).unwrap(), "one");
        let r = install(&source(&d, "two!"), &bin, "s2").unwrap();
        assert_eq!(fs::read_to_string(&r.target).unwrap(), "two!");
        assert_eq!(r.moved_aside, None, "an unlocked target is replaced in place");
        assert_eq!(fs::read_dir(&bin).unwrap().count(), 1, "no temp file is left behind");
    }

    #[test]
    fn installing_the_installed_file_does_nothing() {
        let d = Dir::new("self");
        let bin = d.0.join("bin");
        let r = install(&source(&d, "x"), &bin, "s1").unwrap();
        let r2 = install(&r.target, &bin, "s2").unwrap();
        assert!(r2.already_installed);
        assert_eq!(fs::read_to_string(&r.target).unwrap(), "x");
        assert_eq!(fs::read_dir(&bin).unwrap().count(), 1);
    }

    #[test]
    fn leftovers_are_removed_and_unrelated_files_are_kept() {
        let d = Dir::new("left");
        let bin = d.0.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        for n in [format!("broadsword.old-1{suffix}"), format!("broadsword.new-2{suffix}"), "notes.txt".to_string()] {
            fs::write(bin.join(n), "x").unwrap();
        }
        let r = install(&source(&d, "n"), &bin, "s").unwrap();
        assert_eq!(r.removed.len(), 2, "{r:?}");
        assert!(bin.join("notes.txt").is_file());
        assert_eq!(fs::read_dir(&bin).unwrap().count(), 2);
    }

    /// A file held with sharing mode 0 cannot be renamed, overwritten or deleted: the closest thing to a running .exe
    /// that a unit test can make. (The real thing is tested in tests/install.rs with a running process.)
    #[cfg(windows)]
    fn lock(path: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn a_target_that_cannot_even_be_renamed_fails_cleanly() {
        // A truly running exe can be renamed but not replaced (tests/install.rs proves the rename-aside path with a
        // real process); share_mode(0) forbids even the rename, so this checks the failure path: the install fails
        // cleanly and the old binary and a clean folder remain.
        let d = Dir::new("lock");
        let bin = d.0.join("bin");
        let first = install(&source(&d, "old"), &bin, "s1").unwrap();
        let held = lock(&first.target);
        let err = install(&source(&d, "new"), &bin, "s2").unwrap_err();
        assert!(err.contains("cannot move the new binary into place"), "{err}");
        drop(held);
        assert_eq!(fs::read_to_string(&first.target).unwrap(), "old", "the old binary is untouched");
        assert_eq!(fs::read_dir(&bin).unwrap().count(), 1, "no temp file or aside file is left behind");
        let again = install(&source(&d, "new"), &bin, "s3").unwrap();
        assert_eq!(fs::read_to_string(&again.target).unwrap(), "new");
    }
}

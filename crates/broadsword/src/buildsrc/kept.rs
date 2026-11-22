use super::*;

// ---- main's binaries survive another folder's clean -----------------------------------------

/// THE stable place of main's binaries (people and other projects run these, never cargo's own profile folders): the KEPT copy of
/// each binary the main checkout built: `<build_dir>/main.bins/<project>/<profile dir>/<bin><exe suffix>`, with `<that
/// file>.source` holding the build-source note it was kept under. Cargo does not build there: a profile's folder carries the
/// profile's name, and cargo allows only letters, digits, `-` and `_` in one, so no profile maps to a name with a `.`; `cargo
/// clean -p` removes the named package's artifacts, which live in the profile folders. It stays inside the ONE shared build
/// folder: only final executables are copied, never dependencies.
pub const KEEP_DIR: &str = "main.bins";

/// THE one place the path of a kept copy is made: `bin` for `profile` of the project `project` (its name as job.rs knows it:
/// `broadsword` for the root, the folder name under `projects_dir` otherwise; see `kept_project`).
pub fn kept_exe(cfg: &Config, project: &str, profile: Profile, bin: &str) -> PathBuf {
    cfg.build_dir.join(KEEP_DIR).join(project).join(profile.dir()).join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
}

/// The project name whose kept copies belong to the main checkout `main` (an output of `main_folder`): `broadsword` for `.`,
/// `<p>` for `<projects>/<p>`. `None` (nothing is kept) for anything else, for a project folder named like Broadsword itself
/// (job.rs always builds that name from the root, so it would share the root's copies) and for a name that is not one plain
/// path part.
pub fn kept_project(cfg: &Config, main: &str) -> Option<String> {
    if main == MAIN_SOURCE {
        return Some(crate::job::SELF_PROJECT.to_string());
    }
    let projects = rel_source(cfg, &cfg.projects_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let p = main.strip_prefix(&format!("{projects}/"))?;
    let plain = !p.is_empty() && p != "." && p != ".." && !p.contains(['/', '\\', ':']) && !p.eq_ignore_ascii_case(crate::job::SELF_PROJECT);
    plain.then(|| p.to_string())
}

/// The note file beside a kept copy.
pub(super) fn kept_note(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".source");
    PathBuf::from(s)
}

/// The build-source note a kept copy of `bin` for `profile` of `project` was kept under (`projects/<p>`, `.`, or a `run:` note
/// of one of those), `None` if there is no kept copy or no (readable) note beside it.
pub fn read_kept_note(cfg: &Config, project: &str, profile: Profile, bin: &str) -> Option<String> {
    let exe = kept_exe(cfg, project, profile, bin);
    if !exe.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(kept_note(&exe)).ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// The cargo output path of `bin` for `profile` (cargo's cache: what the last build of any checkout left there).
pub(super) fn built_exe(cfg: &Config, profile: Profile, bin: &str) -> PathBuf {
    cfg.build_dir.join(profile.dir()).join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
}

/// The main checkout of the project that `source` belongs to: `.` for `.` and for a worktree of Broadsword itself
/// (`<worktrees>/broadsword/<id>`), `<projects>/<p>` for `<projects>/<p>` and for `<worktrees>/<p>/<id>`. `None` for any other
/// folder (nothing is kept for it). An exact allow-list of the two shapes job.rs builds sources from.
pub fn main_folder(cfg: &Config, source: &str) -> Option<String> {
    if source == MAIN_SOURCE {
        return Some(MAIN_SOURCE.to_string());
    }
    let projects = rel_source(cfg, &cfg.projects_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let worktrees = rel_source(cfg, &cfg.worktrees_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let one = |s: &str| !s.is_empty() && !s.contains('/') && s != "." && s != "..";
    if let Some(p) = source.strip_prefix(&format!("{projects}/")) {
        return one(p).then(|| source.to_string());
    }
    let (p, id) = source.strip_prefix(&format!("{worktrees}/"))?.split_once('/')?;
    if !one(p) || !one(id) {
        return None;
    }
    Some(if p == crate::job::SELF_PROJECT { MAIN_SOURCE.to_string() } else { format!("{projects}/{p}") })
}

/// Why `copy_atomic` failed: making the full copy beside `to` (disk full, a length mismatch, ...), or putting it in place of `to`
/// (the rename; on Windows a running or open exe cannot be replaced).
pub(super) enum CopyFail {
    Copy(std::io::Error),
    Replace(std::io::Error),
}

impl std::fmt::Display for CopyFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyFail::Copy(e) => write!(f, "{e}"),
            CopyFail::Replace(e) => write!(f, "cannot replace the old file: {e}"),
        }
    }
}

/// The temp file `copy_atomic` writes beside `to` before renaming it over `to`.
pub(super) fn copy_tmp(to: &Path) -> PathBuf {
    let mut tmp = to.as_os_str().to_os_string();
    tmp.push(".bskeep.tmp");
    PathBuf::from(tmp)
}

/// The temp file `write_atomic` writes beside `path` before renaming it over `path`.
pub(super) fn atomic_tmp(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// EVERY path this module creates, writes, renames or removes for the kept exe `exe`: the exe, its `.source` note
/// and `.stale` marker, and the temp file of each (`copy_tmp`, `atomic_tmp`). `guard_kept_files` checks exactly this list, and
/// `copy_atomic` and `write_atomic` build their temp names with the same helpers.
fn kept_files(exe: &Path) -> [PathBuf; 6] {
    let (side, stale) = (kept_note(exe), stale_marker(exe));
    [exe.to_path_buf(), copy_tmp(exe), atomic_tmp(&side), atomic_tmp(&stale), side, stale]
}

/// Create the temp file `tmp` as a NEW file: whatever entry already has that name (a leftover of a crash, a symlink, a hard link
/// to a file elsewhere) is removed first (removing a link removes the link, never its target), and the create fails if anything
/// appears there in between (`create_new`), so writing it never truncates or overwrites any other file.
pub(super) fn create_fresh(tmp: &Path) -> std::io::Result<std::fs::File> {
    match std::fs::remove_file(tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)
}

/// THE check before any file of a kept copy is read, written or removed: the
/// build-folder guard, then for the exe `exe` and every file written beside it (`.source`, `.stale`, the temp file): neither the
/// file nor any folder on its way (`main.bins`, the profile folder, up to the drive root) may be a symlink, junction or reparse
/// point, and a file or folder that exists must resolve inside `build_dir`. Err = refuse: nothing of it is touched.
pub(super) fn guard_kept_files(cfg: &Config, what: &str, exe: &Path) -> Result<(), String> {
    guard_build_dir(cfg, what)?;
    let build = cfg.build_dir.canonicalize().map_err(|e| format!("{what}: cannot resolve the build folder {}: {e}", cfg.build_dir.display()))?;
    for p in kept_files(exe) {
        if !p.starts_with(&cfg.build_dir) {
            return Err(format!("{what}: {} is not inside the build folder {}", p.display(), cfg.build_dir.display()));
        }
        if let Some(l) = link_in_path(&p) {
            return Err(format!("{what}: {} is a symlink, junction or reparse point (on the way to {}), so writing there could reach outside the build folder {}; nothing was written", l.display(), p.display(), cfg.build_dir.display()));
        }
        // the deepest part that exists must resolve inside the build folder
        if let Some(a) = p.ancestors().find(|a| std::fs::symlink_metadata(a).is_ok()) {
            let r = a.canonicalize().map_err(|e| format!("{what}: cannot resolve {}: {e}", a.display()))?;
            if !r.starts_with(&build) {
                return Err(format!("{what}: {} resolves to {}, outside the build folder {}; nothing was written", a.display(), r.display(), build.display()));
            }
        }
    }
    Ok(())
}

/// Copy `from` to `to` through a temp file beside `to` and a rename (a reader sees the old file or the new one). Cargo-built
/// executables only, inside the build folder. On any failure the temp file is removed and `to` is left as it was.
pub(super) fn copy_atomic(from: &Path, to: &Path) -> Result<(), CopyFail> {
    let tmp = copy_tmp(to);
    // flushing needs a handle with write access on Windows (FlushFileBuffers); the copy must be the whole file before it replaces anything
    let copied = (|| {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut out = create_fresh(&tmp)?;
        let n = std::io::copy(&mut std::fs::File::open(from)?, &mut out)?;
        let meta = std::fs::metadata(from)?;
        let want = meta.len();
        if n != want || out.metadata()?.len() != want {
            return Err(std::io::Error::other(format!("the copy has {n} bytes, the original {want}")));
        }
        // the copy keeps the build's time (readers such as render's tests pick the newest exe)
        out.set_modified(meta.modified()?)?;
        // ... and its permissions (the execute bits on Unix, the read-only flag on Windows): a kept exe must stay runnable
        let writable = out.metadata()?.permissions();
        out.set_permissions(meta.permissions())?;
        out.sync_all()?;
        Ok(writable)
    })();
    let r = copied.map_err(CopyFail::Copy).and_then(|writable| {
        // Windows refuses to replace a read-only file: the old copy was a read-only copy of an older build, so it is made
        // replaceable first (the plain permissions of a new file; elsewhere a rename never needs the old file to be writable).
        // When the rename then fails (the old copy is in use and stays), its read-only flag is set back: the old copy is left
        // exactly as it was.
        #[cfg(windows)]
        {
            let was_readonly = std::fs::metadata(to).ok().map(|m| m.permissions()).filter(|p| p.readonly());
            if was_readonly.is_some() {
                std::fs::set_permissions(to, writable).map_err(CopyFail::Replace)?;
            }
            std::fs::rename(&tmp, to).map_err(|e| match was_readonly.map(|p| std::fs::set_permissions(to, p)) {
                Some(Err(back)) => CopyFail::Replace(std::io::Error::new(e.kind(), format!("{e}; and setting the old file's read-only flag back failed: {back}"))),
                _ => CopyFail::Replace(e),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = writable;
            std::fs::rename(&tmp, to).map_err(CopyFail::Replace)
        }
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// True if `p` exists and another process has it open or is running it, so it cannot be replaced now: an exclusive open for
/// writing fails with a sharing or lock violation. Only Windows refuses to replace a running exe (elsewhere a rename always
/// replaces it), so elsewhere this is always false.
#[cfg(windows)]
pub(super) fn in_use(p: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
    matches!(std::fs::OpenOptions::new().write(true).share_mode(0).open(p), Err(e) if matches!(e.raw_os_error(), Some(32 | 33)))
}
#[cfg(not(windows))]
pub(super) fn in_use(_: &Path) -> bool {
    false
}

/// The marker beside a kept copy that could not be refreshed because it was in use: `<exe>.stale`, holding `stale: <note of the
/// newer build>, in use at <UTC>`. The `.source` note keeps naming the build the copy really is; the next refresh that can
/// replace the copy removes the marker.
pub fn stale_marker(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".stale");
    PathBuf::from(s)
}

/// When `keep_main_bins` runs, which decides what it may copy and what a kept copy in use (cannot be replaced) does to the job.
#[derive(Clone, Copy)]
pub(super) enum Keep<'a> {
    /// right after a SUCCESSFUL main step (`refresh_main_bins`), with what cargo's folder held before that step: only an exe the
    /// step itself produced (`StepBins::produced`), or one a stale marker vouches for, is copied. Any other exe there (cached
    /// from an earlier, maybe FAILED, step) keeps its current kept copy and note untouched. A copy in use is main's previous
    /// build and the new one is in cargo's folder, so the job passes, says so loudly and marks the copy stale (with the new
    /// exe's fingerprint)
    AfterMainStep(&'a StepBins),
    /// before another folder's clean: only the exe a stale marker vouches for (a successful build the refresh could not copy:
    /// `vouched_by_marker`) is copied; anything else in cargo's folder may be a failed build's and is left to the clean. A
    /// vouched copy in use refuses the job (the clean would delete main's newer build from cargo's folder with no copy of it;
    /// nothing built or cleaned)
    BeforeForeignClean,
}

/// What the `.stale` marker records of the exe in cargo's folder that a refresh could not copy: its length and modified time.
/// A later build of any checkout writes a new file there (another length or time), so a match means it is still that exe.
pub(super) fn fingerprint(exe: &Path) -> std::io::Result<String> {
    let m = std::fs::metadata(exe)?;
    let t = m.modified()?.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?;
    Ok(format!("{} bytes, modified {}.{:09}", m.len(), t.as_secs(), t.subsec_nanos()))
}

/// The second line of a `.stale` marker: `built: <fingerprint>`.
pub(super) const STALE_BUILT: &str = "built: ";

/// True if the `.stale` marker beside `kept` names exactly the exe `built` (its fingerprint on the marker's `built:` line): that
/// exe is a successful main build the refresh could not copy. False for no marker, an unreadable one, or any other exe.
pub(super) fn vouched_by_marker(kept: &Path, built: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(stale_marker(kept)) else { return false };
    let Ok(now) = fingerprint(built) else { return false };
    text.lines().any(|l| l.strip_prefix(STALE_BUILT) == Some(now.as_str()))
}


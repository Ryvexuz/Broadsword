use super::*;

/// What cargo's folder held for the bin exes of a MAIN checkout's packages just before one step (`snapshot_main_bins`): the
/// fingerprint (`fingerprint`: length and modified time) of each `<profile>/<bin>` exe, or none where there was no exe. After
/// the step, `produced` says which exes the step itself wrote, the only ones `refresh_main_bins` copies. THE one record both
/// the job's step loop and the test step's compile refresh from (job.rs takes it once per step, before the step runs).
pub struct StepBins {
    /// Ok: (profile, bin, fingerprint before the step). Err: the packages could not be listed before the step, so nothing can
    /// be told apart and nothing is refreshed (fail closed).
    pub(super) before: Result<Vec<(Profile, String, Option<String>)>, String>,
}

impl StepBins {
    /// True if the exe `exe` of `bin` for `profile` was written during the step: it exists now and its fingerprint differs from
    /// the one recorded before (or there was none: it appeared). False when the record could not be made.
    fn produced(&self, profile: Profile, bin: &str, exe: &Path) -> bool {
        let Ok(before) = &self.before else { return false };
        let Ok(now) = fingerprint(exe) else { return false };
        let was = before.iter().find(|(p, b, _)| *p == profile && b == bin).and_then(|(_, _, f)| f.as_deref());
        was != Some(now.as_str())
    }
}

/// The record of cargo's bin exes before a step of `dir` built from `source` (see `StepBins`). Call it with the build lock held,
/// after the build folder was prepared and right before the step runs. For a source that is not a main checkout it records
/// nothing (`refresh_main_bins` refreshes nothing for it anyway).
pub fn snapshot_main_bins(cfg: &Config, dir: &Path, source: &str) -> StepBins {
    if main_folder(cfg, source).as_deref() != Some(source) {
        return StepBins { before: Ok(Vec::new()) };
    }
    let before = bins_of(cfg, dir).map(|targets| {
        let mut v = Vec::new();
        for (_, bins) in &targets {
            for b in bins {
                for profile in CLEAN_PROFILES {
                    v.push((profile, b.clone(), fingerprint(&built_exe(cfg, profile, b)).ok()));
                }
            }
        }
        v
    });
    StepBins { before }
}

/// Copy main's exes of package `pkg` (whose binary targets are `bins`) to their kept place: every exe of those targets whose note
/// (the bin note, else the package note for builds from before bin notes) names `main` (plain or `run:`), in each profile, with
/// that note beside it. `when` ends the error message. Called after a successful main step (`refresh_main_bins`) and before a
/// job of ANOTHER folder cleans `pkg` (`prepare_packages`: nothing is cleaned until this succeeded; Err refuses the job, nothing
/// deleted); `moment` says which, and with it what may be copied (see `Keep`). The caller holds the build lock. An unchanged
/// copy is not rewritten (and gets no line). The note beside the copy is removed first and written last, so a crash in between
/// leaves a copy with no note.
pub(super) fn keep_main_bins(cfg: &Config, main: &str, pkg: &str, bins: &[String], when: &str, moment: Keep) -> Result<Vec<String>, String> {
    guard_build_dir(cfg, "refusing to keep main's binaries")?;
    let Some(project) = kept_project(cfg, main) else { return Ok(Vec::new()) };
    let mut lines = Vec::new();
    for b in bins {
        let Some(note) = read_bin_note(cfg, b).or_else(|| read_note(cfg, pkg)) else { continue };
        if note_folder(&note) != main {
            continue;
        }
        for profile in CLEAN_PROFILES {
            let (from, to) = (built_exe(cfg, profile, b), kept_exe(cfg, &project, profile, b));
            if !from.is_file() {
                continue;
            }
            guard_kept_files(cfg, "refusing to keep main's binaries", &to)?;
            // after a main step: an exe this step did not write is left alone, copy and note (it may be a failed step's),
            // unless a stale marker vouches for it (an earlier successful step whose refresh found the copy in use)
            if let Keep::AfterMainStep(step) = moment {
                if !step.produced(profile, b, &from) && !vouched_by_marker(&to, &from) {
                    if !to.is_file() {
                        lines.push(format!("main.bins: `{b}` ({}) has no kept copy, and this build did not write that exe (cargo had nothing to compile), so it was not copied; the next main build that compiles it fills main.bins", profile.dir()));
                    }
                    continue;
                }
            }
            let (side, stale) = (kept_note(&to), stale_marker(&to));
            let old = read_kept_note(cfg, &project, profile, b);
            // sameness is the BYTES only: a copy that is current byte for byte is never replaced because its note text differs
            // (a copy in use must not be refused or marked stale for that); only the note beside it is brought up to date
            let same = std::fs::read(&from).ok().is_some_and(|x| std::fs::read(&to).ok().as_deref() == Some(x.as_slice()));
            let fail = |e: &dyn std::fmt::Display| format!("cannot keep main's `{b}` ({}) at {} {when}: {e}\nIf a program is running the kept copy, stop it and repeat", profile.dir(), to.display());
            let remove = |f: &Path| match std::fs::remove_file(f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fail(&e)),
                _ => Ok(()),
            };
            if same {
                remove(&stale)?; // up to date after all: no longer stale
                // only a step that wrote this exe may rename its kept copy's builder; before another folder's clean the copy
                // and its note stay exactly as the last main step left them
                if matches!(moment, Keep::AfterMainStep(_)) && old.as_deref() != Some(note.as_str()) {
                    // the binary is not touched (it may be running); the note is replaced atomically beside it
                    write_atomic(&side, &note).map_err(|e| fail(&e))?;
                    lines.push(format!("main.bins: `{b}` ({}) is current; its note now says {} (the binary was not touched)", profile.dir(), note));
                }
                continue;
            }
            // before another folder's clean only a successful build the refresh could not copy is kept: the exe in cargo's folder
            // may be a failed or partial main build's (prepare set its bin note to main before that build ran)
            if matches!(moment, Keep::BeforeForeignClean) && !vouched_by_marker(&to, &from) {
                lines.push(format!("main.bins: `{b}` ({}) not kept from {}: no successful main step vouches for that exe (it may be a failed build's); main.bins keeps the last good copy", profile.dir(), from.display()));
                continue;
            }
            // the note goes first and comes back last, so a crash in between leaves a copy with no note
            remove(&side)?;
            match copy_atomic(&from, &to) {
                Ok(()) => {
                    write_atomic(&side, &note).map_err(|e| fail(&e))?;
                    remove(&stale)?;
                    lines.push(format!("kept main's `{b}` ({}, built from {}) at {}", profile.dir(), note_folder(&note), to.display()));
                }
                Err(CopyFail::Replace(e)) if matches!(moment, Keep::AfterMainStep(_)) && in_use(&to) => {
                    // the old copy stays, still named by its own note; the marker says a newer build exists, and which exe it is
                    if let Some(o) = &old {
                        write_atomic(&side, o).map_err(|e| fail(&e))?;
                    }
                    let built = fingerprint(&from).map_err(|e| fail(&e))?;
                    write_atomic(&stale, &format!("stale: {note}, in use at {}\n{STALE_BUILT}{built}", crate::pause::now_string())).map_err(|e| fail(&e))?;
                    lines.push(format!(
                        "main.bins: `{b}` ({}) NOT refreshed (in use: {e}); the copy is from {}. The new build is only in {} for now; the next main build refreshes the copy once nothing runs it ({} says so meanwhile)",
                        profile.dir(),
                        old.as_deref().unwrap_or("an unknown build (no note)"),
                        from.display(),
                        stale.display()
                    ));
                }
                Err(e) => {
                    if let Some(o) = &old {
                        let _ = write_atomic(&side, o); // the old copy is still there: name it again before failing
                    }
                    return Err(fail(&e));
                }
            }
        }
    }
    Ok(lines)
}

/// After a SUCCESSFUL cargo step (or `run` command) of the main checkout `source` itself, with the build lock still held: every
/// bin target exe of `dir`'s packages whose note names `source` AND that this step produced (`before`, taken by
/// `snapshot_main_bins` right before the step) is copied to its kept place (`keep_main_bins`), so `main.bins` holds main's newest
/// build of every bin a successful step built. An exe the step did not write (cached from an earlier step, which may have
/// failed) is not copied, and its kept copy and note stay as they are. Any other folder (a worktree, an unknown one) refreshes
/// nothing. A `run` whose packages cannot be listed (now, or before the step) refreshes nothing and says so. Err = the copy
/// failed (the caller fails the job: its output was built, but `main.bins` does not hold it).
pub fn refresh_main_bins(cfg: &Config, dir: &Path, source: &str, before: &StepBins) -> Result<Vec<String>, String> {
    if main_folder(cfg, source).as_deref() != Some(source) {
        return Ok(Vec::new());
    }
    if let Err(e) = &before.before {
        return Ok(vec![format!("main's binaries: not kept (the packages could not be listed before this step, so nothing tells which binaries it built: {e})")]);
    }
    let targets = match bins_of(cfg, dir) {
        Ok(t) => t,
        Err(e) => return Ok(vec![format!("main's binaries: not kept (the packages cannot be listed: {e})")]),
    };
    let mut lines = Vec::new();
    for (p, bins) in &targets {
        lines.extend(keep_main_bins(cfg, source, p, bins, "after this build (the build itself succeeded, but main.bins does not hold it)", Keep::AfterMainStep(before))?);
    }
    Ok(lines)
}

/// A folder's packages, each with its bin target names (what `bins_of` gives).
pub(super) type PackageBins = Vec<(String, Vec<String>)>;

/// THE list of main checkouts and their packages that a clean must consider before it deletes a package's artifacts from the
/// shared build folder: every project job.rs knows (`job::list_projects`: `broadsword` at the root and each project folder with
/// a Cargo.toml), as (its source folder: `.` or `<projects>/<p>`, its packages with their bin targets from `parse_bins`). An exe
/// path in cargo's folder is shared by every package with a bin of that name, so a clean or build from any folder may remove or
/// overwrite an exe another project's main checkout built (`keep_mains_sharing_exe_paths` matches by exe name). A project
/// folder with no Cargo.toml has no packages (cargo built nothing from it) and is left out.
/// An empty workspace is also skipped: it has no bins to protect (2026-11-22 incident).
/// Err = some project's packages cannot be listed: the caller refuses the clean (fail closed), because nothing then tells whether
/// that project's main binaries would be deleted.
pub(super) fn mains_with_packages(cfg: &Config) -> Result<Vec<(String, PackageBins)>, String> {
    let mut out = Vec::new();
    for name in crate::job::list_projects(cfg) {
        let refuse = |why: String| format!("refusing to clean: the packages of project `{name}` cannot be listed ({why}), so nothing tells whether this clean or build would remove or overwrite a binary its main checkout built at the same path; nothing was built or cleaned. Fix that project's Cargo.toml (or its folder) and repeat");
        let dir = crate::job::resolve_project(&cfg.root, &cfg.projects_dir, &name).map_err(refuse)?;
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        let main = rel_source(cfg, &dir).map_err(refuse)?;
        let metadata = metadata_json(cfg, &dir).map_err(refuse)?;
        let parsed = json::parse(&metadata).map_err(|e| refuse(format!("cannot read cargo metadata output: {e}")))?;
        if matches!(parsed.get("packages"), Some(json::J::Arr(packages)) if packages.is_empty()) {
            continue;
        }
        out.push((main, parse_bins(&metadata).map_err(refuse)?));
    }
    Ok(out)
}

/// True if two bin target names give the same exe path in cargo's folder (`<profile>/<bin><EXE_SUFFIX>`): equal names, and on
/// Windows equal without regard to ASCII case (its file names are case-insensitive; a bin name is plain ASCII).
fn same_exe_name(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// THE check before a job of `source` cleans package `pkg` (whose bin targets in the job's folder are `bins`) and then builds:
/// what is shared, and can be lost, is an exe PATH in cargo's folder, `<profile>/<bin><EXE_SUFFIX>`, whichever package produces
/// it. The paths this job can remove or write are its own bins of `pkg`, plus the bins of a package named `pkg` in any main
/// checkout (a package-level clean: the old package-name match is this case). For EVERY other main checkout in `mains`
/// (`mains_with_packages`) that has a bin, in any package, with one of those exe names, keep-or-refuse runs for that bin
/// (`keep_main_bins` with `Keep::BeforeForeignClean`: the bin note says whether that main checkout built the exe there; a vouched
/// exe is kept, one in use refuses the job, anything else is left). Err = the job is refused before anything is cleaned.
pub(super) fn keep_mains_sharing_exe_paths(cfg: &Config, source: &str, mains: &[(String, PackageBins)], pkg: &str, bins: &[String]) -> Result<Vec<String>, String> {
    let mut names: Vec<&str> = bins.iter().map(String::as_str).collect();
    for (_, targets) in mains {
        for (_, bs) in targets.iter().filter(|(n, _)| n == pkg) {
            names.extend(bs.iter().map(String::as_str));
        }
    }
    let when = format!("before cleaning `{pkg}` (this job is refused: `{pkg}` was not cleaned and nothing was built; a package listed before it may already have been cleaned, with main's binaries kept first)");
    let mut lines = Vec::new();
    for (m, targets) in mains.iter().filter(|(m, _)| m != source) {
        for (owner, bs) in targets {
            let shared: Vec<String> = bs.iter().filter(|b| names.iter().any(|n| same_exe_name(n, b))).cloned().collect();
            if !shared.is_empty() {
                lines.extend(keep_main_bins(cfg, m, owner, &shared, &when, Keep::BeforeForeignClean)?);
            }
        }
    }
    Ok(lines)
}

/// Write `text` and a newline to `path` atomically (temp file, flushed, renamed).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = atomic_tmp(path);
    let r = (|| {
        let mut f = create_fresh(&tmp)?;
        std::io::Write::write_all(&mut f, format!("{text}\n").as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

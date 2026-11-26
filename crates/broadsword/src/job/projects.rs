use super::*;

// ---- projects ---------------------------------------------------------------------------

/// Known project names: `broadsword` plus every folder in `projects_dir` holding a Cargo.toml.
pub fn list_projects(cfg: &Config) -> Vec<String> {
    let mut names = vec![SELF_PROJECT.to_string()];
    let mut found: Vec<String> = std::fs::read_dir(&cfg.projects_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("Cargo.toml").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n != SELF_PROJECT)
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    names.extend(found);
    names
}

/// The ONE place a project name is accepted: the canonical name (the folder's real name as `list_projects` gives it,
/// or `broadsword`), matched case-insensitively. A name is one plain folder name; `..`, separators, absolute paths
/// and names ending in a dot or space (Windows treats `Crash.` as `Crash`) are rejected, so `Crash`, `crash` and
/// `Crash.` can never be different projects.
pub fn canonical_project(projects_dir: &Path, name: &str) -> Result<String, String> {
    let mut comps = Path::new(name).components();
    let plain = matches!((comps.next(), comps.next()), (Some(Component::Normal(_)), None)) && !name.contains(['/', '\\', ':']) && !name.ends_with(['.', ' ']);
    if !plain {
        return Err(format!("invalid project name `{name}`: use a plain folder name, no path, `..`, absolute path, or trailing dot or space"));
    }
    let want = name.to_lowercase();
    if want == SELF_PROJECT {
        return Ok(SELF_PROJECT.to_string());
    }
    let mut found: Vec<String> = std::fs::read_dir(projects_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_lowercase() == want)
        .collect();
    found.sort();
    found.into_iter().next().ok_or_else(|| format!("unknown project `{name}`"))
}

/// Name -> project folder (see `canonical_project` for what a name may be).
pub fn resolve_project(root: &Path, projects_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let canon = canonical_project(projects_dir, name)?;
    Ok(if canon == SELF_PROJECT { root.to_path_buf() } else { projects_dir.join(canon) })
}

pub fn cmd_projects(cfg: &Config) -> ExitCode {
    for n in list_projects(cfg) {
        let dir = resolve_project(&cfg.root, &cfg.projects_dir, &n).unwrap_or_default();
        println!("{n}\t{}", dir.display());
    }
    ExitCode::SUCCESS
}

// ---- disk guard -------------------------------------------------------------------------

/// Pure decision. Err(message) = refuse to start. Free space minus the reserve landing exactly on the floor is allowed.
pub fn guard_decision(paused: bool, free: u64, reserve: u64, floor: u64) -> Result<(), String> {
    if paused {
        return Err("Broadsword is paused (see `broadsword status`); run `broadsword resume` once space is freed".into());
    }
    if free < floor.saturating_add(reserve) {
        return Err(format!(
            "free space {} minus the {} job reserve would be below the {} floor",
            pause::gb(free),
            pause::gb(reserve),
            pause::gb(floor)
        ));
    }
    Ok(())
}

/// Free-space source for a running job. Normally the real disk. TEST ONLY: if the env var
/// `BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE` names a file inside this root's scratch folder, its
/// whitespace-separated byte counts are used one per call (the last repeats) to simulate the disk
/// filling mid-job. Every reading is `min(hook value, real free space)`, so the hook can only make
/// the disk look fuller, never emptier: it can never switch off the real guard. Files outside
/// scratch are ignored, and use is announced on stderr.
pub fn hooked_free_source(cfg: &Config) -> Box<dyn FnMut() -> u64> {
    let root = cfg.root.clone();
    if let Ok(v) = std::env::var(TEST_FREE_HOOK_ENV) {
        let path = PathBuf::from(&v);
        let inside = match (path.canonicalize(), cfg.scratch_dir.canonicalize()) {
            (Ok(p), Ok(s)) => p.starts_with(s),
            _ => false,
        };
        if inside {
            eprintln!("broadsword: TEST HOOK ACTIVE: free space is read from {v} (never above the real free space)");
            let mut n = 0usize;
            return Box::new(move || {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let vals: Vec<u64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                let hooked = vals.get(n.min(vals.len().saturating_sub(1))).copied().unwrap_or(0);
                n += 1;
                hooked.min(real_free(&root))
            });
        }
        eprintln!("broadsword: ignoring {TEST_FREE_HOOK_ENV}: file is not inside the scratch folder");
    }
    Box::new(move || real_free(&root))
}

/// The cleanup a low-disk kill or refusal runs, once: the same pass as `broadsword cleanup` (scratch by age, log caps, the
/// build-folder sweep if cargo-sweep is installed), measured with the job's own free-space source. It re-measures and pauses
/// only if free space is still below the floor (`context` goes into the pause reason). Prints what was freed.
pub(super) fn low_disk_cleanup(cfg: &Config, free_src: &mut dyn FnMut() -> u64, context: &str) -> crate::cleanup::Report {
    let r = crate::cleanup::cleanup_pass_in(cfg, SystemTime::now(), free_src, true, context);
    crate::cleanup::print_report(&r);
    r
}

/// Real free space on the root's disk.
fn real_free(root: &Path) -> u64 {
    sys::disk(root).map(|d| d.free).unwrap_or_else(|e| {
        eprintln!("disk check failed: {e}");
        0
    })
}

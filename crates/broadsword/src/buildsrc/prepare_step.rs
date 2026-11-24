use super::*;

/// What `prepare` did, one line each, for the job's output and log.
pub struct Prepared {
    pub lines: Vec<String>,
}

/// Make the shared build folder safe to build `dir` into: for every package of `dir` whose note names another folder (or is
/// missing), clean that package's artifacts, then record `source` for it. A note `run:<source>` also requires cleaning:
/// an arbitrary command may have overwritten the exe without changing Cargo's fingerprint. The caller holds the build
/// lock. Err = nothing may be built (the message says why and what to do).
pub fn prepare(cfg: &Config, dir: &Path, source: &str) -> Result<Prepared, String> {
    let (targets, manifests) = targets_with_manifests(cfg, dir)?;
    let mut prepared = prepare_packages(cfg, dir, source, targets, false)?;
    prepared.lines.extend(manifests);
    Ok(prepared)
}

/// Read package identities from Cargo, not from a second TOML parser. These lines go into the durable job log along with
/// the source note. A package outside the job's checkout must not acquire that checkout's note.
fn targets_with_manifests(cfg: &Config, dir: &Path) -> Result<(PackageBins, Vec<String>), String> {
    let metadata = metadata_json(cfg, dir)?;
    let targets = parse_bins(&metadata)?;
    let parsed =
        json::parse(&metadata).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let Some(json::J::Arr(packages)) = parsed.get("packages") else {
        return Err("cargo metadata output: `packages` is not a list".into());
    };
    let checkout = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve job checkout {}: {e}", dir.display()))?;
    let mut lines = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(json::J::str)
            .ok_or("cargo metadata output: a package has no name")?;
        let manifest = package
            .get("manifest_path")
            .and_then(json::J::str)
            .ok_or("cargo metadata output: a package has no manifest_path")?;
        if manifest.is_empty()
            || manifest.chars().any(char::is_control)
            || !Path::new(manifest).is_absolute()
        {
            return Err(format!(
                "cargo metadata output: invalid manifest_path for package `{name}`"
            ));
        }
        let path = Path::new(manifest)
            .canonicalize()
            .map_err(|e| format!("cannot resolve manifest for package `{name}`: {e}"))?;
        if !path.starts_with(&checkout) {
            return Err(format!(
                "refusing to note package `{name}` as this checkout's build: manifest {} is outside {}",
                path.display(),
                checkout.display()
            ));
        }
        lines.push(format!("build manifest: {} (package: {name})", path.display()));
    }
    Ok((targets, lines))
}

/// Only a literal, single cargo command can prove which manifest a shell's `--manifest-path` names. Do not try to evaluate
/// shell expansion, scripts, command substitution or a changed working directory. Ambiguous commands are refused, rather
/// than checking one path and letting the shell build another. Backslashes are literal only on Windows.
pub(super) fn guard_run_manifest(dir: &Path, command: &str) -> Result<(), String> {
    if !command.contains("--manifest-path") {
        return Ok(());
    }
    let refuse = || {
        "refusing run: cannot attribute --manifest-path to the job checkout; use a literal cargo command naming this checkout's Cargo.toml, or build through the matching Broadsword project/task".to_string()
    };
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for c in command.chars() {
        if c.is_control()
            || "&|;<>$`%!^()*?[]{}~#".contains(c)
            || (!cfg!(windows) && c == '\\')
            || (cfg!(windows) && c == '\'')
        {
            return Err(refuse());
        }
        if Some(c) == quote {
            quote = None;
        } else if quote.is_none() && (c == '"' || c == '\'') {
            quote = Some(c);
        } else if quote.is_none() && c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() {
        return Err(refuse());
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !matches!(words.first().map(String::as_str), Some("cargo" | "cargo.exe")) {
        return Err(refuse());
    }
    let mut manifest = None;
    let mut args = words.iter().skip(1);
    while let Some(arg) = args.next() {
        let value = if arg == "--manifest-path" {
            Some(args.next().ok_or_else(refuse)?.as_str())
        } else {
            arg.strip_prefix("--manifest-path=")
        };
        if let Some(value) = value {
            if value.is_empty() || value.starts_with('-') || manifest.replace(value).is_some() {
                return Err(refuse());
            }
        } else if arg.contains("--manifest-path") {
            return Err(refuse());
        }
    }
    let manifest = manifest.ok_or_else(refuse)?;
    let path = Path::new(manifest);
    if !path.is_absolute()
        && path
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err(refuse());
    }
    let requested = dir.join(manifest).canonicalize().map_err(|e| {
        format!("refusing run: cannot resolve --manifest-path {manifest:?}: {e}")
    })?;
    let own = dir
        .join("Cargo.toml")
        .canonicalize()
        .map_err(|e| format!("refusing run: cannot resolve the job's Cargo.toml: {e}"))?;
    if requested != own {
        return Err(format!(
            "refusing run: --manifest-path names {}, not this job checkout's manifest {}; nothing was built or noted as main",
            requested.display(),
            own.display()
        ));
    }
    Ok(())
}

/// The preparation a `run` job makes before its command (the command may be `cargo test`, which would otherwise test the
/// binary another checkout left in the shared build folder): the same clean as `prepare` for a package whose note names
/// another folder or is missing, then the note becomes `run:<source>`, because the command may build anything and nothing
/// may vouch for the result (`install` refuses). Same folder: no clean, so `run -- cargo build` followed by `run -- <the
/// built exe>` keeps the exe. `run` runs arbitrary commands, so a folder whose packages cannot be listed (a broken
/// Cargo.toml the command may be there to diagnose) is NOT refused: the line says protection was skipped. A `cargo clean`
/// that fails (a locked binary) refuses the job, as it does for build, test and check. An explicit foreign or ambiguous
/// `--manifest-path` refuses the command before any notes change. Every accepted run invalidates the install binary's note,
/// even when this folder has no package named `broadsword`: a shell command or script may overwrite that shared exe.
/// The caller holds the lock.
pub fn prepare_for_run(
    cfg: &Config,
    dir: &Path,
    source: &str,
    command: &str,
) -> Result<Vec<String>, String> {
    guard_run_manifest(dir, command)?;
    let mut lines = match targets_with_manifests(cfg, dir) {
        Ok((packages, manifests)) => {
            let mut lines = prepare_packages(cfg, dir, source, packages, true)?.lines;
            lines.extend(manifests);
            lines
        }
        Err(e) => {
            // The packages cannot be listed (a broken Cargo.toml the command may be there to repair), but whatever the command
            // builds must still never be installable: note every package name still findable, else mark the whole folder.
            let wanted = format!("{RUN_NOTE_PREFIX}{source}");
            let (names, complete) = manifest_package_names(dir);
            for n in &names {
                write_note(cfg, n, &wanted)?;
                // a package's default binary carries the package's name (its `[[bin]]` names cannot be read here; a renamed
                // bin is still covered for `install` by the folder marker when the list is incomplete, and by nothing when
                // it is complete: a limit, since cargo cannot build a manifest it cannot read)
                write_bin_note(cfg, n, &wanted)?;
            }
            if !complete {
                write_unlisted_marker(cfg, source)?;
            }
            let how = match (names.is_empty(), complete) {
                (true, _) => "no package name found, so the folder is marked (`install` refuses until a plain build of this folder)".to_string(),
                (false, true) => format!("noted `{wanted}` for {}", names.join(", ")),
                (false, false) => format!("noted `{wanted}` for {}, and the folder is marked because a workspace member could not be read", names.join(", ")),
            };
            vec![format!("build source: not checked (the stale-binary protection is skipped for this run; {how}): {e}")]
        }
    };
    // Last, after preservation/cleaning, but before the caller can spawn the command. This is keyed by the shared exe,
    // not by the package the user selected. A later main build must clean it even if Cargo would call its fingerprint fresh.
    write_bin_note(cfg, env!("CARGO_PKG_NAME"), &format!("{RUN_NOTE_PREFIX}{source}"))?;
    lines.push(
        "build source: install provenance invalidated; an arbitrary run may build packages outside its job checkout".into(),
    );
    Ok(lines)
}

/// Directory (inside the notes folder) of the folder markers a `run` leaves when its packages could not be listed: ONE file
/// per source folder, named by the encoded folder, holding that folder's source. The name has a `.`, which no package name
/// has, so it can never collide with a package note.
pub const UNLISTED_DIR: &str = "unlisted-run.d";

pub(super) fn unlisted_dir(cfg: &Config) -> PathBuf {
    notes_dir(cfg).join(UNLISTED_DIR)
}

/// The marker file of a source folder: every character but letters, digits, `-` and `_` becomes `%XX` (per byte).
pub(super) fn marker_file(cfg: &Config, source: &str) -> PathBuf {
    let mut n = String::new();
    for b in source.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            n.push(b as char);
        } else {
            n.push_str(&format!("%{b:02X}"));
        }
    }
    unlisted_dir(cfg).join(n)
}

pub(super) fn write_unlisted_marker(cfg: &Config, source: &str) -> Result<(), String> {
    let dir = unlisted_dir(cfg);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = marker_file(cfg, source);
    std::fs::write(&path, format!("{source}\n")).map_err(|e| format!("cannot write the build-source marker {}: {e}", path.display()))
}

/// Every folder marker that exists: (file, the folder it holds). An unreadable marker names the folder `?`.
pub fn unlisted_markers(cfg: &Config) -> Vec<(PathBuf, String)> {
    let Ok(rd) = std::fs::read_dir(unlisted_dir(cfg)) else { return Vec::new() };
    let mut v: Vec<(PathBuf, String)> = rd
        .flatten()
        .map(|e| {
            let f = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().next().map(|l| l.trim().to_string())).filter(|l| !l.is_empty());
            (e.path(), f.unwrap_or_else(|| "?".into()))
        })
        .collect();
    v.sort();
    v
}

/// A manifest line without its `#` comment, trimmed.
fn toml_line(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// The `[package] name` of one manifest text, read line by line (the file may be too broken for cargo). `[package.metadata.*]`
/// and other `[package.*]` tables are not `[package]`.
pub fn package_name_in(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let l = toml_line(line);
        if l.starts_with('[') {
            in_package = l == "[package]";
        } else if in_package {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == "name" {
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                    if valid_package(v) {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// The quoted strings of the `[workspace] members = [...]` array of one manifest text (it may span lines).
pub fn workspace_members_in(text: &str) -> Vec<String> {
    let (mut in_ws, mut in_arr) = (false, false);
    let mut out = Vec::new();
    for line in text.lines() {
        let l = toml_line(line);
        let mut rest = l;
        if !in_arr {
            if l.starts_with('[') {
                in_ws = l == "[workspace]";
                continue;
            }
            match l.split_once('=') {
                Some((k, v)) if in_ws && k.trim() == "members" => {
                    in_arr = true;
                    rest = v;
                }
                _ => continue,
            }
        }
        let mut quote: Option<char> = None;
        let mut cur = String::new();
        for c in rest.chars() {
            match quote {
                Some(q) if c == q => {
                    out.push(std::mem::take(&mut cur));
                    quote = None;
                }
                Some(_) => cur.push(c),
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == ']' => in_arr = false,
                None => {}
            }
        }
    }
    out
}

/// The package names findable in `dir/Cargo.toml` and in the manifests of its `[workspace] members` (a trailing `/*` lists that
/// folder's subfolders), plus whether the list is complete: false if the root manifest is unreadable, nothing was found, or a
/// member's manifest could not be read or has no name. Incomplete means the caller marks the whole folder.
fn manifest_package_names(dir: &Path) -> (Vec<String>, bool) {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else { return (Vec::new(), false) };
    let mut names: Vec<String> = package_name_in(&text).into_iter().collect();
    let mut complete = true;
    for m in workspace_members_in(&text) {
        let folders: Vec<PathBuf> = match m.strip_suffix("/*") {
            Some(parent) => match std::fs::read_dir(dir.join(parent)) {
                Ok(rd) => rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect(),
                Err(_) => {
                    complete = false;
                    continue;
                }
            },
            None if m.contains('*') || m.contains('?') || m.contains('[') => {
                complete = false;
                continue;
            }
            None => vec![dir.join(&m)],
        };
        for f in folders {
            match std::fs::read_to_string(f.join("Cargo.toml")).ok().as_deref().and_then(package_name_in) {
                Some(n) => {
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                None => complete = false,
            }
        }
    }
    if names.is_empty() {
        complete = false;
    }
    (names, complete)
}

/// Packages are noted by name AND by each binary target's name (`bin.<name>` notes): two packages can build the same exe
/// name (a renamed package with `[[bin]] name = "broadsword"`), the later build overwrites the exe, and the package note of
/// the first stays valid. A package is stale when its own note OR any of its bin notes differs from it (another folder, a
/// `run:` note, or missing), and cleaning it (`cargo clean -p`, which removes the exes of ITS bin targets) is what removes a
/// same-named exe another package left. Every note of a cleaned package, bins included, is then written before the build.
fn prepare_packages(cfg: &Config, dir: &Path, source: &str, targets: Vec<(String, Vec<String>)>, as_run: bool) -> Result<Prepared, String> {
    let packages: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
    let bins_of_pkg = |p: &str| -> &[String] { targets.iter().find(|(n, _)| n == p).map(|(_, b)| b.as_slice()).unwrap_or(&[]) };
    // A plain build also cleans a package whose note is `run:<this folder>` (the run may have replaced the built exe while
    // cargo's fingerprint stays fresh), and every package of the folder when a run in THIS folder left a folder marker (only
    // that folder's marker is removed, and only after every package here is cleaned).
    let marker = marker_file(cfg, source);
    let marked = !as_run && marker.exists();
    let is_stale = |n: &Option<String>| -> bool {
        n.as_deref().map(note_folder) != Some(source) || (!as_run && (marked || n.as_deref().is_some_and(|x| x.starts_with(RUN_NOTE_PREFIX))))
    };
    // the note that decides a package's staleness, and the one its `cleaned` line names: its own note when that is stale,
    // else the first of its bin notes that is stale (another folder, a `run:` note where a plain build needs a clean, or
    // missing), else its own note
    let stale: Vec<(String, Option<String>)> = packages
        .iter()
        .filter_map(|p| {
            let own = read_note(cfg, p);
            if is_stale(&own) {
                return Some((p.clone(), own));
            }
            bins_of_pkg(p).iter().map(|b| read_bin_note(cfg, b)).find(|n| is_stale(n)).map(|n| (p.clone(), n))
        })
        .collect();
    task::wait_for_gate(cfg, TEST_SYNC_GATE_ENV, "the build-source sync (holding the build lock)");
    let mut lines = vec![format!("build source: {source} (packages: {})", packages.join(", "))];
    let wanted = if as_run { format!("{RUN_NOTE_PREFIX}{source}") } else { source.to_string() };
    // main's binaries: before a job of ANOTHER folder cleans a package, a successful main build that main.bins could not take yet
    // (its copy was in use) is kept (see `keep_main_bins`), for EVERY project whose main checkout has a bin with the same exe
    // name (`keep_mains_sharing_exe_paths`: an exe path is shared whichever package builds it, so another project's main exe can
    // be the one this clean removes or the build after it overwrites). Nothing is
    // ever copied back into cargo's folder after the clean: it then holds only what this job builds.
    let mains = if stale.is_empty() { Vec::new() } else { mains_with_packages(cfg)? };
    for (p, was) in &stale {
        lines.extend(keep_mains_sharing_exe_paths(cfg, source, &mains, p, bins_of_pkg(p))?);
        for profile in CLEAN_PROFILES {
            let flag = if profile == Profile::Release { " --release" } else { "" };
            let out = clean_command(cfg, dir, p, profile)?.output().map_err(|e| format!("cannot start `cargo clean -p {p}{flag}`: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "`cargo clean -p {p}{flag}` failed in {}: {}
nothing was built. The old copy of `{p}` in the build folder could not be removed; on Windows a binary that is running cannot be deleted, so stop whatever runs it (run the installed copy under install_dir, not the one in the build folder) and repeat",
                    dir.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        write_note(cfg, p, &wanted)?;
        for b in bins_of_pkg(p) {
            write_bin_note(cfg, b, &wanted)?;
        }
        lines.push(match was {
            Some(w) if w.starts_with(RUN_NOTE_PREFIX) && note_folder(w) == source => format!("cleaned `{p}` in the shared build folder (a `run` in this folder may have replaced its binary); its dependencies were kept"),
            Some(w) => format!("cleaned `{p}` in the shared build folder (it was last built from {}); its other packages and all dependencies were kept", note_folder(w)),
            None => format!("cleaned `{p}` in the shared build folder (no record of what built it); its dependencies were kept"),
        });
    }
    if marked {
        std::fs::remove_file(&marker).map_err(|e| format!("cannot remove the build-source marker {}: {e}", marker.display()))?;
    }
    // same folder: no clean; only the note's kind may change (run -> vouched by a build step, or a build -> run), for the
    // package note and each bin note alike (read again: a cleaned package's notes are already `wanted`)
    let same_kind_only = |was: Option<String>| was.as_deref().map(note_folder) == Some(source) && was.as_deref() != Some(wanted.as_str());
    for p in &packages {
        if same_kind_only(read_note(cfg, p)) {
            write_note(cfg, p, &wanted)?;
        }
        for b in bins_of_pkg(p) {
            if same_kind_only(read_bin_note(cfg, b)) {
                write_bin_note(cfg, b, &wanted)?;
            }
        }
    }
    Ok(Prepared { lines })
}


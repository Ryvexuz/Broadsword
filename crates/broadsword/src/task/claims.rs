use super::*;

// ---- claims -----------------------------------------------------------------------------

/// The files that may be claimed by key (`<file>#<key>`): the settings file, its loader and the shared test support,
/// which many tasks touch one key at a time. clients/codex_coder.py keeps the same list.
pub const KEY_CLAIM_FILES: [&str; 3] = ["broadsword.toml", "crates/broadsword/src/config.rs", "crates/broadsword/tests/support/mod.rs"];

/// Validate and normalise one claim: a file or folder path relative to the project root.
/// `/` and `\` are the same, `.` segments and a trailing slash vanish. Absolute paths, drive letters, `..`,
/// empty claims and patterns (`*`, `?` inside a path) are rejected; `*` alone is the whole project.
/// Only the files in [`KEY_CLAIM_FILES`] may have a `#<key>` suffix (`[a-z0-9_]+`).
pub fn normalize_claim(raw: &str) -> Result<String, String> {
    let bad = |why: &str| Err(format!("invalid claim `{raw}`: {why}"));
    if raw.chars().any(char::is_control) {
        return bad("contains a control character");
    }
    let s = raw.trim();
    if s.is_empty() {
        return bad("empty; name a file or folder, or `*` for the whole project");
    }
    if s == WHOLE_PROJECT {
        return Ok(WHOLE_PROJECT.into());
    }
    // Trim the path as before, but never trim an invalid key into a valid one.
    let (path, key) = match raw.trim_start().split_once('#') {
        Some((path, key)) => {
            if key.is_empty() || !key.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') {
                return bad("key must match [a-z0-9_]+");
            }
            (path.trim(), Some(key))
        }
        None => (s, None),
    };
    let s = path.replace('\\', "/");
    if s.starts_with('/') {
        return bad("absolute paths are not allowed; claims are relative to the project root");
    }
    let mut out: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => return bad("`..` is not allowed; a claim must stay inside the project"),
            _ if seg.contains(':') => return bad("`:` is not allowed (drive letters and stream names are not project-relative paths)"),
            _ if seg.contains(['*', '?']) => return bad("claims are paths, not patterns: no `*` or `?` (use `*` alone to claim the whole project)"),
            _ if seg.ends_with(' ') || seg.ends_with('.') => return bad("a name may not end with a space or a dot (Windows treats it as another name)"),
            _ => out.push(seg),
        }
    }
    if out.is_empty() {
        return bad("it names no file or folder; use `*` for the whole project");
    }
    let path = out.join("/");
    if let Some(key) = key {
        if !KEY_CLAIM_FILES.iter().any(|f| same(&path, f)) {
            return bad("key claims are only for broadsword.toml, crates/broadsword/src/config.rs and crates/broadsword/tests/support/mod.rs");
        }
        return Ok(format!("{path}#{key}"));
    }
    Ok(path)
}

/// Do two normalised claims overlap? Equal, or one is an ancestor folder of the other by whole path segments
/// (`src` overlaps `src/a.rs`, not `srcx`). `*` overlaps everything. `case_insensitive` is true on Windows.
/// Different keys of the same file do not overlap; a whole-file or containing-folder claim overlaps every key.
pub fn overlaps_with(a: &str, b: &str, case_insensitive: bool) -> bool {
    if a == WHOLE_PROJECT || b == WHOLE_PROJECT {
        return true;
    }
    let fold = |s: &str| if case_insensitive { s.to_lowercase() } else { s.to_string() };
    let (a, b) = (fold(a), fold(b));
    let (a, ak) = a.split_once('#').map_or((a.as_str(), None), |(p, k)| (p, Some(k)));
    let (b, bk) = b.split_once('#').map_or((b.as_str(), None), |(p, k)| (p, Some(k)));
    if a == b {
        return ak.is_none() || bk.is_none() || ak == bk;
    }
    let (mut x, mut y) = (a.split('/'), b.split('/'));
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) if p == q => {}
            (Some(_), Some(_)) => return false,
            (None, Some(_)) => return ak.is_none(), // only a plain claim can be an ancestor folder
            (Some(_), None) => return bk.is_none(),
            (None, None) => return true,
        }
    }
}

pub fn overlaps(a: &str, b: &str) -> bool {
    overlaps_with(a, b, cfg!(windows))
}

/// Normalise a list of claims, dropping repeats. At least one claim is required.
pub fn normalize_claims(raw: &[&str]) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Err("at least one claim is required".into());
    }
    let mut out: Vec<String> = Vec::new();
    for r in raw {
        let c = normalize_claim(r)?;
        if !out.iter().any(|o| same(o, &c)) {
            out.push(c);
        }
    }
    Ok(out)
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}


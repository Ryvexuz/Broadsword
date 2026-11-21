//! The ONE secret boundary for child processes. Every child Broadsword starts gets the environment this module returns, never
//! the parent's raw one. New spawn sites start their child with `command` (a `Config` in hand) or `command_in` (only a folder in
//! hand: git in `git.rs` and `wtgit.rs`); `tests/scrub.rs` fails on any `Command::new` under `src/` outside this file that is
//! not on its allow-list (each entry says why it is safe).
//!
//! The rule (also used by the reviewer, `reviewer.rs` re-exports it):
//! - a variable whose NAME looks like a secret (`secret_name`) is dropped;
//! - a variable whose VALUE equals or contains a value of `<root>/.env` is dropped;
//! - a `.env` value shorter than the `redact_min_chars` setting is not redacted from text (it would blank ordinary words), but a
//!   variable that EQUALS one is still dropped.
//!
//! This file depends only on std and `config.rs`, so a test crate that includes `wtgit.rs` by `#[path]` can include it too.
//!
//! Known limit: output is redacted line by line (job logs) or after the whole output is collected (MCP), so a secret split by a
//! line break inside a job's output is not recognised.

use crate::config::{find_config_dir, main_checkout_root, Config, CONFIG_FILE};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Command;

// ---- the rule ---------------------------------------------------------------------------------

/// Names that carry a secret (case-insensitive, as Windows treats them): `*_API_KEY`, `*_KEY`, `*_TOKEN`, `*_SECRET`, `*_PAT`,
/// `*_PASSWORD`, `*_PASS`, `*_CREDENTIAL*`, `*_AUTH*`, and the bare `API_KEY`, `TOKEN`, `SECRET`, `PAT`, `PASSWORD`, `PASS`.
pub fn secret_name(name: &str) -> bool {
    let n = name.to_uppercase();
    ["_KEY", "_TOKEN", "_SECRET", "_PAT", "_PASSWORD", "_PASS"].iter().any(|suffix| n.ends_with(suffix))
        || n.contains("_CREDENTIAL")
        || n.contains("_AUTH")
        || ["API_KEY", "TOKEN", "SECRET", "PAT", "PASSWORD", "PASS"].contains(&n.as_str())
}

/// Every secret value this process knows of.
/// - EVERY value in `<root>/.env` is a secret, whatever its name (`GITHUB_PAT`, `DB_PASSWORD`, `REGION`...);
/// - so is the value of every secret-named variable of the process environment.
///
/// Values of `redact_min_chars` characters or more are replaced by `[REDACTED]` in any text; shorter `.env` values are not (they
/// would redact common words), but a variable of the reviewer's environment that EQUALS one is still removed.
#[derive(Debug, Default, Clone)]
pub struct Secrets {
    redact: Vec<String>,
    short_env: Vec<String>,
}

fn env_file_values(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join(".env")) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((_, val)) = line.split_once('=') {
                let val = val.trim().trim_matches(['"', '\'']);
                if !val.is_empty() {
                    v.push(val.to_string());
                }
            }
        }
    }
    v
}

impl Secrets {
    /// `min_len` is the `redact_min_chars` setting: values shorter than this are not redacted from text (they would blank out
    /// ordinary words); they are still kept out of the reviewer's environment (see `scrub_names`).
    pub fn collect(root: &Path, env: &[(String, String)], min_len: usize) -> Secrets {
        let mut redact: Vec<String> = Vec::new();
        let mut short_env: Vec<String> = Vec::new();
        for val in env_file_values(root) {
            if val.chars().count() >= min_len {
                redact.push(val);
            } else {
                short_env.push(val);
            }
        }
        for (k, val) in env {
            if secret_name(k) && val.chars().count() >= min_len {
                redact.push(val.clone());
            }
        }
        // longest first, so a secret that contains another is replaced whole
        redact.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        redact.dedup();
        short_env.sort();
        short_env.dedup();
        Secrets { redact, short_env }
    }

    /// How many `.env` values are too short to redact from text.
    pub fn short_count(&self) -> usize {
        self.short_env.len()
    }

    /// `text` with every known secret value replaced by `[REDACTED]`.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for s in &self.redact {
            out = out.replace(s.as_str(), "[REDACTED]");
        }
        out
    }

    /// `kept`, a piece of a longer RAW (not yet redacted) text that was cut at its start, its end or both, made safe to show:
    /// THE one rule for text that is cut or tailed. Besides every whole secret (as `redact`), it masks the longest PREFIX of
    /// `kept` that equals the end of a secret value and the longest SUFFIX that equals the start of one, of 1 byte or more, so
    /// a secret split by the cut never shows in part; and if any secret value is longer than `kept`, all of `kept` is masked
    /// (it could lie wholly inside one). The masked spans are found on the raw text and merged, each run becoming one
    /// `[REDACTED]`, so a secret that overlaps a masked end is covered too. Fail-closed by design: it masks more than needed
    /// (an end that merely looks like the start or end of a secret, even an end that was not cut), never less.
    pub fn redact_cut(&self, kept: &str) -> String {
        if self.redact.iter().any(|s| s.len() > kept.len()) {
            return if kept.is_empty() { String::new() } else { "[REDACTED]".to_string() };
        }
        let b = kept.as_bytes();
        let n = b.len();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for s in self.redact.iter().filter(|s| !s.is_empty()) {
            let sb = s.as_bytes();
            spans.extend(kept.match_indices(s.as_str()).map(|(i, m)| (i, i + m.len())));
            // the longest suffix of `kept` that is a prefix of `s` (the cut fell inside `s`, after its start)
            if let Some(p) = (n + 1 - sb.len().min(n)..n).find(|&p| sb.starts_with(&b[p..])) {
                spans.push((p, n));
            }
            // the longest prefix of `kept` that is a suffix of `s` (the cut fell inside `s`, before its end)
            if let Some(q) = (1..sb.len().min(n + 1)).rev().find(|&q| sb.ends_with(&b[..q])) {
                spans.push((0, q));
            }
        }
        // widen each span to char boundaries (they already are when a secret's bytes match whole characters), then merge
        for sp in spans.iter_mut() {
            while !kept.is_char_boundary(sp.0) {
                sp.0 -= 1;
            }
            while !kept.is_char_boundary(sp.1) {
                sp.1 += 1;
            }
        }
        spans.sort_unstable();
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (st, en) in spans {
            match runs.last_mut() {
                Some(last) if st <= last.1 => last.1 = last.1.max(en),
                _ => runs.push((st, en)),
            }
        }
        let mut out = String::new();
        let mut at = 0usize;
        for (st, en) in runs {
            out.push_str(&kept[at..st]);
            out.push_str("[REDACTED]");
            at = en;
        }
        out.push_str(&kept[at..]);
        out
    }

    /// Which variables of `env` the reviewer CLI must NOT inherit: secret-named ones; any whose value equals or contains a
    /// secret value; any whose value equals a short `.env` value.
    pub fn scrub_names(&self, env: &[(String, String)]) -> Vec<String> {
        env.iter()
            .filter(|(k, v)| secret_name(k) || self.redact.iter().any(|s| v.contains(s.as_str())) || self.short_env.iter().any(|s| v == s))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

// ---- the boundary -----------------------------------------------------------------------------

/// Every secret this process knows of: the root's `.env` values and the secret-named variables of the current environment.
pub fn secrets(cfg: &Config) -> Secrets {
    let env: Vec<(String, String)> = std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())).collect();
    Secrets::collect(&cfg.root, &env, cfg.redact_min_chars as usize)
}

/// The parent's environment minus every secret-named variable and every variable holding a `.env` value.
pub fn child_env(cfg: &Config) -> Vec<(OsString, OsString)> {
    let all: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    filter_env(&secrets(cfg), all)
}

fn filter_env(secrets: &Secrets, all: Vec<(OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let text: Vec<(String, String)> = all.iter().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())).collect();
    let mut drop: Vec<String> = secrets.scrub_names(&text).into_iter().map(|n| n.to_uppercase()).collect(); // Windows names are case-insensitive
    // the Broadsword markers (`MARKERS`) pass only in their exact shape; the secret rule above still applies to them first
    for (k, v) in &text {
        if let Some((_, _, shape_ok)) = MARKERS.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(k)) {
            if !shape_ok(v) {
                drop.push(k.to_uppercase());
            }
        }
    }
    all.into_iter().filter(|(k, _)| !drop.contains(&k.to_string_lossy().to_uppercase())).collect()
}

// ---- Broadsword's own markers -----------------------------------------------------------------

/// Set by job.rs on the command of a `run` that holds the build lock, as `<job id>:<pid>` (`run_lock_marker`). A Broadsword
/// call inside that command reads it in `buildsrc::lock` and refuses at once instead of waiting out a lock its parent holds.
pub const RUN_LOCK_MARKER: &str = "BROADSWORD_RUN_HOLDS_BUILD_LOCK";

/// (name, why it is safe, the only value shape that passes).
type Marker = (&'static str, &'static str, fn(&str) -> bool);

/// The allow-list of Broadsword's own variables that cross `child_env`: (name, why it is safe, the only value shape that
/// passes). An entry never overrides the secret rule (a marker holding a secret value is dropped like any variable); it only
/// adds that a malformed value is dropped too.
const MARKERS: &[Marker] = &[(
    RUN_LOCK_MARKER,
    "holds only a job id and a pid (no secret); a nested Broadsword call needs it to refuse at once instead of deadlocking on the build lock its parent run holds (buildsrc::lock)",
    |v| parse_run_lock_marker(v).is_some(),
)];

/// The marker value for job `job_id` of this process.
pub fn run_lock_marker(job_id: &str) -> String {
    format!("{job_id}:{}", std::process::id())
}

/// `<job id>:<pid>` -> (job id, pid). The job id is 1 to 200 ASCII letters, digits, `-`, `_` or `.`; the pid is decimal digits
/// only (no sign, no space), not 0. Anything else is None (no marker).
pub fn parse_run_lock_marker(v: &str) -> Option<(&str, u32)> {
    let (id, pid) = v.rsplit_once(':')?;
    let id_ok = !id.is_empty() && id.len() <= 200 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
    let pid_ok = !pid.is_empty() && pid.len() <= 10 && pid.bytes().all(|b| b.is_ascii_digit());
    if !id_ok || !pid_ok {
        return None;
    }
    let pid: u32 = pid.parse().ok()?;
    (pid != 0).then_some((id, pid))
}

/// Give `cmd` exactly the scrubbed environment: everything is cleared first, so nothing the parent holds leaks through.
pub fn apply(cmd: &mut Command, cfg: &Config) {
    cmd.env_clear().envs(child_env(cfg));
}

/// THE spawn helper: `program`, with the scrubbed environment already in place. The caller adds its own variables after.
pub fn command(cfg: &Config, program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    apply(&mut c, cfg);
    c
}

/// `command` for a caller that has only the folder the child runs in (git in `git.rs` and `wtgit.rs`): the root, and so its
/// `.env` and `redact_min_chars`, are found from `dir` exactly as `config::load` finds them from the current folder (the first
/// folder upwards holding `broadsword.toml`, then the main checkout of a linked worktree). Fails closed: with no
/// `broadsword.toml` above `dir`, or one that does not load, the secrets to keep out are unknown, so no child is started.
pub fn command_in(dir: &Path, program: impl AsRef<OsStr>) -> Result<Command, String> {
    let start = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| format!("cannot read the current folder to find {CONFIG_FILE}: {e}"))?.join(dir)
    };
    let found = find_config_dir(&start).ok_or_else(|| {
        format!(
            "no {CONFIG_FILE} in {} or any parent, so the secrets to keep from `{}` are unknown; refusing to start it",
            start.display(),
            program.as_ref().to_string_lossy()
        )
    })?;
    let cfg = Config::load_from(&main_checkout_root(&found)).map_err(|e| format!("refusing to start `{}`: the settings that say which secrets to keep from it do not load: {e}", program.as_ref().to_string_lossy()))?;
    Ok(command(&cfg, program))
}

#[cfg(test)]
pub mod planted {
    //! A throwaway Broadsword root with a planted secret, for the spawn tests of every module that starts a child.
    use std::path::{Path, PathBuf};

    /// A secret-named variable planted in this test process's environment.
    pub const API_NAME: &str = "PLANTED_API_KEY";
    pub const API_VALUE: &str = "planted-api-key-7f3a91c2";
    /// A `.env` value of the planted root, also held by this process under a harmless name.
    pub const DOTENV_VALUE: &str = "planted-dotenv-value-5be84d";
    pub const COPY_NAME: &str = "BS_PLANTED_DOTENV_COPY";

    /// `<repo>/scratch/<tag>-<pid>`, emptied, holding this build's own shipped `broadsword.toml` and nothing else: the root
    /// for a unit test that starts a child (git, python) through `command_in`. Why: a test folder with no `broadsword.toml`
    /// of its own would find the checkout's, and in a linked worktree `command_in` then loads the MAIN checkout's file,
    /// which lacks any required key this branch adds, so every git call in the test would be refused. No `.env`, and no
    /// variable is set.
    pub fn config_root(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("broadsword.toml"), include_str!("../../../broadsword.toml")).unwrap();
        d
    }

    /// `config_root("scrub-<tag>")` plus a `.env` holding `DOTENV_VALUE`; the two
    /// planted variables are set in this process.
    pub fn root(tag: &str) -> PathBuf {
        let d = config_root(&format!("scrub-{tag}"));
        std::fs::write(d.join(".env"), format!("SOME_SETTING={DOTENV_VALUE}\n")).unwrap();
        std::env::set_var(API_NAME, API_VALUE);
        std::env::set_var(COPY_NAME, DOTENV_VALUE);
        d
    }

    /// `out` is a child's environment dump: it must list PATH (so the dump is real) and nothing planted. A failure names only
    /// variables, never the dump's values, so a broken scrub cannot write this machine's real secrets into a test log.
    pub fn assert_clean(what: &str, out: &str) {
        let names: Vec<&str> = out.lines().filter_map(|l| l.split_once('=').map(|(k, _)| k.trim())).collect();
        assert!(names.iter().any(|k| k.eq_ignore_ascii_case("PATH")), "{what}: the child's environment dump lists no PATH, so it proves nothing ({} lines)", out.lines().count());
        for bad in [API_NAME, API_VALUE, COPY_NAME, DOTENV_VALUE] {
            let holders: Vec<&str> = out.lines().filter(|l| l.contains(bad)).filter_map(|l| l.split_once('=').map(|(k, _)| k.trim())).collect();
            assert!(!out.contains(bad), "{what}: the child saw `{bad}` (in the variable(s) {holders:?}); its variables: {names:?}");
        }
    }

    /// The program and arguments that print the environment.
    pub fn env_dump() -> (&'static str, &'static [&'static str]) {
        if cfg!(windows) {
            ("cmd", &["/C", "set"])
        } else {
            ("env", &[])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(k: &str, v: &str) -> (OsString, OsString) {
        (k.into(), v.into())
    }

    fn one(secret: &str) -> Secrets {
        Secrets::collect(Path::new("no-such-root-for-scrub-test"), &[("CUT_TEST_TOKEN".into(), secret.into())], 6)
    }

    #[test]
    fn redact_cut_masks_a_secret_whose_first_half_ends_the_kept_text() {
        let secret = "QZXJVKWBGHMF";
        let s = one(secret);
        assert_eq!(s.redact_cut(&format!("log: value {}", &secret[..6])), "log: value [REDACTED]");
        assert_eq!(s.redact_cut(&format!("log: value {}", &secret[..1])), "log: value [REDACTED]", "even one byte");
        assert_eq!(s.redact_cut(&format!("{} and more", &secret[6..])), "[REDACTED] and more", "a cut at the start too");
    }

    #[test]
    fn redact_cut_redacts_a_short_secret_fully_inside_as_before() {
        let secret = "QZXJVKWBGHMF";
        let s = one(secret);
        let text = format!("a {secret} b {secret} c");
        assert_eq!(s.redact_cut(&text), "a [REDACTED] b [REDACTED] c");
        assert_eq!(s.redact_cut(&text), s.redact(&text), "the same as redact when no end looks like part of a secret");
        assert_eq!(s.redact_cut("nothing secret here"), "nothing secret here");
    }

    #[test]
    fn redact_cut_masks_everything_when_a_secret_is_longer_and_merges_overlaps() {
        let secret = "QZXJVKWBGHMF";
        let s = one(secret);
        assert_eq!(s.redact_cut(&secret[3..9]), "[REDACTED]", "it could lie inside the secret");
        assert_eq!(s.redact_cut(""), "");
        // a whole secret overlapping a masked end, and both ends masked at once: one marker per merged run, no byte of it left
        let text = format!("{}{secret}xy{}", &secret[4..], &secret[..3]);
        let out = s.redact_cut(&text);
        assert_eq!(out, "[REDACTED]xy[REDACTED]", "{out}");
        // multi-byte characters around the cut stay whole
        let u = one("éQZXJVKWBé");
        assert_eq!(u.redact_cut("ü-ü-ü-ü éQZ"), "ü-ü-ü-ü [REDACTED]");
    }

    #[test]
    fn drops_secret_names_and_values_containing_a_dotenv_value_and_keeps_the_rest() {
        let s = Secrets::collect(Path::new("no-such-root-for-scrub-test"), &[("MY_API_KEY".into(), "longsecretvalue".into())], 6);
        let kept = filter_env(&s, vec![os("PATH", "/bin"), os("my_api_key", "x"), os("OTHER", "prefix-longsecretvalue-suffix"), os("HARMLESS", "ok")]);
        let names: Vec<String> = kept.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["PATH", "HARMLESS"]);
    }

    #[test]
    fn the_run_lock_marker_passes_only_in_its_shape_and_never_with_a_secret() {
        assert_eq!(parse_run_lock_marker("20261121-211202-broadsword-run:4242"), Some(("20261121-211202-broadsword-run", 4242)));
        for bad in ["", ":1", "job:", "job:0", "job:-1", "job:+1", "job: 1", "job:1 ", "a b:1", "job:99999999999", "j\\x:1", "j:1\n"] {
            assert_eq!(parse_run_lock_marker(bad), None, "{bad:?}");
        }
        let s = Secrets::collect(Path::new("no-such-root-for-scrub-test"), &[("X_TOKEN".into(), "markersecret77".into())], 6);
        let names = |kept: Vec<(OsString, OsString)>| kept.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(filter_env(&s, vec![os("PATH", "/bin"), os(RUN_LOCK_MARKER, "job-1:77")])), ["PATH", RUN_LOCK_MARKER]);
        assert_eq!(names(filter_env(&s, vec![os("PATH", "/bin"), os("broadsword_run_holds_build_lock", "not a marker")])), ["PATH"], "malformed: dropped, any case");
        assert_eq!(names(filter_env(&s, vec![os("PATH", "/bin"), os(RUN_LOCK_MARKER, "markersecret77:77")])), ["PATH"], "a secret value is dropped even in the marker's shape");
    }

    fn dump(mut c: Command) -> String {
        let (_, args) = planted::env_dump();
        let o = c.args(args).output().unwrap();
        assert!(o.status.success(), "env dump failed: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn command_child_sees_no_planted_secret() {
        let root = planted::root("command");
        let cfg = Config::load_from(&root).unwrap();
        let out = dump(command(&cfg, planted::env_dump().0));
        planted::assert_clean("scrub::command", &out);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn command_in_child_sees_no_planted_secret() {
        let root = planted::root("command-in");
        let sub = root.join("deeper").join("dir");
        std::fs::create_dir_all(&sub).unwrap();
        let out = dump(command_in(&sub, planted::env_dump().0).unwrap());
        planted::assert_clean("scrub::command_in", &out);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn command_in_fails_closed_without_a_config() {
        let nowhere = std::env::temp_dir().join(format!("bs-scrub-no-config-{}", std::process::id())).join("x");
        assert!(find_config_dir(&nowhere).is_none(), "test premise: no {CONFIG_FILE} above {}", nowhere.display());
        let e = command_in(&nowhere, "git").expect_err("must refuse");
        assert!(e.contains("refusing to start it") && e.contains(CONFIG_FILE), "{e}");
    }

    #[test]
    fn command_in_fails_closed_on_a_config_that_does_not_load() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("scrub-badcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(CONFIG_FILE), "root = \".\"\n").unwrap();
        let e = command_in(&d, "git").expect_err("must refuse");
        assert!(e.contains("refusing to start `git`"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

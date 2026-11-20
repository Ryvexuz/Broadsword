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
    let drop: Vec<String> = secrets.scrub_names(&text).into_iter().map(|n| n.to_uppercase()).collect(); // Windows names are case-insensitive
    all.into_iter().filter(|(k, _)| !drop.contains(&k.to_string_lossy().to_uppercase())).collect()
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

    /// `<repo>/scratch/scrub-<tag>-<pid>`: a root with the real `broadsword.toml` and a `.env` holding `DOTENV_VALUE`; the two
    /// planted variables are set in this process.
    pub fn root(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("scrub-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let toml = include_str!("../../../broadsword.toml");
        std::fs::write(d.join("broadsword.toml"), toml).unwrap();
        std::fs::write(d.join(".env"), format!("SOME_SETTING={DOTENV_VALUE}\n")).unwrap();
        std::env::set_var(API_NAME, API_VALUE);
        std::env::set_var(COPY_NAME, DOTENV_VALUE);
        d
    }

    /// `out` is a child's environment dump: it must list PATH (so the dump is real) and nothing planted.
    pub fn assert_clean(what: &str, out: &str) {
        assert!(out.to_uppercase().contains("PATH="), "{what}: the child's environment dump has no PATH, so it proves nothing:\n{out}");
        for bad in [API_NAME, API_VALUE, COPY_NAME, DOTENV_VALUE] {
            assert!(!out.contains(bad), "{what}: the child saw `{bad}`:\n{out}");
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

    #[test]
    fn drops_secret_names_and_values_containing_a_dotenv_value_and_keeps_the_rest() {
        let s = Secrets::collect(Path::new("no-such-root-for-scrub-test"), &[("MY_API_KEY".into(), "longsecretvalue".into())], 6);
        let kept = filter_env(&s, vec![os("PATH", "/bin"), os("my_api_key", "x"), os("OTHER", "prefix-longsecretvalue-suffix"), os("HARMLESS", "ok")]);
        let names: Vec<String> = kept.iter().map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["PATH", "HARMLESS"]);
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
        let e = command_in(&nowhere, "git").err().expect("must refuse");
        assert!(e.contains("refusing to start it") && e.contains(CONFIG_FILE), "{e}");
    }

    #[test]
    fn command_in_fails_closed_on_a_config_that_does_not_load() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("scrub-badcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(CONFIG_FILE), "root = \".\"\n").unwrap();
        let e = command_in(&d, "git").err().expect("must refuse");
        assert!(e.contains("refusing to start `git`"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}

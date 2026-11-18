//! The ONE secret boundary for child processes. Every child Broadsword starts for a job (`job.rs`) and every tool child the
//! MCP server starts (`mcp.rs`) gets the environment this module returns, never the parent's raw one.
//!
//! The rule is the reviewer's (`reviewer::Secrets`, reused here, not copied):
//! - a variable whose NAME looks like a secret (`reviewer::secret_name`) is dropped;
//! - a variable whose VALUE equals or contains a value of `<root>/.env` is dropped;
//! - a `.env` value shorter than the `redact_min_chars` setting is not redacted from text (it would blank ordinary words), but a
//!   variable that EQUALS one is still dropped.
//!
//! Known limit: output is redacted line by line (job logs) or after the whole output is collected (MCP), so a secret split by a
//! line break inside a job's output is not recognised.

use crate::config::Config;
use crate::reviewer::Secrets;
use std::ffi::OsString;
use std::process::Command;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
}

//! Low-risk Claude advisory reviews. Their records never participate in the gating decision.

use super::entry::queue_dir;
use super::Run;
use crate::reviewer::{self, CallError, ReviewBackend};

// Keep the existing fresh-attempt reset loop authoritative for both recorded-only reviews.
pub(super) const REVIEW_KEYS: [&str; 16] = [
    "shadow_model",
    "shadow_model_used",
    "shadow_verdict",
    "shadow_reason",
    "shadow_blocking",
    "shadow_cost_usd",
    "shadow_tokens",
    "shadow_secs",
    "shadow_file",
    "advisory_model",
    "advisory_verdict",
    "advisory_reason",
    "advisory_blocking",
    "advisory_cost_usd",
    "advisory_secs",
    "advisory_file",
];

impl Run<'_> {
    /// Deliberately returns no pipeline result: even a recording failure cannot veto the gate.
    pub(super) fn s_advisory(&mut self, prompt: &str) {
        let result = self.advisory_review(prompt, &reviewer::ClaudeCli);
        if let Err(reason) = result {
            let reason = self.secrets.redact(&reason);
            eprintln!(
                "queue entry {}: advisory recording error: {reason}",
                self.e.id()
            );
            if let Err(error) = self.advisory_end("ERROR", &reason) {
                eprintln!(
                    "queue entry {}: cannot persist advisory ERROR: {}",
                    self.e.id(),
                    self.secrets.redact(&error)
                );
            }
        }
    }

    fn advisory_review(&mut self, prompt: &str, backend: &dyn ReviewBackend) -> Result<(), String> {
        let model = self.cfg.reviewer_model_low_advisory.clone();
        if model == "off"
            || self.e.get("risk") != "low"
            || self.e.get("verdict").is_empty()
            || !self.e.get("advisory_model").is_empty()
        {
            return Ok(());
        }
        self.e.set("advisory_model", model.clone());
        for (role, other) in [
            ("gating reviewer", self.e.get("reviewer_model").to_string()),
            ("coder", self.e.get("coder").to_string()),
        ] {
            if reviewer::model_family(&model) == reviewer::model_family(&other) {
                let reason = format!(
                    "the advisory model `{model}` is the same family as the {role} `{other}`"
                );
                return self.advisory_end("SKIPPED", &reason);
            }
        }
        // Persist the one-call reservation before preparing or starting a child. A crash leaves an
        // explicit ERROR, not an invitation to spend again when this attempt is resumed.
        self.e.set("advisory_verdict", "ERROR");
        self.e.set(
            "advisory_reason",
            "advisory review interrupted before its result was recorded",
        );
        self.advisory_save()?;
        let cli = match backend.prepare(self.cfg) {
            Ok(cli) => cli,
            Err(reason) => return self.advisory_end("ERROR", &reason),
        };
        self.advisory_log(&format!(
            "calling the advisory reviewer {model} (recorded only, never gating)"
        ))?;
        let start = std::time::Instant::now();
        let result = backend.call(
            self.cfg,
            &cli,
            &model,
            "",
            reviewer::gate_system_prompt(self.e.get("risk")),
            prompt,
        );
        self.e.set(
            "advisory_secs",
            format!("{:.1}", start.elapsed().as_secs_f64()),
        );
        let review = match result {
            Ok(review) => review,
            Err(CallError::Timeout(seconds)) => {
                return self.advisory_end(
                    "ERROR",
                    &format!("no answer within {seconds} s (reviewer_timeout_seconds); it was killed"),
                );
            }
            Err(CallError::Start(reason) | CallError::Failed(reason)) => {
                return self.advisory_end("ERROR", &reason);
            }
        };
        let verdict = reviewer::judge(&review);
        self.e.set(
            "advisory_cost_usd",
            review.cost_usd.clone().unwrap_or_default(),
        );
        self.e.set("advisory_blocking", verdict.blocking.join("\n"));
        let answer = if review.invalid.is_some() || !verdict.parsed {
            "ERROR"
        } else if verdict.pass {
            "PASS"
        } else {
            "FAIL"
        };
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 {
            format!("{}.advisory.review", self.e.id())
        } else {
            format!("{}.{attempt}.advisory.review", self.e.id())
        };
        let path = queue_dir(self.cfg).join(name);
        let text = self.secrets.redact(&format!(
            "entry: {}\nattempt: {attempt}\nadvisory review: recorded only, never gating\nmodel: {model}\nverdict: {answer}\nreason: {}\ncost_usd: {}\nsecs: {}\nblocking:\n{}\n--- raw advisory output ---\n{}\n",
            self.e.id(),
            verdict.note,
            self.e.get("advisory_cost_usd"),
            self.e.get("advisory_secs"),
            self.e.get("advisory_blocking"),
            review.raw
        ));
        let tmp = path.with_extension("review.tmp");
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|error| format!("cannot write advisory review {}: {error}", path.display()))?;
        self.e.set(
            "advisory_file",
            path.strip_prefix(&self.cfg.root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        self.advisory_end(answer, if answer == "ERROR" { &verdict.note } else { "" })
    }

    fn advisory_save(&mut self) -> Result<(), String> {
        // These fields are not in entry.rs's legacy TEXT_KEYS. Scrub before *any* save or log.
        for key in REVIEW_KEYS.iter().filter(|key| key.starts_with("advisory_")) {
            let value = self.secrets.redact(self.e.get(key));
            self.e.set(key, value);
        }
        self.e
            .save()
            .map_err(|error| format!("cannot save advisory entry: {error}"))
    }

    fn advisory_log(&mut self, text: &str) -> Result<(), String> {
        let text = self.secrets.redact(text);
        println!("queue entry {}: {text}", self.e.id());
        let log = format!(
            "{}\n{} advisory: {text}",
            self.e.get("log"),
            crate::pause::now_string()
        );
        self.e.set("log", log);
        self.advisory_save()
    }

    fn advisory_end(&mut self, verdict: &str, reason: &str) -> Result<(), String> {
        self.e.set("advisory_verdict", verdict);
        self.e.set("advisory_reason", reason);
        self.advisory_save()?;
        let reason = self.secrets.redact(reason);
        self.advisory_log(&format!(
            "advisory review {verdict} (recorded only; the gating review decides){}",
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::record::Record;
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        cfg: Config,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../scratch")
                .join(format!(
                    "advisory-unit-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            let mut cfg =
                Config::from_text(include_str!("../../../../broadsword.toml"), &root).unwrap();
            cfg.reviewer_model_low_advisory = "sonnet".into();
            std::fs::create_dir_all(queue_dir(&cfg)).unwrap();
            Self { cfg }
        }

        fn run(&self) -> Run<'_> {
            let path = queue_dir(&self.cfg).join("unit.entry");
            let entry = Record::parse(
                &path,
                "id = unit\nrisk = low\ncoder = opus\nreviewer_model = haiku\nverdict = PASS\nreview_calls = 1\n",
            );
            Run::new(&self.cfg, entry, self.cfg.root.clone())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let scratch = std::fs::canonicalize(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scratch"),
            )
            .unwrap();
            let root = std::fs::canonicalize(&self.cfg.root).unwrap();
            assert!(root.starts_with(&scratch) && root != scratch);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    struct Backend {
        calls: Cell<usize>,
        kind: &'static str,
    }

    impl Backend {
        fn new(kind: &'static str) -> Self {
            Self {
                calls: Cell::new(0),
                kind,
            }
        }
    }

    impl ReviewBackend for Backend {
        fn name(&self) -> &'static str {
            "unit advisory"
        }

        fn prepare(&self, _cfg: &Config) -> Result<reviewer::Cli, String> {
            if self.kind == "prepare" {
                return Err("advisory CLI refused".into());
            }
            Ok(reviewer::Cli {
                path: "unit".into(),
                test_hook: true,
            })
        }

        fn call(
            &self,
            cfg: &Config,
            _cli: &reviewer::Cli,
            model: &str,
            effort: &str,
            system: &str,
            prompt: &str,
        ) -> Result<reviewer::Review, CallError> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(model, "sonnet");
            assert_eq!(effort, "");
            assert_eq!(system, reviewer::gate_system_prompt("low"));
            assert_eq!(prompt, "same prompt");
            if self.kind == "timeout" {
                return Err(CallError::Timeout(cfg.reviewer_timeout_seconds));
            }
            if self.kind == "start" {
                return Err(CallError::Start("cannot start advisory".into()));
            }
            if self.kind == "failed" {
                return Err(CallError::Failed("advisory CLI failed".into()));
            }
            let text = match self.kind {
                "fail" => "VERDICT: FAIL\nBLOCKING: src/lib.rs:1 - advisory finding\nMINOR: none",
                "malformed" => "not a verdict",
                _ => "VERDICT: PASS\nBLOCKING: none\nMINOR: none",
            };
            let raw = if self.kind == "large" {
                format!("{text}\n{}", "x".repeat(cfg.reviewer_diff_max_kb as usize * 1024))
            } else {
                text.into()
            };
            Ok(reviewer::Review {
                raw,
                text: text.into(),
                cost_usd: Some("0.0123".into()),
                input_tokens: None,
                output_tokens: None,
                models_used: vec!["sonnet".into()],
                model_reported: None,
                reported_models: vec![],
                invalid: (self.kind == "invalid").then(|| "refused or truncated response".into()),
            })
        }
    }

    #[test]
    fn advisory_guard_is_at_the_call_entry_point() {
        for (key, value, expected) in [
            ("risk", "high", ""),
            ("verdict", "", ""),
            ("advisory_model", "sonnet", ""),
            ("reviewer_model", "claude-sonnet-4-5", "SKIPPED"),
            ("coder", " SONNET ", "SKIPPED"),
        ] {
            let fixture = Fixture::new();
            let mut run = fixture.run();
            let backend = Backend::new("pass");
            run.e.set(key, value);
            run.advisory_review("same prompt", &backend).unwrap();
            assert_eq!(backend.calls.get(), 0, "{key}");
            assert_eq!(run.e.get("advisory_verdict"), expected, "{key}");
        }
        let mut fixture = Fixture::new();
        fixture.cfg.reviewer_model_low_advisory = "off".into();
        let mut run = fixture.run();
        let backend = Backend::new("pass");
        run.advisory_review("same prompt", &backend).unwrap();
        assert_eq!(backend.calls.get(), 0);
        assert!(run.e.get("advisory_model").is_empty());
    }

    #[test]
    fn advisory_records_answers_and_errors_once_without_changing_the_gate_or_cap() {
        for (kind, expected) in [
            ("pass", "PASS"),
            ("fail", "FAIL"),
            ("malformed", "ERROR"),
            ("invalid", "ERROR"),
            ("timeout", "ERROR"),
            ("start", "ERROR"),
            ("failed", "ERROR"),
        ] {
            for gate in ["PASS", "FAIL"] {
                let fixture = Fixture::new();
                let mut run = fixture.run();
                run.e.set("verdict", gate);
                run.e.set("blocking", "original gating finding");
                let backend = Backend::new(kind);
                run.advisory_review("same prompt", &backend).unwrap();
                run.advisory_review("same prompt", &backend).unwrap();
                assert_eq!(backend.calls.get(), 1, "{kind}");
                assert_eq!(run.e.get("verdict"), gate);
                assert_eq!(run.e.get("blocking"), "original gating finding");
                assert_eq!(run.e.get("review_calls"), "1");
                assert_eq!(run.e.get("advisory_verdict"), expected, "{kind}");
                if expected == "ERROR" {
                    assert!(!run.e.get("advisory_reason").is_empty(), "{kind}");
                }
                assert!(!run.e.get("advisory_secs").is_empty());
                if matches!(kind, "pass" | "fail" | "malformed" | "invalid") {
                    assert_eq!(run.e.get("advisory_cost_usd"), "0.0123");
                    assert!(fixture.cfg.root.join(run.e.get("advisory_file")).is_file());
                }
            }
        }
    }

    #[test]
    fn advisory_reset_keys_cover_every_advisory_field() {
        let keys: Vec<_> = REVIEW_KEYS
            .iter()
            .filter(|key| key.starts_with("advisory_"))
            .copied()
            .collect();
        assert_eq!(
            keys,
            [
                "advisory_model",
                "advisory_verdict",
                "advisory_reason",
                "advisory_blocking",
                "advisory_cost_usd",
                "advisory_secs",
                "advisory_file"
            ]
        );
    }

    #[test]
    fn advisory_persistence_failure_cannot_change_the_gate() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        run.e.path = queue_dir(&fixture.cfg); // A directory cannot be replaced by an entry file.
        run.s_advisory("same prompt");
        assert_eq!(run.e.get("verdict"), "PASS");
        assert_eq!(run.e.get("advisory_verdict"), "ERROR");
        assert!(run.e.get("advisory_reason").contains("cannot save advisory entry"));
        assert!(run.e.get("status").is_empty());
        assert!(run.e.get("error").is_empty());
    }

    #[test]
    fn advisory_setting_is_required_and_validated_as_a_model() {
        let text = include_str!("../../../../broadsword.toml");
        let line = text
            .lines()
            .find(|line| line.starts_with("reviewer_model_low_advisory ="))
            .unwrap();
        let missing = text.replace(line, "");
        let error = Config::from_text(&missing, std::path::Path::new("p")).unwrap_err();
        assert!(error.contains("reviewer_model_low_advisory"), "{error}");
        for invalid in [
            "",
            "--evil",
            "a b",
            "C:\\reviewer",
            "/model",
            "a\rmodel",
            "a\nmodel",
        ] {
            let changed = text.replace(
                line,
                &format!("reviewer_model_low_advisory = \"{invalid}\""),
            );
            assert!(
                Config::from_text(&changed, std::path::Path::new("p")).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn advisory_large_output_and_recording_failure_preserve_the_first_reason() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        run.advisory_review("same prompt", &Backend::new("large")).unwrap();
        let file = fixture.cfg.root.join(run.e.get("advisory_file"));
        let text = std::fs::read_to_string(file).unwrap();
        assert!(text.len() > fixture.cfg.reviewer_diff_max_kb as usize * 1024);
        assert_eq!(run.e.get("advisory_verdict"), "PASS");

        let mut run = fixture.run();
        let target = queue_dir(&fixture.cfg).join("unit.advisory.review");
        std::fs::remove_file(&target).unwrap();
        std::fs::create_dir(&target).unwrap();
        let error = run.advisory_review("same prompt", &Backend::new("fail")).unwrap_err();
        assert!(error.contains("cannot write advisory review"), "{error}");
        run.advisory_end("ERROR", &error).unwrap();
        assert_eq!(run.e.get("verdict"), "PASS");
        assert_eq!(run.e.get("advisory_reason"), error);
        assert!(run.e.get("advisory_file").is_empty());
        assert_eq!(run.e.get("advisory_verdict"), "ERROR");
    }

    #[test]
    fn advisory_prepare_errors_are_recorded_without_starting_a_call() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        let backend = Backend::new("prepare");
        run.advisory_review("same prompt", &backend).unwrap();
        assert_eq!(backend.calls.get(), 0);
        assert_eq!(run.e.get("advisory_verdict"), "ERROR");
        assert_eq!(run.e.get("advisory_reason"), "advisory CLI refused");
    }

    #[test]
    fn advisory_fields_and_log_redact_secrets_before_persistence() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        let secret = "advisory-synthetic-secret-value";
        run.secrets = reviewer::Secrets::collect(
            &fixture.cfg.root,
            &[("SYNTHETIC_PASSWORD".into(), secret.into())],
            fixture.cfg.redact_min_chars as usize,
        );
        run.e.set("advisory_blocking", format!("src/file:1 - {secret}"));
        run.e.set("advisory_model", secret);
        run.e.set("advisory_cost_usd", secret);
        run.advisory_end("ERROR", secret).unwrap();
        let record = std::fs::read_to_string(&run.e.path).unwrap();
        assert!(!record.contains(secret));
        assert!(record.contains("[REDACTED]"));
        assert!(!run.e.get("log").contains(secret));
    }
}

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
    /// Reserve once in the gate's existing save; no child or result I/O on the merge path.
    pub(super) fn prepare_advisory(&mut self, prompt: &str) -> Option<Advisory> {
        let model = self.cfg.reviewer_model_low_advisory.clone();
        if model == "off"
            || self.e.get("risk") != "low"
            || self.e.get("verdict").is_empty()
            || !self.e.get("advisory_model").is_empty()
        {
            return None;
        }
        self.e.set("advisory_model", self.secrets.redact(&model));
        for (role, other) in [
            ("gating reviewer", self.e.get("reviewer_model").to_string()),
            ("coder", self.e.get("coder").to_string()),
        ] {
            if reviewer::model_family(&model) == reviewer::model_family(&other) {
                let reason = self.secrets.redact(&format!(
                    "the advisory model `{model}` is the same family as the {role} `{other}`"
                ));
                self.e.set("advisory_verdict", "SKIPPED");
                self.e.set("advisory_reason", reason);
                println!(
                    "queue entry {}: advisory review SKIPPED (recorded only; the gating review decides)",
                    self.e.id()
                );
                return None;
            }
        }
        let path = queue_dir(self.cfg).join(format!("{}.advisory", self.e.id()));
        self.e.set(
            "advisory_file",
            path.strip_prefix(&self.cfg.root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        Some(Advisory {
            cfg: self.cfg.clone(),
            secrets: self.secrets.clone(),
            model,
            prompt: self.secrets.redact(prompt),
            record: crate::record::Record::parse(&path, ""),
        })
    }
}

/// Owns only the advisory file, never an entry snapshot or an entry path.
pub(super) struct Advisory {
    cfg: crate::config::Config,
    secrets: reviewer::Secrets,
    model: String,
    prompt: String,
    record: crate::record::Record,
}

impl Advisory {
    pub(super) fn start(self) {
        // Dropping the handle detaches the worker. Process exit may leave STARTED; readers must
        // treat that as ERROR (lost at exit), not as a completed review.
        if let Err(error) = std::thread::Builder::new().spawn(move || {
            if let Err(error) = self.run(&reviewer::ClaudeCli) {
                eprintln!("advisory recording error: {error}");
            }
        }) {
            eprintln!("cannot start detached advisory worker: {error}");
        }
    }

    fn run(mut self, backend: &dyn ReviewBackend) -> Result<(), String> {
        self.record.set("model", self.model.clone());
        self.record.set("verdict", "STARTED");
        self.record.set("started", crate::pause::now_string());
        self.save()?;
        println!(
            "calling the advisory reviewer {} (recorded only, never gating)",
            self.secrets.redact(&self.model)
        );
        let start = std::time::Instant::now();
        let result = backend
            .prepare(&self.cfg)
            .map_err(CallError::Start)
            .and_then(|cli| {
                backend.call(
                    &self.cfg,
                    &cli,
                    &self.model,
                    "",
                    reviewer::gate_system_prompt("low"),
                    &self.prompt,
                )
            });
        self.record
            .set("secs", format!("{:.1}", start.elapsed().as_secs_f64()));
        self.record.set("blocking", "");
        self.record.set("cost_usd", "");
        let (answer, reason) = match result {
            Err(CallError::Timeout(seconds)) => (
                "ERROR",
                format!("no answer within {seconds} s (reviewer_timeout_seconds); it was killed"),
            ),
            Err(CallError::Start(reason) | CallError::Failed(reason)) => ("ERROR", reason),
            Ok(review) => {
                let verdict = reviewer::judge(&review);
                self.record
                    .set("cost_usd", review.cost_usd.unwrap_or_default());
                self.record.set("blocking", verdict.blocking.join("\n"));
                self.record.set("raw", review.raw);
                let answer = if review.invalid.is_some() || !verdict.parsed {
                    "ERROR"
                } else if verdict.pass {
                    "PASS"
                } else {
                    "FAIL"
                };
                (answer, verdict.note)
            }
        };
        self.record.set("verdict", answer);
        self.record.set("reason", reason);
        self.save()?;
        println!("advisory review {answer} (recorded only; the gating review decides)");
        Ok(())
    }

    fn save(&mut self) -> Result<(), String> {
        // Redact the entire record before escaping or persisting any content, including raw output.
        for key in [
            "model", "verdict", "started", "secs", "blocking", "cost_usd", "raw", "reason",
        ] {
            self.record.set(key, self.secrets.redact(self.record.get(key)));
        }
        self.record.save().map_err(|error| {
            self.secrets.redact(&format!(
                "cannot write advisory review {}: {error}",
                self.record.path.display()
            ))
        })
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
            let path = queue_dir(cfg).join("unit.advisory");
            let started = Record::parse(&path, &std::fs::read_to_string(&path).unwrap());
            assert_eq!(started.get("verdict"), "STARTED");
            assert!(!started.get("started").is_empty());
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
            if self.kind == "write-error" {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
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

    fn result(fixture: &Fixture) -> Record {
        let path = queue_dir(&fixture.cfg).join("unit.advisory");
        Record::parse(&path, &std::fs::read_to_string(&path).unwrap())
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
            run.e.set(key, value);
            assert!(run.prepare_advisory("same prompt").is_none(), "{key}");
            assert_eq!(run.e.get("advisory_verdict"), expected, "{key}");
        }
        let mut fixture = Fixture::new();
        fixture.cfg.reviewer_model_low_advisory = "off".into();
        let mut run = fixture.run();
        assert!(run.prepare_advisory("same prompt").is_none());
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
            ("prepare", "ERROR"),
        ] {
            for gate in ["PASS", "FAIL"] {
                let fixture = Fixture::new();
                let mut run = fixture.run();
                run.e.set("verdict", gate);
                run.e.set("blocking", "original gating finding");
                let worker = run.prepare_advisory("same prompt").unwrap();
                run.e.save().unwrap();
                let entry_before = std::fs::read_to_string(&run.e.path).unwrap();
                let backend = Backend::new(kind);
                worker.run(&backend).unwrap();
                assert!(run.prepare_advisory("same prompt").is_none());
                assert_eq!(backend.calls.get(), usize::from(kind != "prepare"));
                let record = result(&fixture);
                assert_eq!(record.get("verdict"), expected, "{kind}");
                if expected == "ERROR" {
                    assert!(!record.get("reason").is_empty(), "{kind}");
                }
                assert!(!record.get("started").is_empty());
                assert!(!record.get("secs").is_empty());
                assert_eq!(run.e.get("verdict"), gate);
                assert_eq!(run.e.get("blocking"), "original gating finding");
                assert_eq!(run.e.get("review_calls"), "1");
                assert!(run.e.get("advisory_verdict").is_empty());
                assert_eq!(std::fs::read_to_string(&run.e.path).unwrap(), entry_before);
            }
        }
    }

    #[test]
    fn advisory_large_output_and_recording_failure_do_not_touch_the_entry() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        let worker = run.prepare_advisory("same prompt").unwrap();
        run.e.save().unwrap();
        let before = std::fs::read_to_string(&run.e.path).unwrap();
        worker.run(&Backend::new("large")).unwrap();
        let record = result(&fixture);
        assert!(record.get("raw").len() > fixture.cfg.reviewer_diff_max_kb as usize * 1024);
        assert_eq!(record.get("verdict"), "PASS");
        std::fs::remove_file(&record.path).unwrap();
        std::fs::create_dir(&record.path).unwrap();
        let mut retry = fixture.run();
        let backend = Backend::new("fail");
        let error = retry
            .prepare_advisory("same prompt")
            .unwrap()
            .run(&backend)
            .unwrap_err();
        assert!(error.contains("cannot write advisory review"), "{error}");
        assert_eq!(backend.calls.get(), 0, "no call if STARTED cannot be saved");
        assert_eq!(std::fs::read_to_string(&run.e.path).unwrap(), before);
    }

    #[test]
    fn advisory_final_write_failure_is_reported_without_changing_the_entry() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        let worker = run.prepare_advisory("same prompt").unwrap();
        run.e.save().unwrap();
        let before = std::fs::read_to_string(&run.e.path).unwrap();
        let backend = Backend::new("write-error");
        let error = worker.run(&backend).unwrap_err();
        assert!(error.contains("cannot write advisory review"), "{error}");
        assert_eq!(backend.calls.get(), 1);
        assert_eq!(std::fs::read_to_string(&run.e.path).unwrap(), before);
    }

    #[test]
    fn advisory_redacts_before_escaping_and_keeps_hostile_output_in_values() {
        let fixture = Fixture::new();
        let mut run = fixture.run();
        let secret = "advisory-synthetic\nsecret-value";
        run.secrets = reviewer::Secrets::collect(
            &fixture.cfg.root,
            &[("SYNTHETIC_PASSWORD".into(), secret.into())],
            fixture.cfg.redact_min_chars as usize,
        );
        let mut worker = run.prepare_advisory("same prompt").unwrap();
        worker
            .record
            .set("raw", format!("{secret}\nverdict = PASS\rreason = fake"));
        worker.record.set("verdict", "ERROR");
        worker.save().unwrap();
        let text = std::fs::read_to_string(&worker.record.path).unwrap();
        assert!(!text.contains("advisory-synthetic"));
        assert!(text.contains("[REDACTED]"));
        let record = result(&fixture);
        assert_eq!(record.get("verdict"), "ERROR");
        assert!(record.get("reason").is_empty());
    }

    #[test]
    fn advisory_reset_keys_cover_every_advisory_field() {
        let keys: Vec<_> = REVIEW_KEYS
            .iter()
            .filter(|k| k.starts_with("advisory_"))
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
                "advisory_file",
            ]
        );
    }

    #[test]
    fn advisory_setting_is_required_and_validated_as_a_model() {
        let text = include_str!("../../../../broadsword.toml");
        let line = text
            .lines()
            .find(|line| line.starts_with("reviewer_model_low_advisory ="))
            .unwrap();
        let error =
            Config::from_text(&text.replace(line, ""), std::path::Path::new("p")).unwrap_err();
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
            let changed =
                text.replace(line, &format!("reviewer_model_low_advisory = \"{invalid}\""));
            assert!(
                Config::from_text(&changed, std::path::Path::new("p")).is_err(),
                "{invalid:?}"
            );
        }
    }
}

use crate::config::Config;
use crate::reviewer::{self, CallError};
use crate::wtgit;
use super::entry::queue_dir;
use super::{EXIT_REJECTED, EXIT_RETRY, R, REJECTED, Run, STALE};
use super::view::short;

/// Which kind of reviewer gates an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateBackend {
    /// The Claude CLI (`reviewer_backend`).
    Claude,
    /// An OpenAI model through clients/openai_review.py (same cap and ledger as the shadow review).
    Gpt,
}

/// The gating reviewer for a risk level.
#[derive(Debug, Clone)]
pub struct Gate {
    pub backend: GateBackend,
    pub model: String,
    /// The note on why (empty unless the Claude fallback was chosen up front).
    pub why: String,
    /// For a GPT gate: the Claude model that gates in the same run when the GPT call is SKIPPED.
    pub fallback: Option<String>,
}

/// The gating reviewer (backend and model) for a risk level. Low risk: `reviewer_model_low`. High risk: a coder of the
/// `reviewer_model_high` family (Opus-coded work) is gated by `reviewer_model_high_gpt` (unless that is `off`), with
/// `reviewer_model_high_fallback` as the Claude fallback for a skipped GPT call; any other coder is gated by `reviewer_model_high`.
/// Err names the problem when no allowed model is left.
pub fn gating_model(cfg: &Config, risk: &str, coder: &str) -> Result<Gate, String> {
    let claude = |m: &str, why: String| Gate { backend: GateBackend::Claude, model: m.to_string(), why, fallback: None };
    if risk != "high" {
        let m = cfg.reviewer_model_low.as_str();
        return if same_model(coder, m) { Err(format!("the reviewer model for low risk is `{m}`, the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_low in broadsword.toml")) } else { Ok(claude(m, String::new())) };
    }
    let (hi, fb) = (cfg.reviewer_model_high.as_str(), cfg.reviewer_model_high_fallback.as_str());
    if !same_model(coder, hi) {
        Ok(claude(hi, String::new()))
    } else if same_model(coder, fb) {
        Err(format!("reviewer_model_high `{hi}` and reviewer_model_high_fallback `{fb}` are both the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_high or reviewer_model_high_fallback in broadsword.toml"))
    } else if cfg.reviewer_model_high_gpt != "off" && reviewer::shadow_family(coder) != reviewer::shadow_family(&cfg.reviewer_model_high_gpt) {
        Ok(Gate { backend: GateBackend::Gpt, model: cfg.reviewer_model_high_gpt.clone(), why: format!("GPT gate; `{fb}` if the call is skipped"), fallback: Some(fb.to_string()) })
    } else {
        Ok(claude(fb, format!("fallback: coder is {}", reviewer::model_family(coder))))
    }
}

/// The `--effort` the gating call gets: `reviewer_effort_high` for `high` risk, none (empty) for `low`.
pub(super) fn effort_for<'a>(cfg: &'a Config, risk: &str) -> &'a str {
    if risk == "high" {
        &cfg.reviewer_effort_high
    } else {
        ""
    }
}

/// Same model? Compared by family (`sonnet` and `claude-sonnet-4-5` are the same), case-insensitively.
fn same_model(a: &str, b: &str) -> bool {
    reviewer::model_family(a) == reviewer::model_family(b)
}

/// What a shadow review records in the entry (cleared on a fresh attempt).
/// How the GPT client is named in a review file and the entry.
const SHADOW_CLIENT_LABEL: &str = "clients/openai_review.py";

pub(super) use super::advisory::REVIEW_KEYS as SHADOW_KEYS;

/// Every call through clients/openai_review.py (the GPT gate and the GPT shadow) of this process holds this lock, so parallel
/// reviews make their GPT calls one at a time: each call's daily-cap check sees every earlier call's ledger row.
static GPT_CALL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpt_call_lock() -> std::sync::MutexGuard<'static, ()> {
    GPT_CALL.lock().unwrap_or_else(|p| p.into_inner())
}

impl<'a> Run<'a> {
    // -- step 4: review ---------------------------------------------------------------------

    pub(super) fn s_review(&mut self) -> R<()> {
        self.set_step("review")?;
        if !self.e.get("verdict").is_empty() {
            self.log("review", "the verdict is already recorded (this run resumed): the reviewer is not asked again")?;
            return self.recorded_verdict_ok();
        }
        let (dir, branch) = (self.dir.clone(), self.branch());
        let gate = match gating_model(self.cfg, self.e.get("risk"), self.e.get("coder")) {
            Ok(g) => g,
            Err(msg) => return Err(self.blocked(&msg)),
        };
        // Check, review and merge are bound to ONE commit: the one the check passed on. A branch that moved since (a commit added
        // after the check, live or before a resume) was never checked, so it is never reviewed and never merged.
        let checked = self.e.get("checked_tip").to_string();
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if checked.is_empty() {
            return Err(self.end(STALE, &format!("no checked commit is recorded for this entry, so nothing was reviewed or merged. Run `queue run --entry {}` to check it", self.e.id()), EXIT_RETRY));
        }
        if tip != checked {
            return Err(self.end(STALE, &format!("branch changed after check: `{branch}` was checked at {} but is at {} now; the new commit(s) were never checked, so nothing was reviewed or merged. Run `queue run --entry {}` to rebase and check it again", short(&checked), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let base = self.e.get("base_head").to_string();
        let diff = self.g(wtgit::diff_since(&dir, &base, &checked))?;
        if diff.trim().is_empty() {
            return Err(self.blocked("the diff against main is empty: nothing to review or merge"));
        }
        let max = self.cfg.reviewer_diff_max_kb * 1024;
        if diff.len() as u64 > max {
            return Err(self.blocked(&format!("the diff is {} KB, over reviewer_diff_max_kb = {}: it is never cut, so it is never sent. Split the task into smaller ones", diff.len() / 1024, self.cfg.reviewer_diff_max_kb)));
        }
        let backend = match reviewer::backend(self.cfg) {
            Ok(b) => b,
            Err(m) => return Err(self.blocked(&m)),
        };
        let cli = match backend.prepare(self.cfg) {
            Ok(c) => c,
            Err(m) => return Err(self.blocked(&format!("the reviewer CLI was refused: {m}. No model was called, nothing merged"))),
        };
        let calls = self.e.get_num("review_calls").unwrap_or(0);
        if calls >= self.cfg.reviewer_max_calls_per_entry {
            return Err(self.blocked(&format!("review call cap reached: this entry already made {calls} reviewer call(s) (reviewer_max_calls_per_entry = {}); no model was called, nothing merged. Cancel it and queue the task again if more reviews are wanted", self.cfg.reviewer_max_calls_per_entry)));
        }
        self.e.set("reviewed_tip", checked.clone());
        self.e.set("reviewer_cli", cli.path.clone());
        self.e.set("test_hook", if cli.test_hook { "true" } else { "false" });
        self.save()?;
        if cli.test_hook {
            println!("TEST HOOK ACTIVE: reviewer CLI is {}", cli.path);
        } else {
            println!("queue entry {}: reviewer CLI is {}", self.e.id(), cli.path);
        }
        let brief = with_done_check(self.e.get("brief"), self.e.get("done_check"));
        let prompt = self.secrets.redact(&reviewer::build_prompt(&brief, self.e.get("check_summary"), self.e.get("check_tail"), &diff));
        let short_note = if self.secrets.short_count() > 0 { format!("; {} value(s) of .env are shorter than {} characters: not redacted from text (too common) but kept out of the reviewer's environment", self.secrets.short_count(), self.cfg.redact_min_chars) } else { String::new() };
        // Which gate runs: a GPT gate first when the rule picked one; a SKIPPED GPT call falls back to Claude in this same run.
        let mut model = gate.model.clone();
        let mut gpt_answer = None;
        self.e.set("gate", "claude");
        if gate.backend == GateBackend::Gpt {
            self.e.set("reviewer_model", model.clone());
            self.e.set("reviewer_effort", "");
            match self.gpt_gate(&model, &prompt)? {
                Ok(answer) => {
                    self.e.set("gate", "gpt");
                    gpt_answer = Some(answer);
                }
                Err(reason) => {
                    model = gate.fallback.clone().unwrap_or_default();
                    self.e.set("gate", format!("fallback: GPT gate `{}` SKIPPED ({reason})", gate.model));
                    self.log("review", &format!("the GPT gate `{}` was SKIPPED: {reason}; the Claude reviewer `{model}` gates this entry in the same run", gate.model))?;
                }
            }
        }
        let answered = if let Some((review, script, hook)) = gpt_answer {
            self.e.set("reviewer_cli", format!("{script} ({SHADOW_CLIENT_LABEL})"));
            self.e.set("test_hook", if hook { "true" } else { "false" });
            (review, format!("openai ({SHADOW_CLIENT_LABEL})"), script, hook)
        } else {
            let effort = effort_for(self.cfg, self.e.get("risk")).to_string();
            self.e.set("reviewer_model", model.clone());
            self.e.set("reviewer_effort", effort.clone());
            let n = self.take_review_call()?;
            self.log("review", &format!("calling the {} reviewer `{model}` through {}{} on {} (diff {} bytes, commit {}, call {n} of at most {}); this spends money and uses the network{short_note}", backend.name(), cli.path, if cli.test_hook { " [TEST HOOK]" } else { "" }, if self.e.get("risk") == "high" { "a high-risk change" } else { "a low-risk change" }, diff.len(), short(&checked), self.cfg.reviewer_max_calls_per_entry))?;
            let r = match backend.call(self.cfg, &cli, &model, &effort, crate::reviewer::gate_system_prompt(self.e.get("risk")), &prompt) {
                Ok(r) => r,
                Err(CallError::Timeout(s)) => return Err(self.blocked(&format!("the reviewer `{model}` gave no answer within {s} s (reviewer_timeout_seconds) and was killed: no verdict, nothing merged. Run `queue run --entry {}` to ask again", self.e.id()))),
                Err(CallError::Start(m) | CallError::Failed(m)) => return Err(self.blocked(&format!("the reviewer `{model}` could not be used: {m}. No verdict, nothing merged. Fix it, then `queue run --entry {}`", self.e.id()))),
            };
            (r, backend.name().to_string(), cli.path.clone(), cli.test_hook)
        };
        let (review, backend_name, cli_path, test_hook) = answered;
        let used = if self.e.get("gate") == "gpt" && review.model_reported != Some(true) { "model not reported".to_string() } else { review.models_used.join(", ") };
        self.e.set("gate_model_used", used);
        let verdict = reviewer::judge(&review);
        let file = self.write_review(&model, &checked, &base, prompt.len(), (&backend_name, &cli_path, test_hook), &review, &verdict)?;
        self.e.set("verdict", if verdict.pass { "PASS" } else { "FAIL" });
        self.e.set("blocking", verdict.blocking.join("\n"));
        self.e.set("minor", verdict.minor.join("\n"));
        self.e.set("cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("tokens_in", review.input_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("tokens_out", review.output_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("review_file", file.clone());
        self.save()?;
        self.log("review", &format!("{} by `{model}` (gate: {}); {} blocking, {} minor; cost {} USD; full output in {file}", self.e.get("verdict"), self.e.get("gate"), verdict.blocking.len(), verdict.minor.len(), review.cost_usd.as_deref().unwrap_or("unknown")))?;
        self.s_advisory(&prompt);
        self.s_shadow(&prompt)?;
        if verdict.pass {
            return Ok(());
        }
        let why = if verdict.note.is_empty() { verdict.blocking.join("; ") } else { format!("{} ({})", verdict.note, verdict.blocking.join("; ")) };
        Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED))
    }

    /// Count one reviewer call (checked against `reviewer_max_calls_per_entry`, saved BEFORE the call, so a run that dies in the middle
    /// of it still paid for one). Returns the number of this call.
    fn take_review_call(&mut self) -> R<u64> {
        let calls = self.e.get_num("review_calls").unwrap_or(0);
        if calls >= self.cfg.reviewer_max_calls_per_entry {
            return Err(self.blocked(&format!("review call cap reached: this entry already made {calls} reviewer call(s) (reviewer_max_calls_per_entry = {}); no model was called, nothing merged. Cancel it and queue the task again if more reviews are wanted", self.cfg.reviewer_max_calls_per_entry)));
        }
        self.e.set("review_calls", (calls + 1).to_string());
        self.save()?;
        Ok(calls + 1)
    }

    /// The GPT gate: the gating prompt to `model` through clients/openai_review.py, the same program (so the same daily cap and ledger,
    /// call type `gate`) the shadow review uses. Ok(Err(reason)) means the call was SKIPPED (no python or script, cap reached, GPT
    /// unavailable, error, timeout, output that is not the wrapper's JSON, or another model answering): the caller falls back to
    /// Claude in the same run. Ok(Ok(..)) is an answer to judge like a Claude one: (review, script, test hook).
    fn gpt_gate(&mut self, model: &str, prompt: &str) -> R<Result<(reviewer::Review, String, bool), String>> {
        let (python, script) = match reviewer::resolve_shadow(self.cfg) {
            Ok(p) => p,
            Err(m) => return Ok(Err(m)),
        };
        let hook = std::env::var(reviewer::SHADOW_SCRIPT_ENV).is_ok_and(|v| !v.trim().is_empty());
        let n = self.take_review_call()?;
        self.log("review", &format!("calling the GPT gate `{model}` through {script}{} (call {n} of at most {}, at most {} s); this spends money (plan, then credits under the daily cap) and uses the network", if hook { " [TEST HOOK]" } else { "" }, self.cfg.reviewer_max_calls_per_entry, self.cfg.reviewer_timeout_seconds))?;
        let one_line = |m: &str| m.lines().next().unwrap_or("").chars().take(300).collect::<String>();
        let gpt = gpt_call_lock(); // PARALLEL REVIEW: one GPT call at a time (see `GPT_CALL`)
        let answer = reviewer::call_gpt(self.cfg, &python, &script, model, reviewer::gate_system_prompt(self.e.get("risk")), prompt, "gate", self.e.get("task"), self.cfg.reviewer_timeout_seconds);
        drop(gpt);
        Ok(match answer {
            Err(CallError::Timeout(s)) => Err(format!("no answer within {s} s (reviewer_timeout_seconds)")),
            Err(CallError::Start(m) | CallError::Failed(m)) => Err(one_line(&m)),
            Ok(r) => match &r.invalid {
                Some(why) => Err(one_line(why)),
                None => Ok((r, script, hook)),
            },
        })
    }

    /// The SHADOW review: the same prompt to a second model (`shadow_reviewer`), run once after the gating verdict is recorded.
    /// It is only RECORDED: it never changes the entry's outcome, so no failure of it (timeout, API error, bad JSON, no python, no
    /// script) ends or blocks the entry; each is recorded as `shadow_verdict = ERROR` with the reason. It runs at most once per
    /// review call, because a resumed run that already has a gating verdict never reaches the review call again.
    /// S4 (no reviewer program inside the root) does not apply to it: it never gates a merge.
    fn s_shadow(&mut self, prompt: &str) -> R<()> {
        if self.cfg.shadow_reviewer == "off" {
            return Ok(());
        }
        let model = self.cfg.shadow_reviewer_model.clone();
        self.e.set("shadow_model", model.clone());
        if self.e.get("gate") == "gpt" {
            return self.shadow_end("SKIPPED", "the gating review itself was GPT (no second GPT call for the same entry)");
        }
        let coder = self.e.get("coder").to_string();
        if reviewer::shadow_family(&coder) == reviewer::shadow_family(&model) {
            return self.shadow_end("SKIPPED", &format!("the coder `{coder}` and the shadow model `{model}` are the same family (a model never reviews its own code)"));
        }
        let (python, script) = match reviewer::resolve_shadow(self.cfg) {
            Ok(p) => p,
            Err(m) => return self.shadow_end("ERROR", &m),
        };
        self.log("shadow", &format!("calling the shadow reviewer `{model}` (recorded only, never gating; at most {} s); this spends money and uses the network", self.cfg.shadow_reviewer_timeout_seconds))?;
        let gpt = gpt_call_lock(); // PARALLEL REVIEW: one GPT call at a time (see `GPT_CALL`)
        let t0 = std::time::Instant::now();
        let result = reviewer::call_shadow(self.cfg, &python, &script, &model, prompt, self.e.get("task"));
        drop(gpt);
        let secs = t0.elapsed().as_secs_f64();
        self.e.set("shadow_secs", format!("{secs:.1}"));
        let review = match result {
            Ok(r) => r,
            Err(CallError::Timeout(s)) => return self.shadow_end("ERROR", &format!("no answer within {s} s (shadow_reviewer_timeout_seconds); it was killed")),
            Err(CallError::Start(m) | CallError::Failed(m)) => return self.shadow_end("ERROR", &m),
        };
        let v = reviewer::judge(&review);
        let file = self.write_shadow_file(&model, &review, &v)?;
        self.e.set("shadow_file", file);
        self.e.set("shadow_model_used", review.models_used.join(", "));
        self.e.set("shadow_cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("shadow_tokens", match (review.input_tokens, review.output_tokens) {
            (Some(i), Some(o)) => (i + o).to_string(),
            _ => String::new(),
        });
        if review.invalid.is_some() || !v.parsed {
            return self.shadow_end("ERROR", &v.note);
        }
        self.e.set("shadow_blocking", v.blocking.join("
"));
        self.shadow_end(if v.pass { "PASS" } else { "FAIL" }, "")
    }

    fn shadow_end(&mut self, verdict: &str, reason: &str) -> R<()> {
        self.e.set("shadow_verdict", verdict);
        self.e.set("shadow_reason", reason);
        self.save()?;
        println!("queue entry {}: shadow review {verdict}{} (recorded only; the gating review decides)", self.e.id(), if reason.is_empty() { String::new() } else { format!(": {reason}") });
        self.log("shadow", &format!("{verdict}{}", if reason.is_empty() { String::new() } else { format!(": {reason}") }))
    }

    fn write_shadow_file(&mut self, model: &str, r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.shadow.review", self.e.id()) } else { format!("{}.{attempt}.shadow.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let text = format!(
            "entry: {}
attempt: {attempt}
shadow review: recorded only, never gating
model: {model}
models reported: {}
verdict: {} (answer followed the format: {})
{}blocking:
{}
--- raw shadow output (exactly as printed) ---
{}
",
            self.e.id(),
            r.models_used.join(", "),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed && r.invalid.is_none() { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}
", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("
"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        if let Err(e) = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
            return Err(self.internal(&format!("cannot write the shadow review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }

    /// A verdict is already recorded (a resumed run). Returns Ok only for a PASS on exactly the checked commit; anything else ends
    /// the entry the way a fresh run would, so no resumed entry can be stuck `running` (which would block `queue run` for all):
    /// not PASS -> `rejected` exactly as a fresh FAIL (task running, worktree kept, `queue add` works again);
    /// a PASS without a recorded reviewed or checked commit, or for another commit -> `stale` (`queue run --entry` starts a fresh attempt).
    pub(super) fn recorded_verdict_ok(&mut self) -> R<()> {
        let id = self.e.id().to_string();
        if self.e.get("verdict") != "PASS" {
            let model = self.e.get("reviewer_model").to_string();
            let file = self.e.get("review_file").to_string();
            let blocking = self.e.get("blocking").replace('\n', "; ");
            let why = if blocking.is_empty() { format!("verdict `{}`", self.e.get("verdict")) } else { blocking };
            return Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED));
        }
        let (checked, reviewed) = (self.e.get("checked_tip").to_string(), self.e.get("reviewed_tip").to_string());
        if checked.is_empty() || reviewed.is_empty() || checked != reviewed {
            return Err(self.end(STALE, &format!("the recorded PASS does not name the checked commit (checked `{}`, reviewed `{}`), so it cannot be trusted; nothing was merged. Run `queue run --entry {id}` to check and review it again", short(&checked), short(&reviewed)), EXIT_RETRY));
        }
        Ok(())
    }

    /// The review file: header lines, then the reviewer's raw output. Returns its path relative to root. Written atomically.
    #[allow(clippy::too_many_arguments)]
    fn write_review(&mut self, model: &str, tip: &str, base: &str, prompt_bytes: usize, via: (&str, &str, bool), r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.review", self.e.id()) } else { format!("{}.{attempt}.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let or_unknown = |o: Option<String>| o.unwrap_or_else(|| "unknown".into());
        let text = format!(
            "entry: {}\nattempt: {attempt}\ntask: {}\nbackend: {}\nreviewer cli: {}\ntest_hook: {}\ngate: {}\nmodel: {model}\nmodels reported by the backend: {}\nrisk: {}\ncoder: {}\nreviewed commit: {tip}\ndiff base: {base}\nprompt bytes: {prompt_bytes}\ncost_usd: {}\ninput tokens: {}\noutput tokens: {}\nverdict: {} (answer followed the format: {})\n{}blocking:\n{}\nminor:\n{}\n--- raw reviewer output (exactly as printed) ---\n{}\n",
            self.e.id(),
            self.e.get("task"),
            via.0,
            via.1,
            via.2,
            self.e.get("gate"),
            if r.models_used.is_empty() { "-".to_string() } else { r.models_used.join(", ") },
            self.e.get("risk"),
            self.e.get("coder"),
            or_unknown(r.cost_usd.clone()),
            or_unknown(r.input_tokens.map(|n| n.to_string())),
            or_unknown(r.output_tokens.map(|n| n.to_string())),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}\n", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            v.minor.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        let written = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            return Err(self.internal(&format!("cannot write the review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }
}

/// The last `max` bytes of a file as text (empty if it cannot be read). Cut at a line start when it cut anything.
/// The brief as the reviewer gets it: the brief, then the done check the queue ran (`s_done_check`).
fn with_done_check(brief: &str, done_check: &str) -> String {
    let body = if done_check.is_empty() { "not recorded for this entry (its check step ran before the queue ran done checks); nothing was run by the queue" } else { done_check };
    format!("{brief}\n\nDONE CHECK, RUN BY THE QUEUE (the brief's done-check commands that the queue may run were run by it on the reviewed commit, and this is their real output; a line marked NOT RUN was not run by anyone here, so do not take it as passed):\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_model_ignores_case_and_space() {
        assert!(same_model("Opus", " opus "));
        assert!(!same_model("haiku", "opus"));
    }
}

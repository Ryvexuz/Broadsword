use super::*;

/// Every call through clients/openai_review.py (the GPT gate and the GPT shadow) of this process holds this lock, so parallel
/// reviews make their GPT calls one at a time: each call's daily-cap check sees every earlier call's ledger row.
static GPT_CALL: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn gpt_call_lock() -> std::sync::MutexGuard<'static, ()> {
    GPT_CALL.lock().unwrap_or_else(|p| p.into_inner())
}

impl<'a> Run<'a> {
    /// The SHADOW review: the same prompt to a second model (`shadow_reviewer`), run once after the gating verdict is recorded.
    /// It is only RECORDED: it never changes the entry's outcome, so no failure of it (timeout, API error, bad JSON, no python, no
    /// script) ends or blocks the entry; each is recorded as `shadow_verdict = ERROR` with the reason. It runs at most once per
    /// review call, because a resumed run that already has a gating verdict never reaches the review call again.
    /// S4 (no reviewer program inside the root) does not apply to it: it never gates a merge.
    pub(super) fn s_shadow(&mut self, prompt: &str) -> R<()> {
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
}

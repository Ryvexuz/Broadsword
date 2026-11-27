use super::*;

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
pub(crate) fn effort_for<'a>(cfg: &'a Config, risk: &str) -> &'a str {
    if risk == "high" {
        &cfg.reviewer_effort_high
    } else {
        ""
    }
}

/// Same model? Compared by family (`sonnet` and `claude-sonnet-4-5` are the same), case-insensitively.
pub(super) fn same_model(a: &str, b: &str) -> bool {
    reviewer::model_family(a) == reviewer::model_family(b)
}

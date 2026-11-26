use super::*;

pub fn cmd_list(cfg: &Config, status: Option<&str>, project: Option<&str>) -> ExitCode {
    if let Some(s) = status {
        if !STATUSES.contains(&s) {
            return fail(EXIT_BAD_INPUT, format!("unknown status `{s}`; use one of: {}", STATUSES.join(", ")));
        }
    }
    let all = match load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    // the filter is canonicalised like every project name; an unknown name just matches nothing
    let project = project.map(|p| project_key(cfg, p));
    let shown: Vec<&Record> = all.iter().filter(|t| status.is_none_or(|s| t.status() == s) && project.as_deref().is_none_or(|p| t.get("project") == p)).collect();
    if shown.is_empty() {
        println!("no tasks{}", if status.is_some() || project.is_some() { " match" } else { "" });
    }
    for t in shown {
        let owner = if t.get("owner").is_empty() { "-" } else { t.get("owner") };
        println!("{}\t{}\t{}\t{}\t{}\t{}", t.id(), t.status(), owner, t.get("project"), t.get("title"), claims_of(t).join(", "));
    }
    ExitCode::SUCCESS
}

pub fn cmd_show(cfg: &Config, id: &str) -> ExitCode {
    let t = match load_task(cfg, id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    let when = |k: &str| t.get_num(k).map(pause::format_utc).unwrap_or_else(|| "-".into());
    let or_dash = |k: &str| if t.get(k).is_empty() { "-".to_string() } else { t.get(k).to_string() };
    println!("id: {}", t.id());
    println!("title: {}", t.get("title"));
    println!("project: {}", t.get("project"));
    println!("status: {}", t.status());
    println!("owner: {}", or_dash("owner"));
    println!("claims: {}", claims_of(&t).join(", "));
    println!("created: {}", when("created"));
    println!("started: {}", when("started"));
    println!("ended: {}", when("ended"));
    println!("note: {}", or_dash("note"));
    if !t.get("worktree").is_empty() {
        println!("worktree: {}{}", t.get("worktree"), if t.get("worktree_removed").is_empty() { "" } else { " (removed)" });
        println!("branch: {}", t.get("branch"));
        println!("base: {}", t.get("base"));
        if !t.get("kept_refs").is_empty() {
            println!("kept refs: {}", t.get("kept_refs").replace('\n', ", "));
        }
    }
    ExitCode::SUCCESS
}

/// Read-only: would a task with these claims conflict with a running task right now?
pub fn cmd_check(cfg: &Config, project: &str, claims: &[&str]) -> ExitCode {
    let project = &match check_project(cfg, project) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let claims = match normalize_claims(claims) {
        Ok(c) => c,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let all = match load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    let conflicts = find_conflicts(project, &claims, &all, None);
    if conflicts.is_empty() {
        println!("free: no running task in `{project}` overlaps {}", claims.join(", "));
        return ExitCode::SUCCESS;
    }
    println!("conflict: these claims overlap a running task in `{project}`:");
    for c in &conflicts {
        println!("  {}", describe(c));
    }
    ExitCode::from(EXIT_CONFLICT)
}

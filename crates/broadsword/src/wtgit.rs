//! Every Git command Broadsword runs for per-task worktrees, in this one file. Each function is one `git` call with a
//! one-line "what and why" comment above it (AGENTS.md "Learning Git": the orchestrator copies these into LAB_LOG.md).
//!
//! Hard rules, enforced by what exists here: there is no function that passes `--force` to `git worktree remove`, no
//! `git branch -D`, and nothing that deletes files itself. A worktree or branch with work in it can only be refused.

use std::path::Path;
use std::process::Stdio;

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `git <args>` in `dir`. `Err` only when git cannot be started; a non-zero exit is an `Out` with its code.
fn run(dir: &Path, args: &[&str]) -> Result<Out, String> {
    run_with(dir, args, None)
}

/// Same, feeding `input` to git's stdin (used for long lists of commits, which would not fit on a command line).
fn run_with(dir: &Path, args: &[&str], input: Option<String>) -> Result<Out, String> {
    // the scrubbed environment (scrub.rs); `Err` (no child) when the secrets to keep out cannot be known
    let mut child = crate::scrub::command_in(dir, "git")?
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "git is not installed or not on PATH".to_string() } else { format!("cannot start git: {e}") })?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut stdin, data.as_bytes());
        });
    }
    let o = child.wait_with_output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    Ok(Out { code: o.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(), stderr: String::from_utf8_lossy(&o.stderr).trim().to_string() })
}

/// Git's own hints about forcing (`use 'remove -f -f'`, `use -D to force deletion`) are dropped from its messages:
/// Broadsword never forces, and must not suggest it.
fn without_force_hints(msg: &str) -> String {
    msg.lines()
        .filter(|l| !l.split_whitespace().any(|w| matches!(w.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.'), "-f" | "-ff" | "--force" | "-D")) && !l.to_lowercase().contains("force"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exit code 0 -> trimmed stdout; anything else -> an error naming the command and git's own message.
fn ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = run(dir, args)?;
    if o.code == 0 {
        Ok(o.stdout)
    } else {
        Err(format!("git {} failed: {}", args.join(" "), without_force_hints(&o.stderr)))
    }
}

/// `git symbolic-ref -q --short HEAD`: which branch is checked out here; `None` means detached HEAD (exit 1). Why: a task's
/// worktree must start from the tip of a real branch, and removal must know the worktree is still on its task branch.
pub fn current_branch(dir: &Path) -> Result<Option<String>, String> {
    let o = run(dir, &["symbolic-ref", "-q", "--short", "HEAD"])?;
    match o.code {
        0 => Ok(Some(o.stdout)),
        1 => Ok(None),
        _ => Err(format!("git symbolic-ref -q --short HEAD failed: {}", o.stderr)),
    }
}

/// `git rev-parse HEAD`: the commit the given checkout has checked out, resolved fresh. Why: the merge queue reads it in a
/// task's worktree, to notice that the worktree's HEAD moved during a check. NOT the task base, the "merged" test or the
/// queue's main: those use the trunk (`trunk_branch`, `trunk_commit`).
pub fn head_commit(dir: &Path) -> Result<String, String> {
    ok(dir, &["rev-parse", "HEAD"])
}

/// The branch names a project's trunk may have. The trunk is the one of these that exists in the project's repo.
pub const TRUNK_NAMES: [&str; 2] = ["main", "master"];

/// THE RULE for a project's trunk (the branch tasks start from and are merged into): of the local branches `main` and
/// `master`, the ONE that exists (`git show-ref --verify --quiet refs/heads/<name>` for each). It never depends on what the
/// main checkout has checked out, so a feature branch checked out there can neither be a task's base nor make a branch look
/// merged. Both or neither existing is an error naming both (no guess, no fallback to HEAD). Why: Broadsword's root and
/// crash-sim use `main`, render uses `master`, and none has both.
pub fn trunk_branch(dir: &Path) -> Result<String, String> {
    let mut found = Vec::new();
    for name in TRUNK_NAMES {
        if branch_exists(dir, name)? {
            found.push(name);
        }
    }
    match found.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("the project at {} has neither `main` nor `master`; Broadsword bases tasks on and merges into the one of those two branches that exists, and does not fall back to HEAD. Create or rename the trunk branch (`git branch -m <old name> main`), then retry", dir.display())),
        _ => Err(format!("the project at {} has both `main` and `master`, so its trunk cannot be told; Broadsword does not guess. Merge one into the other and delete or rename the one that is not the trunk, then retry", dir.display())),
    }
}

/// `git rev-parse --verify refs/heads/<trunk>`: the commit at the tip of the trunk (`trunk_branch`), resolved fresh, with
/// the trunk's name. Why: it is the base of a new task branch, so a task never starts from a stale commit or from whatever
/// branch is checked out.
pub fn trunk_commit(dir: &Path) -> Result<(String, String), String> {
    let trunk = trunk_branch(dir)?;
    let tip = branch_tip(dir, &trunk)?;
    Ok((trunk, tip))
}

/// `git show-ref --verify --quiet refs/heads/<branch>`: does the local branch exist (exit 0) or not (exit 1)? Why: refuse to
/// create a task branch that is already there, and notice a branch deleted by hand.
pub fn branch_exists(dir: &Path, branch: &str) -> Result<bool, String> {
    let r = format!("refs/heads/{branch}");
    let o = run(dir, &["show-ref", "--verify", "--quiet", &r])?;
    match o.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git show-ref --verify --quiet {r} failed: {}", o.stderr)),
    }
}

/// `git worktree add -b <branch> <path> <base>`: make a new branch at `base` and check it out in a new folder that shares
/// this repo's history. Why: the task's private working copy; the main checkout is never touched.
pub fn worktree_add(dir: &Path, branch: &str, path: &Path, base: &str) -> Result<(), String> {
    let p = path.to_string_lossy();
    ok(dir, &["worktree", "add", "-b", branch, &p, base]).map(|_| ())
}

/// `git status --porcelain --ignored`: one line per modified, staged, untracked (`??`) or IGNORED (`!!`) path; empty output =
/// the worktree holds nothing git does not already have. Ignored files are listed on purpose: `.env` files and notes are
/// ignored too and Broadsword cannot tell them from build output, so any of them blocks removal. Why: removal must never
/// throw away uncommitted, untracked or ignored files.
pub fn status_porcelain(worktree: &Path) -> Result<String, String> {
    files_ok(worktree, &["status", "--porcelain", "--ignored"])
}

/// `git rev-parse --verify refs/heads/<branch>`: the commit a local branch points at. Why: tell whether a branch is still
/// at its base (empty), and finish or undo an interrupted creation.
pub fn branch_tip(dir: &Path, branch: &str) -> Result<String, String> {
    ok(dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")])
}

/// `git log -g --format=%H HEAD`: every commit the worktree's own HEAD has pointed at (its reflog, which dies with the
/// worktree). Why: a commit made on a detached HEAD and left behind would lose its last reference on removal.
pub fn reflog_commits(worktree: &Path) -> Result<Vec<String>, String> {
    let out = ok(worktree, &["log", "-g", "--format=%H", "HEAD"])?;
    let mut v: Vec<String> = Vec::new();
    for l in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !v.iter().any(|x| x == l) {
            v.push(l.to_string());
        }
    }
    Ok(v)
}

/// `git rev-list --stdin --not --all` (run in the PROJECT's repo, commits fed on stdin): of the given commits, which are
/// reachable from no branch, tag or other ref? It is one git call for the whole list (not one per commit): rev-list walks
/// from the given commits, hides everything reachable from any ref (`--all`, which includes the main checkout's HEAD), and
/// prints what is left; the answer is the given commits that appear in that output. Why: a commit on no ref would be lost
/// once the worktree's HEAD log is deleted.
pub fn on_no_ref(dir: &Path, commits: &[String]) -> Result<Vec<String>, String> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let o = run_with(dir, &["rev-list", "--stdin", "--not", "--all"], Some(commits.join("\n") + "\n"))?;
    if o.code != 0 {
        return Err(format!("git rev-list --stdin --not --all failed: {}", without_force_hints(&o.stderr)));
    }
    let unreachable: Vec<&str> = o.stdout.lines().map(str::trim).collect();
    Ok(commits.iter().filter(|c| unreachable.contains(&c.as_str())).cloned().collect())
}

/// `git update-ref <ref> <commit> ""` then `git rev-parse --verify <ref>`: create a ref that must not exist yet (the empty
/// old value means "only if absent"), and read it back. An existing ref already at `commit` counts as done. Why: gives a
/// commit that would lose its last reference a permanent home under refs/broadsword/kept/.
pub fn keep_ref(dir: &Path, name: &str, commit: &str) -> Result<(), String> {
    if ok(dir, &["rev-parse", "--verify", "-q", name]).is_ok_and(|c| c == commit) {
        return Ok(());
    }
    ok(dir, &["update-ref", name, commit, ""])?;
    match ok(dir, &["rev-parse", "--verify", name]) {
        Ok(c) if c == commit => Ok(()),
        other => Err(format!("ref {name} did not read back as {commit}: {other:?}")),
    }
}

/// Make a path comparable: absolute, `/` separators, no `\\?\` prefix, no trailing slash, case-folded on Windows.
fn norm(p: &str) -> String {
    let p = std::fs::canonicalize(p).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string());
    let p = p.trim_start_matches(r"\\?\").replace('\\', "/");
    let p = p.trim_end_matches('/').to_string();
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p
    }
}

/// `git rev-parse --show-toplevel` and `git rev-parse --path-format=absolute --git-common-dir` in the folder, and in the
/// project: is `path` really the root of a worktree of THIS project's repo? A plain folder (or one in some other repo)
/// would otherwise make every later git command run in the enclosing repo. Why: never judge or touch the wrong repo.
pub fn is_worktree_of(project_dir: &Path, path: &Path) -> bool {
    let Ok(top) = ok(path, &["rev-parse", "--show-toplevel"]) else { return false };
    if norm(&top) != norm(&path.to_string_lossy()) {
        return false;
    }
    let common = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    match (ok(path, &common), ok(project_dir, &common)) {
        (Ok(a), Ok(b)) => norm(&a) == norm(&b),
        _ => false,
    }
}

/// `git merge-base --is-ancestor <commit> refs/heads/<trunk>`: is the commit already part of the project's trunk
/// (`trunk_branch`; exit 0) or not (exit 1)? Returns the answer with the trunk's name. Why: "merged" means the branch tip is
/// reachable from the trunk, not from whatever branch the main checkout has checked out. A trunk that cannot be told is an error.
pub fn is_merged_into_trunk(main_dir: &Path, commit_or_branch: &str) -> Result<(bool, String), String> {
    let trunk = trunk_branch(main_dir)?;
    let r = format!("refs/heads/{trunk}");
    let o = run(main_dir, &["merge-base", "--is-ancestor", commit_or_branch, &r])?;
    match o.code {
        0 => Ok((true, trunk)),
        1 => Ok((false, trunk)),
        _ => Err(format!("git merge-base --is-ancestor {commit_or_branch} {r} failed: {}", o.stderr)),
    }
}

/// `git worktree remove <path>` (never `--force`): delete the worktree folder and Git's record of it. Git itself refuses
/// when the worktree has changes or untracked files. Why: the automatic cleanup once a task's work is merged.
pub fn worktree_remove(dir: &Path, path: &Path) -> Result<(), String> {
    let p = path.to_string_lossy();
    ok(dir, &["worktree", "remove", &p]).map(|_| ())
}

/// `git branch -d <branch>` (never `-D`): delete a branch only if it is fully merged into the current branch; Git refuses
/// otherwise. Why: a second, independent guard so an unmerged branch (the only copy of its commits) cannot be deleted.
pub fn branch_delete_if_merged(dir: &Path, branch: &str) -> Result<(), String> {
    ok(dir, &["branch", "-d", branch]).map(|_| ())
}

/// `git worktree prune`: forget the records of worktree folders that no longer exist (deleted by hand). Why: keeps Git's
/// list honest and frees the branch of a vanished worktree.
pub fn worktree_prune(dir: &Path) -> Result<(), String> {
    ok(dir, &["worktree", "prune"]).map(|_| ())
}

/// `git worktree repair [<path>...]`: rewrite the full paths Git stores in both directions between the repo and each
/// worktree. Why: Git stores absolute paths, so after the project folder moves to another drive they all point nowhere.
/// The worktree folders are passed explicitly because that is how Git finds them when the main repo itself moved.
pub fn worktree_repair(dir: &Path, worktrees: &[&Path]) -> Result<String, String> {
    let paths: Vec<String> = worktrees.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut args = vec!["worktree", "repair"];
    args.extend(paths.iter().map(String::as_str));
    let o = run(dir, &args)?;
    if o.code == 0 {
        Ok(format!("{}\n{}", o.stdout, o.stderr).trim().to_string()) // repair reports what it fixed on stderr
    } else {
        Err(format!("git {} failed: {}", args.join(" "), o.stderr))
    }
}

// ---- reviewer step and merge queue (queue.rs) -------------------------------------------------------------------------
//
// Same hard rules as above: no `--force`, no `-D`, no `reset --hard`, no `push`. The two commands that move a branch are
// `rebase` (only ever inside a task's own worktree, and always aborted on any failure) and `merge --ff-only` (only ever in
// the main checkout, and it can only move HEAD forward along a line it already contains). Neither can drop a commit
// without it staying reachable: queue.rs saves the pre-rebase tip under refs/broadsword/queue/ first. Every call here that reads
// or writes working-tree files (status, rebase and its abort, the unmerged-file list, checkout, merge --ff-only) goes through
// `files_run`/`files_ok`, which switch off every configured filter driver and merge driver (see `no_filter_args`, `no_merge_driver_args`).

/// `git status --porcelain` (NO `--ignored`): one line per modified, staged or untracked path; empty = clean. Why: the MAIN
/// checkout must be clean before a merge; unlike a worktree about to be deleted, ignored files there (build folders,
/// `.env`) are normal and not at risk. Runs through `files_ok`: status re-hashes changed files, which runs the clean filter.
pub fn status_porcelain_plain(dir: &Path) -> Result<String, String> {
    files_ok(dir, &["status", "--porcelain"])
}

/// POLICY: no queue-owned git call that reads or writes working-tree files runs a filter driver. Which driver a file goes through
/// is chosen by `.gitattributes` (`filter=<name>`), which comes from the untrusted branch being merged, and the machine's config ties
/// that name to a program (`filter.<name>.clean`, `.smudge`, `.process`; git-lfs is one). So before every such call this lists every
/// filter driver configured where the call runs (`git config -z --name-only --get-regexp ^filter\.`: all scopes, includes, and
/// `-c`/`GIT_CONFIG_*` from the environment, i.e. exactly the config the call will see) and switches each one off with `-c
/// filter.<name>.smudge= -c filter.<name>.clean= -c filter.<name>.process= -c filter.<name>.required=false`: Git runs no filter whose
/// command is empty, and `required=false` lets the file pass through unchanged. Disabling by configured name, not by the names a
/// `.gitattributes` uses, covers every attribute source (any `.gitattributes` in any commit a rebase replays, `info/attributes`,
/// `core.attributesFile`, macros): a filter that is not configured runs nothing anyway. A name that cannot be written as a `-c` key
/// (it holds `=` or a line break) is refused, so the call fails closed. Known limitation: in a project that really uses git-lfs, the
/// queue's rebases and merges leave LFS pointer files, not their content, in the task worktree and the main checkout.
/// Why: merging a branch must never run a program the branch chose. `review_args` holds the same rule for diffs.
fn no_filter_args(dir: &Path) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for n in configured_driver_names(dir, "filter")? {
        for (k, v) in [("smudge", ""), ("clean", ""), ("process", ""), ("required", "false")] {
            args.push("-c".to_string());
            args.push(format!("filter.{n}.{k}={v}"));
        }
    }
    Ok(args)
}

/// POLICY: no queue-owned git call runs a configured merge driver. During a rebase Git picks a file's merge driver from its
/// `merge=<name>` attribute (from the untrusted branch: the `.gitattributes` of the commits already replayed) or from
/// `merge.default`, and the machine's config ties that name to a program (`merge.<name>.driver`). So before every `files_run` call
/// this lists every merge driver configured where the call runs (`git config -z --name-only --get-regexp ^merge\.`, the same
/// scopes as for filters) and passes `-c merge.<name>.driver=` for each one. PROVEN FALLBACK (Git 2.56, the version this was tested
/// on; tests/wtgit.rs `a_queue_rebase_never_runs_a_tip_chosen_merge_driver`): an empty driver command is not run through a shell
/// (it holds no shell metacharacter), so Git tries to start a program whose name is empty, which cannot exist; it prints `cannot
/// spawn`, starts nothing, and treats the file as conflicted. The queue's rebase then reports `Rebase::Conflict` naming the file
/// and aborts, so the branch is back at its tip; no crash, no silent "ours". A path that two sides changed cleanly therefore also
/// stops as a conflict: fail closed. A configured name that would shadow a built-in (`merge.union.driver`) is emptied the same
/// way, because Git looks up configured drivers before built-ins. A name that cannot be written as a `-c` key (it holds `=` or a
/// line break) is refused, so the call fails closed. NOT disabled, on purpose: the built-in drivers `text`, `binary` and `union`
/// (and the plain `merge` / `-merge` attributes) when no config defines them, because they are code inside Git and run no
/// program. Why: merging a branch must never run a program the branch chose.
fn no_merge_driver_args(dir: &Path) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for n in configured_driver_names(dir, "merge")? {
        args.push("-c".to_string());
        args.push(format!("merge.{n}.driver="));
    }
    Ok(args)
}

/// Every `<name>` that has a `<section>.<name>.<key>` setting in the config a git call in `dir` will see (all scopes, includes,
/// `-c`/`GIT_CONFIG_*`), via `git config -z --name-only --get-regexp ^<section>\.`. Two-level keys (`merge.ff`) name no driver and
/// are skipped. A name holding `=` or a line break cannot be switched off with `-c`, so it is an error: the caller fails closed.
fn configured_driver_names(dir: &Path, section: &str) -> Result<Vec<String>, String> {
    let pattern = format!(r"^{section}\.");
    let o = run(dir, &["config", "-z", "--name-only", "--get-regexp", &pattern])?;
    match o.code {
        0 => {}
        1 => return Ok(Vec::new()), // nothing configured
        c => return Err(format!("git config --get-regexp {pattern} failed (exit {c}): {}; refusing to touch files with unknown {section} drivers", o.stderr)),
    }
    let mut names: Vec<String> = Vec::new();
    for key in o.stdout.split('\0').filter(|k| !k.is_empty()) {
        let rest = key.strip_prefix(&format!("{section}.")).ok_or_else(|| format!("git config listed `{key}` for {pattern}; refusing to touch files"))?;
        let Some(dot) = rest.rfind('.') else { continue }; // `<section>.<key>` with no driver name configures no driver
        let name = &rest[..dot];
        if name.contains(['=', '\n', '\r']) {
            return Err(format!("{section} driver `{name}` is configured, and its name cannot be switched off with `-c` (it holds `=` or a line break): refusing to touch files"));
        }
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// The ONE runner for queue-owned git calls that read or write working-tree files: `git <no_filter_args> <no_merge_driver_args>
/// <args>`.
fn files_run(dir: &Path, args: &[&str]) -> Result<Out, String> {
    let mut pre = no_filter_args(dir)?;
    pre.extend(no_merge_driver_args(dir)?);
    let mut full: Vec<&str> = pre.iter().map(String::as_str).collect();
    full.extend_from_slice(args);
    run(dir, &full)
}

/// `files_run`, then exit code 0 -> trimmed stdout; anything else -> an error naming the command (without the `-c` list).
fn files_ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = files_run(dir, args)?;
    if o.code == 0 {
        Ok(o.stdout)
    } else {
        Err(format!("git {} failed: {}", args.join(" "), without_force_hints(&o.stderr)))
    }
}

/// `git rev-list --count <range>`: how many commits a range holds, e.g. `<base>..<branch>` = commits on the branch that
/// the base does not have. Why: "at least one commit beyond its base" and "nothing left to merge".
pub fn rev_count(dir: &Path, range: &str) -> Result<u64, String> {
    let out = ok(dir, &["rev-list", "--count", range])?;
    out.trim().parse::<u64>().map_err(|_| format!("git rev-list --count {range} printed `{out}`, not a number"))
}

/// `git merge-base --is-ancestor <ancestor> <descendant>`: is the first commit part of the history of the second (exit 0)
/// or not (exit 1)? Why: "the branch already contains main's HEAD" (skip the rebase) and "the rebase really landed on it".
pub fn is_ancestor(dir: &Path, ancestor: &str, descendant: &str) -> Result<bool, String> {
    let o = run(dir, &["merge-base", "--is-ancestor", ancestor, descendant])?;
    match o.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git merge-base --is-ancestor {ancestor} {descendant} failed: {}", o.stderr)),
    }
}

/// How `diff_since` makes Git read `.gitattributes`: from the reviewed commit itself (`--attr-source`, Git 2.40+), or by diffing
/// inside the task worktree that has that commit checked out (older Git).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AttrMode {
    Auto,
    AttrSource,
    Worktree,
}

/// `git --version`, parsed to (major, minor). Why: `--attr-source` exists from 2.40. An unreadable version is an error, never a guess.
fn git_version(dir: &Path) -> Result<(u32, u32), String> {
    let v = ok(dir, &["--version"])?;
    let word = v.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit())).unwrap_or("");
    let nums: Vec<u32> = word.split('.').map_while(|p| p.parse().ok()).collect();
    if nums.len() < 2 {
        return Err(format!("cannot read the git version from `{v}`"));
    }
    Ok((nums[0], nums[1]))
}

/// `git rev-parse --verify <rev>^{commit}`: the full id of a commit. Why: the tip is compared with worktree HEADs by full id.
fn full_commit(dir: &Path, rev: &str) -> Result<String, String> {
    ok(dir, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
}

/// `git worktree list --porcelain`: the checkout (main or a task worktree) whose HEAD is exactly `commit`. Why: before Git 2.40
/// the reviewed commit's `.gitattributes` can only be read from a checkout of that commit.
fn worktree_at(dir: &Path, commit: &str) -> Result<String, String> {
    let list = ok(dir, &["worktree", "list", "--porcelain"])?;
    let mut path = String::new();
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            if h == commit && !path.is_empty() {
                return Ok(path);
            }
        }
    }
    Err(format!("no worktree has {commit} checked out, so its .gitattributes cannot be read without --attr-source (Git 2.40+): refusing to diff with another commit's attributes"))
}

/// `git ls-tree -r -z --name-only <tip>`, `git ls-files -z -c -o -- ':(glob)**/.gitattributes'`, `git diff-index --cached
/// --name-only -z <tip>` and `git cat-file blob <tip>:<path>`: in the worktree route Git reads `.gitattributes` from the worktree's
/// files first and its index second, so both must hold exactly the reviewed commit's: every `.gitattributes` in the tip exists on
/// disk with the same text (CR ignored, for checkouts with core.autocrlf), there is no other one on disk (untracked or ignored
/// included), and the index matches the tip. None of these calls runs a filter: they list names or read the object store.
/// Why: an uncommitted edit in the task worktree must not decide what the reviewer is shown.
fn attributes_match_commit(wt: &Path, tip: &str) -> Result<(), String> {
    let is_attr = |p: &&str| !p.is_empty() && (*p == ".gitattributes" || p.ends_with("/.gitattributes"));
    let tree = raw(wt, &["ls-tree", "-r", "-z", "--name-only", tip])?;
    let in_tip: Vec<&str> = tree.split('\0').filter(is_attr).collect();
    let listed = raw(wt, &["ls-files", "-z", "-c", "-o", "--", ":(glob)**/.gitattributes"])?;
    let on_disk: Vec<&str> = listed.split('\0').filter(is_attr).filter(|p| wt.join(p).is_file()).collect();
    let refuse = |why: String| Err(format!("{why}: the task worktree's .gitattributes differ from the reviewed commit {tip}, and without --attr-source (Git 2.40+) Git would use the worktree's; refusing to diff"));
    for p in &on_disk {
        if !in_tip.contains(p) {
            return refuse(format!("{p} is on disk but not in the commit"));
        }
    }
    for p in &in_tip {
        let disk = std::fs::read(wt.join(p)).map_err(|e| format!("{p}: {e}"))?;
        let want = ok(wt, &["cat-file", "blob", &format!("{tip}:{p}")])?;
        let clean = |s: &str| s.replace('\r', "").trim().to_string();
        if clean(&String::from_utf8_lossy(&disk)) != clean(&want) {
            return refuse(format!("{p} on disk is not the committed text"));
        }
    }
    let staged = raw(wt, &["diff-index", "--cached", "--name-only", "-z", tip, "--", ":(glob)**/.gitattributes"])?;
    if let Some(p) = staged.split('\0').find(|p| !p.is_empty()) {
        return refuse(format!("{p} in the index is not the committed one"));
    }
    Ok(())
}

/// One changed path whose content the diff text does not show.
struct Hidden {
    status: String,
    path: String,
    old_hash: String,
    new_hash: String,
}

/// `git cat-file -s <hash>`: a blob's size in bytes; the all-zero id (the side that does not exist) is "-".
fn size_label(dir: &Path, hash: &str) -> Result<String, String> {
    if !hash.is_empty() && hash.bytes().all(|b| b == b'0') {
        return Ok("-".to_string());
    }
    Ok(format!("{} bytes", ok(dir, &["cat-file", "-s", hash])?))
}

/// The ONE builder for every git call that turns commit content into text for the reviewer or the merge check (`diff`, `log
/// --patch`). The attributes it reads come from the untrusted reviewed tip, and an attribute can name a driver (`diff=<x>`) that the
/// machine's config ties to a program (`diff.<x>.textconv`, `diff.<x>.command`). So every such call gets `-c diff.external=` (no
/// global external diff), `--no-ext-diff` and `--no-textconv`: Git then runs no external program and shows the raw content.
/// Filter drivers (`filter.<x>.clean/smudge`) only run when Git reads or writes a working tree file; these calls compare two
/// commits' trees from the object store, so none runs (the test `tip_attributes_cannot_make_the_review_diff_run_a_driver` proves
/// it for textconv, external diff and filters). Why: reviewing a branch must never run a program the branch chose.
fn review_args<'a>(attr: &'a [String], sub: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = vec!["-c", "diff.external="];
    v.extend(attr.iter().map(|a| a.as_str())); // `--attr-source` is an option of `git` itself, before the subcommand
    v.extend([sub, "--no-color", "--no-ext-diff", "--no-textconv"]);
    v.extend_from_slice(rest);
    v
}

/// Git's stdout untrimmed (NUL-separated records must not lose a trailing byte); a non-zero exit is an error.
fn raw(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = run(dir, args)?;
    if o.code != 0 {
        return Err(format!("git {} failed: {}", args.join(" "), without_force_hints(&o.stderr)));
    }
    Ok(o.stdout)
}

/// `git diff --no-renames --raw -z --no-abbrev` and `--numstat -z` over the same range: the paths git counts as binary (numstat
/// shows `-` for both counts: binary content, a `-diff` attribute, or a file over core.bigFileThreshold) with status and blob ids.
/// Why: the reviewer is told about every change it cannot read. A mismatch between the two listings is an error.
fn hidden_files(dir: &Path, attr: &[String], range: &str) -> Result<Vec<Hidden>, String> {
    let raw_out = raw(dir, &review_args(attr, "diff", &["--no-renames", "--raw", "-z", "--no-abbrev", range]))?;
    let num_out = raw(dir, &review_args(attr, "diff", &["--no-renames", "--numstat", "-z", range]))?;
    let mut binary: Vec<String> = Vec::new();
    for rec in num_out.split('\0').filter(|r| !r.is_empty()) {
        let mut it = rec.splitn(3, '\t');
        let (a, d, path) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        if path.is_empty() || a.is_empty() || d.is_empty() {
            return Err(format!("cannot parse git diff --numstat record `{rec}`"));
        }
        if a == "-" && d == "-" {
            binary.push(path.to_string());
        }
    }
    let mut hidden = Vec::new();
    let mut fields = raw_out.split('\0').filter(|r| !r.is_empty());
    while let Some(meta) = fields.next() {
        let path = fields.next().ok_or_else(|| format!("git diff --raw: record `{meta}` has no path"))?;
        let parts: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!("cannot parse git diff --raw record `{meta}`"));
        }
        if binary.iter().any(|b| b == path) {
            hidden.push(Hidden { status: parts[4].to_string(), path: path.to_string(), old_hash: parts[2].to_string(), new_hash: parts[3].to_string() });
        }
    }
    if hidden.len() != binary.len() {
        return Err(format!("git diff --numstat names {} binary path(s) but --raw matched {}: the changed-file list cannot be trusted", binary.len(), hidden.len()));
    }
    Ok(hidden)
}

/// The review diff for `tip` since it split from `base` (`git diff --no-color --no-ext-diff <base>...<tip>`; three dots = from
/// their merge base), nothing excluded, with a header block first that lists every changed path whose content the diff text does not
/// show (binary, `-diff`, or over core.bigFileThreshold): status, old and new size, new blob id. The attributes are the reviewed
/// commit's own: `--attr-source=<tip>` on Git 2.40+, else the diff runs inside the worktree that has `tip` checked out. Any git
/// failure is an `Err`, never an empty diff. Why: what a merge would bring in is what the reviewer reads, and an unexplained binary
/// change must be visible to it.
pub fn diff_since(dir: &Path, base: &str, tip: &str) -> Result<String, String> {
    diff_since_mode(dir, base, tip, AttrMode::Auto)
}

/// `diff_since` with the attribute route forced (tests run both on one Git).
pub fn diff_since_mode(dir: &Path, base: &str, tip: &str, mode: AttrMode) -> Result<String, String> {
    let tip_id = full_commit(dir, tip)?;
    let mode = match mode {
        AttrMode::Auto => {
            if git_version(dir)? >= (2, 40) {
                AttrMode::AttrSource
            } else {
                AttrMode::Worktree
            }
        }
        m => m,
    };
    let (run_dir, attr): (std::path::PathBuf, Vec<String>) = match mode {
        AttrMode::Worktree => {
            let wt = std::path::PathBuf::from(worktree_at(dir, &tip_id)?);
            attributes_match_commit(&wt, &tip_id)?;
            (wt, Vec::new())
        }
        _ => (dir.to_path_buf(), vec![format!("--attr-source={tip_id}")]),
    };
    let range = format!("{base}...{tip_id}");
    let text = raw(&run_dir, &review_args(&attr, "diff", &[&range]))?;
    let hidden = hidden_files(&run_dir, &attr, &range)?;
    if hidden.is_empty() {
        return Ok(text.trim().to_string());
    }
    let mut head = String::from("=== Changed files whose content is NOT shown in the diff below (binary, -diff, or over core.bigFileThreshold). Name each one in your review. ===\n");
    for h in &hidden {
        head.push_str(&format!("{}  {}  old size {}  new size {}  new blob {}\n", h.status, h.path, size_label(&run_dir, &h.old_hash)?, size_label(&run_dir, &h.new_hash)?, h.new_hash));
    }
    head.push_str("=== end of list ===\n\n");
    Ok(format!("{head}{}", text.trim()))
}

/// `git rev-parse --path-format=absolute --git-path rebase-merge` (and `rebase-apply`), then does that folder exist? Git
/// keeps the state of a rebase that is stopped or was killed there. Why: a crashed `queue run` may have left a worktree
/// in the middle of a rebase, which must be aborted before anything else touches it.
pub fn rebase_in_progress(worktree: &Path) -> Result<bool, String> {
    for name in ["rebase-merge", "rebase-apply"] {
        let p = ok(worktree, &["rev-parse", "--path-format=absolute", "--git-path", name])?;
        if Path::new(&p).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `git rebase --abort`: stop a rebase and put the branch back exactly where it was before it began. Why: a conflict (or
/// any failure) must leave the branch as it was.
pub fn rebase_abort(worktree: &Path) -> Result<(), String> {
    files_ok(worktree, &["rebase", "--abort"]).map(|_| ())
}

/// `git diff --name-only --diff-filter=U`: the paths with unresolved conflicts. Why: the entry names them for the coder.
fn unmerged_files(worktree: &Path) -> Result<Vec<String>, String> {
    Ok(files_ok(worktree, &["diff", "--name-only", "--diff-filter=U"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

pub enum Rebase {
    /// The branch now sits on top of `onto`.
    Done,
    /// Conflicts: the rebase was aborted, the branch is as it was; these are the conflicting files.
    Conflict(Vec<String>),
    /// Git failed for another reason; any half-done rebase was aborted (the text says whether that worked).
    Failed(String),
}

/// `git rebase <onto>` in the task's worktree (its branch is checked out there): replay the branch's own commits on top of
/// `onto`. On ANY failure it then runs `git rebase --abort` (a rebase that stopped is never left behind), so the branch is
/// back where it started; the caller still verifies the tip. Why: the merge into main must be a pure fast-forward.
pub fn rebase(worktree: &Path, onto: &str) -> Result<Rebase, String> {
    rebase_with(worktree, &["rebase", onto], onto)
}

/// `git rebase --onto <onto> <upstream>` in the task's worktree: replay the commits between `upstream` and the branch tip on top of
/// `onto`, and abort on any failure exactly as `rebase` does. Why: the merge train chains entries one after another, and when an
/// entry drops out of a chain the entries after it must be lifted off its commits (`upstream` = the commit just before their own).
pub fn rebase_onto(worktree: &Path, onto: &str, upstream: &str) -> Result<Rebase, String> {
    rebase_with(worktree, &["rebase", "--onto", onto, upstream], onto)
}

/// `git checkout --detach <commit>` in a task's worktree (clean, checked by the caller): put its HEAD on `commit` without moving any
/// branch. Why: the merge train must check an entry's own earlier commit; the branch still holds every commit and is put back by
/// `checkout_branch`.
pub fn checkout_detach(worktree: &Path, commit: &str) -> Result<(), String> {
    files_ok(worktree, &["checkout", "--detach", commit]).map(|_| ())
}

/// `git checkout <branch>` in a task's worktree: put the worktree back on its branch after `checkout_detach`. Why: a task's
/// worktree must always end on its own branch.
pub fn checkout_branch(worktree: &Path, branch: &str) -> Result<(), String> {
    files_ok(worktree, &["checkout", branch]).map(|_| ())
}

/// `git log --patch --reverse <range>` piped into `git patch-id --stable`: one patch id per commit of `range` that has a diff, oldest
/// first. A patch id hashes the diff itself (line numbers and whitespace ignored), so two lists are equal exactly when the commits
/// make the same changes. Why: the merge train rebases an entry's commits after their review; the review only stands when the
/// rebased commits are the same patches that were reviewed.
pub fn patch_ids(dir: &Path, range: &str) -> Result<Vec<String>, String> {
    let log = run(dir, &review_args(&[], "log", &["--patch", "--reverse", range]))?;
    if log.code != 0 {
        return Err(format!("git log --patch {range} failed: {}", log.stderr));
    }
    if log.stdout.is_empty() {
        return Ok(Vec::new());
    }
    let o = run_with(dir, &["patch-id", "--stable"], Some(log.stdout + "\n"))?;
    if o.code != 0 {
        return Err(format!("git patch-id --stable failed: {}", o.stderr));
    }
    Ok(o.stdout.lines().filter_map(|l| l.split_whitespace().next().map(str::to_string)).collect())
}

fn rebase_with(worktree: &Path, args: &[&str], onto: &str) -> Result<Rebase, String> {
    let o = files_run(worktree, args)?;
    if o.code == 0 {
        return Ok(Rebase::Done);
    }
    let files = unmerged_files(worktree).unwrap_or_default();
    let msg = without_force_hints(if o.stderr.is_empty() { &o.stdout } else { &o.stderr });
    let aborted = match rebase_in_progress(worktree) {
        Ok(true) => rebase_abort(worktree).map(|_| "the rebase was aborted".to_string()),
        Ok(false) => Ok("no rebase was left running".to_string()),
        Err(e) => Err(e),
    };
    Ok(match (files.is_empty(), aborted) {
        (false, Ok(_)) => Rebase::Conflict(files),
        (_, Ok(a)) => Rebase::Failed(format!("git rebase {onto} failed ({a}): {msg}")),
        (_, Err(e)) => Rebase::Failed(format!("git rebase {onto} failed AND it could not be aborted: {e}; look at the worktree: {msg}")),
    })
}

/// `git merge --ff-only <commit>` in the main checkout: move the current branch forward to `commit`, only if that is a pure
/// fast-forward (the branch tip is an ancestor of it); never a merge commit, git refuses otherwise. Why: the one way task
/// work enters main, and it cannot rewrite or lose anything main already has.
pub fn merge_ff_only(main_dir: &Path, commit: &str) -> Result<(), String> {
    files_ok(main_dir, &["merge", "--ff-only", commit]).map(|_| ())
}

/// `git revert --no-commit <range>` in the main checkout: undo the commits of `range` (`<before>..<last>`) in the files and the
/// index, without committing; `commit_file` then makes ONE new commit of it. Why: a main check that failed after the queue merged
/// an entry or a merge train is undone by a new commit (history kept), never by a reset.
pub fn revert_no_commit(main_dir: &Path, range: &str) -> Result<(), String> {
    files_ok(main_dir, &["revert", "--no-commit", range]).map(|_| ())
}

/// `git revert --abort`: stop a revert and put the checked-out branch, the index and the files back as they were before it began.
/// Why: a revert that conflicted or could not be committed must leave main exactly as it was.
pub fn revert_abort(main_dir: &Path) -> Result<(), String> {
    files_ok(main_dir, &["revert", "--abort"]).map(|_| ())
}

/// `git rev-parse --path-format=absolute --git-path REVERT_HEAD`, then does that file exist? Git keeps it while a revert is begun and
/// not committed or aborted. Why: a failed queue revert must be aborted, and only one that is really in progress can be.
pub fn revert_in_progress(main_dir: &Path) -> Result<bool, String> {
    let p = ok(main_dir, &["rev-parse", "--path-format=absolute", "--git-path", "REVERT_HEAD"])?;
    Ok(Path::new(&p).exists())
}

/// `git commit --file <msg_file>` in the main checkout: commit what is staged (the queue's revert) with the message in that file.
/// Why: the revert of a range becomes ONE commit whose message names every reverted commit and the queue entry that reverted it.
pub fn commit_file(main_dir: &Path, msg_file: &Path) -> Result<(), String> {
    let f = msg_file.to_string_lossy();
    files_ok(main_dir, &["commit", "--file", &f]).map(|_| ())
}

/// `git log --format=%H %s <range>`: every commit of `range`, newest first, as (sha, subject). Why: the queue's revert message lists
/// git's default `Revert "<subject>"` line for each commit it undoes.
pub fn commit_subjects(dir: &Path, range: &str) -> Result<Vec<(String, String)>, String> {
    let out = ok(dir, &["log", "--format=%H %s", range])?;
    Ok(out.lines().filter_map(|l| l.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())).or_else(|| (!l.is_empty()).then(|| (l.to_string(), String::new())))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The git child (`run_with`, which `run`, `ok`, `files_run` and `files_ok` all use) sees no planted secret.
    #[test]
    fn git_child_sees_no_planted_secret() {
        let root = crate::scrub::planted::root("wtgit");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        assert_eq!(run(&repo, &["init", "-q"]).unwrap().code, 0);
        let o = run(&repo, &["-c", "alias.envdump=!env", "envdump"]).unwrap();
        assert_eq!(o.code, 0, "{}", o.stderr);
        crate::scrub::planted::assert_clean("wtgit.rs run_with", &o.stdout);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_is_not_started_without_a_config() {
        let nowhere = std::env::temp_dir().join(format!("bs-wtgit-no-config-{}", std::process::id()));
        let e = run(&nowhere, &["--version"]).err().expect("must refuse");
        assert!(e.contains("refusing to start it"), "{e}");
    }
}

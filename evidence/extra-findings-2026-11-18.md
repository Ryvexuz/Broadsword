# The 43 extra BLOCKING lines from the single-bug benchmark, checked against main (43e3916)

Date: 2026-11-19. Source rows: `state/bench-results/extra-lines/lines.tsv` (n = line number) and `labels.tsv` (blind Sonnet labels). No model calls were made. Product code was only read; the one hand experiment (row 34) used a throwaway git repo under scratch, since deleted.

Scope: every `real-bug` row (21), the `unsure` row (22), every `adopt=yes` hardening row (10, 11, 12, 15, 38), and the four `restates-known-bug` rows (7, 17, 27, 35) because they were cheap to check. Rows labelled `false` or `adopt=no` hardening are listed at the end.

Statuses: `fixed-since`, `still-present`, `not-applicable`. Code quoted is main at 43e3916. "crash-sim" paths are in `projects/crash-sim` (its own repo, HEAD 5415a5b).

Board tasks filed (all `open`, briefs in `state/backlog/<id>.md`):

| Task | Theme |
|---|---|
| t-20261119-215207 | crash-sim: failed stdout write exits non-zero |
| t-20261119-215208 | crash-sim: step count and time column |
| t-20261119-215209 | task board fails loudly on I/O errors |
| t-20261119-215210 | worktree base and merged check use `main`, not HEAD |
| t-20261119-215211 | worktree start: failing post-checkout hook leaves residue |
| t-20261119-215212 | config overlap check in both directions |
| t-20261119-215213 | openai_mcp redaction gaps (argparse, payload, JSON-escaped key) |
| t-20261119-215214 | openai_mcp truncated reply is an error |
| t-20261119-215215 | build notes keyed by bin target too (low risk) |
| t-20261119-215302 | task claims compare resolved project names |

## Findings

| n | Label | Status | Task | Evidence on main |
|---|---|---|---|---|
| 1 | real-bug | still-present | t-20261119-215207 | crash-sim `src/main.rs:54`: `let _ = std::io::stdout().write_all(to_csv(&samples).as_bytes());`. A failed write still reaches `exit` only through `result`, so exit 0. |
| 8 | real-bug | still-present | t-20261119-215207 | same line as 1 (duplicate). |
| 9 | real-bug | still-present | t-20261119-215207 | same line as 1 (duplicate). The `--summary` branch (`main.rs:47`, `println!`) also panics instead of exiting cleanly on a closed pipe. |
| 6 | real-bug | still-present | t-20261119-215208 | crash-sim `src/lib.rs:334`: `let steps = (cfg.duration / cfg.dt).round() as u64;`. Duration 0.016, dt 0.01 gives 2 steps, last row t = 0.0200. |
| 33 | real-bug | still-present | t-20261119-215208 | crash-sim `src/lib.rs:361`: `format!("{:.4},{:.6},...", r.t, ...)`. With dt 0.00001 rows repeat the same t. |
| 17 | restates-known-bug | fixed-since | - | crash-sim `src/lib.rs:144-162`: `Config::validate` refuses a step that reaches the damped stability limit ("raise --substeps or lower --dt"), and `run_checked_with_contact` returns Err on a blown-up state (`lib.rs:341`). |
| 2 | real-bug | still-present | t-20261119-215209 | `crates/broadsword/src/task.rs:201`: `let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(v) };`. `cmd_start` (`task.rs:387`) uses it for the overlap check. Failing input: `state/tasks` replaced by a file (or an unlistable folder). |
| 37 | real-bug | still-present | t-20261119-215209 | same line as 2. |
| 42 | real-bug | still-present | t-20261119-215209 | same lines as 2, plus `task.rs:202`: `for e in rd.flatten() {` drops per-entry errors. |
| 7 | restates-known-bug | still-present | t-20261119-215302 | `task.rs:145`: `t.get("project") != project` compares names as typed (`Crash-Sim` vs `crash-sim` on Windows). |
| 27 | restates-known-bug | still-present | t-20261119-215302 | same line as 7 (symlink variant). |
| 13 | real-bug | fixed-since | - | `crates/broadsword/src/lock.rs:3-7` and `:36`: the lock is now an OS file lock (`file.try_lock()`), no stale takeover, file content "never used for any decision". The 60 s ownerless-lock path no longer exists. |
| 30 | real-bug | fixed-since | - | `task.rs:420-426`: status running and the worktree fields are set in ONE record and one `save(&t)`: `worktree::mark_creating(&mut t, p)` sets `worktree_creating`, `branch`, `base` before that save. A kill after it leaves `worktree_creating` set; `settle_creating` (`worktree.rs:~325`) records an existing folder on `task fail`/`abandon`. |
| 4 | real-bug | still-present | t-20261119-215210 | `wtgit.rs:75-76`: `pub fn head_commit ... ok(dir, &["rev-parse", "HEAD"])`; used as the base at `worktree.rs:187`. `plan` (`worktree.rs:184`) only refuses a detached HEAD, not a non-main branch. Failing input: project checkout on `feature` ahead of `main`. |
| 20 | real-bug | still-present | t-20261119-215210 | same as 4. |
| 25 | real-bug | still-present | t-20261119-215210 | creation half as 4; removal half `wtgit.rs:186`: `run(main_dir, &["merge-base", "--is-ancestor", commit_or_branch, "HEAD"])`. |
| 26 | real-bug | still-present | t-20261119-215210 | `wtgit.rs:186` as above; callers `worktree.rs:362` and `:722`. |
| 43 | real-bug | still-present | t-20261119-215210 | same as 26. |
| 22 | unsure | fixed-since (for the checkout-fails case) | - | `worktree.rs:234-236`: after a failed `worktree_add`, `if !e.contains("already exists") && branch_tip(..) == p.base { branch_delete_if_merged(..) }` removes the empty branch. Plain checkout failure with no hook is covered; the hook case is row 34. |
| 34 | real-bug | still-present | t-20261119-215211 | Run by hand in a throwaway repo with `.git/hooks/post-checkout` = `exit 1`: `git worktree add -b task/t ../wt HEAD` exit 1, `git worktree list` still shows `wt [task/t]`, `git branch -d task/t` says "cannot delete branch 'task/t' used by worktree". `worktree.rs:230` `remove_dir` fails on a populated folder and `:234` branch delete is refused, so both stay while `roll_back_start` reports `open`. |
| 36 | real-bug | still-present | t-20261119-215212 | `config.rs:190-195`: only `worktrees_dir.starts_with(other)` for scratch, build, state. `scratch_dir = "worktrees/p"` with `worktrees_dir = "worktrees"` loads. |
| 3 | real-bug | fixed-since | - | `clients/openai_mcp.py:485-490`: `class _NoRedirect ... redirect_request ... return None` and `_OPENER = urllib.request.build_opener(_NoRedirect)`; `:522` raises "the API answered with a redirect, which is not followed". |
| 5 | real-bug | fixed-since | - | same as 3. |
| 35 | restates-known-bug | fixed-since | - | `openai_mcp.py:656-658`: `text = redact(text)` before `messages.append({"role": "tool", ...})`. |
| 10 | hardening (adopt) | still-present | t-20261119-215213 | `openai_mcp.py:699`: `args = build_parser().parse_args(argv)` runs before `load_api_key`; argparse prints `invalid int value: 'sk-...'` itself, outside `err()`. |
| 15 | hardening (adopt) | still-present | t-20261119-215213 | same as 10. |
| 38 | hardening (adopt) | still-present | t-20261119-215213 | same as 10 (`--max-turns`). |
| 12 | hardening (adopt) | still-present | t-20261119-215213 | `openai_mcp.py:592`: `payload: dict = {"model": args.model, "messages": messages}` is sent without `redact`; a user prompt holding the key reaches the model. |
| 21 | real-bug | still-present | t-20261119-215213 | `openai_mcp.py:688`: `text = redact(json.dumps(doc, indent=2, ensure_ascii=False))`; `load_api_key` (`:121`) accepts `"` and `\`, whose escaped form no longer equals the stored secret. Failing input: key `abc"def-12345`. |
| 11 | hardening (adopt) | still-present | t-20261119-215214 | `openai_mcp.py:602-606`: finish_reason only logged; `if not calls: run.final = message.get("content") or ""; return`. |
| 39 | real-bug | still-present (low risk) | t-20261119-215215 | `buildsrc.rs:410-411` `prepare_packages` and `read_note(cfg, p)` key notes by package; `check_installable` (`:460-471`) reads the note for `broadsword` only, `check_running_binary` compares only the exe path. A worktree whose package is renamed with `[[bin]] name = "broadsword"` overwrites the exe and cleans nothing. Not run. |

## Not filed

| n | Label | Reason |
|---|---|---|
| 14, 23, 40 | false | The real commit 36bca91 edits `broadsword.toml`; the old case diff left it out. Harness artifact, fixed below. |
| 18, 41 | false | Same for commit cadbb6d (`worktrees_dir`). Harness artifact. |
| 19 | false | Not a harness artifact: config root derives from the folder found via current_dir. |
| 16, 28, 29, 31, 32, 24 | hardening, adopt=no | Inherent git races, git behaviour or the user's own config. Unchanged from the labels; not re-checked. |

Re-scoring of the 5 harness rows (14, 18, 23, 40, 41): no reviewer was re-run (no model calls), so they are relabelled by hand as `harness-artifact`, not `real`. Counting them out, the 43 lines are 21 real + 1 unsure + 11 hardening + 4 restatements + 1 false (19) + 5 harness artifacts. (The brief's "6 false" = 5 harness + row 19.)

## Harness fix

`bench/reviewer/make_cases.sh` now diffs every touched file (all paths, only `__pycache__` binaries excluded), and refuses to finish if a touched file is missing from its case diff or if the 3 commits that changed `broadsword.toml` do not show it.

Run from the worktree (with a clone of crash-sim at `projects/crash-sim`, since `projects/` is git-ignored):

    bash bench/reviewer/make_cases.sh   -> exit 0
      1493 36bca91.diff   1885 3ad7320.diff   515 a19ee1a.diff   1826 cadbb6d.diff   1219 fd98728.diff
    grep -c '^diff --git a/broadsword.toml b/broadsword.toml'  ->  36bca91: 1, 3ad7320: 1, cadbb6d: 1

Fails before: with the old pathspec (`-- 'crates/broadsword/src/*'`) for 3ad7320 the new check printed `make_cases: 3ad7320.diff is missing broadsword.toml` and exited 1; the old 36bca91 diff had 0 `broadsword.toml` entries.

Warning: these diffs are 2-3 times longer than the old source-only ones (they now include tests). Costs and catch rates from runs on the old diffs are not directly comparable. `bench/reviewer/cases/` is git-ignored, so the diffs are regenerated, not committed. The README line "Only source files" in bench/reviewer/README.md (outside this task's claim) and the `cases.json` brief for each case still describe the old diffs; the orchestrator should update them.

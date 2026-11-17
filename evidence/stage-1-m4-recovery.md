# Stage 1, milestone 4: recovery — done check

**Check:** kill Broadsword mid-job; on restart it resumes or rolls back and nothing is lost.

**Result: PASS**, 2026-11-17, on `main` at merge commit `fde0798`.

Both parts were run on real projects by the orchestrator, using `scratch/m4-done-check.sh`. Broadsword was hard-killed with `taskkill /PID <pid> /F`, without `/T`. That simulates a crash and leaves its child process orphaned.

## Part 1: a file-changing job is rolled back

The job was `broadsword run crash-sim -- "echo PARTIAL-EDIT>>README.md & echo new>job-new-file.txt & ping -n 90 127.0.0.1 >nul"`.

```text
--- crash-sim before:            (git status empty)   9ff5289   README.md md5 be235e36...
--- crash-sim mid-job:            M README.md   ?? job-new-file.txt
SUCCESS: The process with PID 21396 has been terminated.
--- child still alive after killing broadsword?   1

--- build crash-sim before recover (auto-recover must refuse, not roll back):
  killing orphaned process tree (pid 22484)
  this run job may have changed files: run `broadsword recover` to roll it back, or `broadsword recover --abandon ...`
broadsword: refusing to start: job ... was interrupted and may have changed files ...
build exit=3

--- recover:
  rolled back to the checkpoint: restored 1 file(s), removed 1 new file(s), restored 0 ignored file(s)
  checkpoint (state before the job):  refs/broadsword/checkpoints/20261117-175625-crash-sim-run
  interrupted (partial work, kept):   refs/broadsword/interrupted/20261117-175625-crash-sim-run/1
  index before rollback, kept:        refs/broadsword/interrupted-index/20261117-175625-crash-sim-run/1
recover exit=0

--- crash-sim after:              (git status empty)   9ff5289   README.md md5 be235e36...  (identical)
--- child alive after recover?    0
--- record status: rolled_back
--- partial work kept:            README.md +1 (PARTIAL-EDIT), job-new-file.txt +1
```

## Part 2: a read-only job is resumed

The job was `broadsword check broadsword`. It was killed during its test step.

```text
--- job running: record 20261117-175629-broadsword-check.rec, step test, broadsword pid 3944
SUCCESS: The process with PID 3944 has been terminated.
--- recover:
recovering job 20261117-175629-broadsword-check: check broadsword was interrupted (broadsword pid 3944 is gone)
  read-only job: resuming `check broadsword` from step `test`
test: ok — 84 passed, 0 failed
recover exit=0
--- old record: resumed -> 20261117-175635-broadsword-check     new record: done
```

## Tests and review

- 46 unit tests, 15 CLI, 10 jobs and 13 recovery, all passing. Clippy runs with `-D warnings` and is clean.
- The reviewer (Opus) failed four passes, each on a reproduced way to lose work:
  1. A retried rollback overwrote edits made after a failed attempt. Line-ending conversion (`core.autocrlf=true` here) changed restored bytes. Ignored files such as `.env` were not protected.
  2. The ignored-file size cap could overwrite bytes that weren't saved. A file turned into a folder blocked recovery.
  3. The size cap could still overwrite work through the normal restore path. The fix was one `Guard` that every working-tree write and delete passes through.
  4. Restoring the Git index dropped content staged after the kill.
- **Fifth pass: PASS.** Every reproduction holds, and each new regression test fails on the version before its fix. Rollback never moves or deletes an existing ref, HEAD or config.

## Known limitations (from the reviewer's final pass)

1. Ignored files over `checkpoint_ignored_max_mb` are not protected, and neither is anything under `target/`, build, scratch or state. They get a warning at checkpoint, and a "not protected" note at rollback only if their size changed.
2. Checkpoints copy ignored files in full every time. 20,000 small ignored files took about 29 s per job.
3. `refs/broadsword/*` and their objects accumulate; nothing prunes them.
4. Empty folders are not restored, and new empty folders a job made are left in place.
5. Restoring the index drops index-only flags (intent-to-add, skip-worktree, assume-unchanged). No content is lost; it is in the interrupted-index ref.
6. There is a microsecond window between the Guard's hash check and its write, against a process writing at the same moment.
7. Rare lock race: two recovers can run at once. That gives a busy exit or duplicate resumes, but no lost work.
8. A partial or impossible rollback blocks the project until `recover` or `recover --abandon <id>`.
9. Processes a job left behind whose parent already exited are not killed.
10. Process checks are implemented and tested on Windows only.
11. In Part 2 no "killing orphaned process tree" line was printed for the interrupted check. Either its cargo child had already exited, or its identity could not be verified exactly, in which case recover does not kill it. Not investigated.

use super::*;

// ---- the reviewer says FAIL ---------------------------------------------------------------

#[test]
fn a_planted_bug_is_rejected_before_merging_then_the_fix_is_requeued_and_merged() {
    let r = Root::new("reject", 30);
    let id = r.started("src");
    let buggy = lib_with("\npub fn sub(a: i32, b: i32) -> i32 {\n    a + b // PLANTED_BUG: this should subtract\n}\n");
    let bad = r.commit(&r.wt(&id), "src/lib.rs", &buggy, "task: add sub (buggy)");
    let main_before = r.head();
    let e1 = r.add(&id, "high", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains(&e1) && o.stderr.contains("rejected") && o.stderr.contains("planted bug"), "{}", o.stderr);
    assert_eq!(r.status(&e1), "rejected");
    assert!(r.field(&e1, "reason").contains("opus"), "the high-risk reviewer model");
    assert!(r.show(&e1).contains("planted bug: the function subtracts instead of adding"));
    assert_eq!(r.head(), main_before, "main is unchanged");
    assert_eq!(r.task_status(&id), "running", "the task stays running");
    assert!(r.wt(&id).exists(), "the worktree is kept");
    assert_eq!(r.branch_tip(&id), bad);
    assert!(fs::read_to_string(r.dir.join("state/queue").join(format!("{e1}.review"))).unwrap().contains("VERDICT: FAIL"));
    assert!(fs::read_to_string(r.fake_out.join("calls.log")).unwrap().contains("--model\u{1f}opus"));

    // the coder fixes it and queues it again
    let fixed = lib_with("\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n");
    r.commit(&r.wt(&id), "src/lib.rs", &fixed, "task: fix sub");
    let e2 = r.add(&id, "high", "sonnet");
    assert_ne!(e1, e2);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e2), "merged");
    assert_eq!(r.status(&e1), "rejected", "the first entry stays as history");
    assert_eq!(r.task_status(&id), "done");
    assert!(!r.wt(&id).exists());
    assert!(fs::read_to_string(r.p().join("src/lib.rs")).unwrap().contains("a - b"));
    assert!(!fs::read_to_string(r.p().join("src/lib.rs")).unwrap().contains("PLANTED_BUG"));
}

#[test]
fn unparseable_or_inconsistent_review_output_counts_as_fail_and_the_raw_text_is_kept() {
    let r = Root::new("garbage", 30);
    for (claim, marker, raw) in [("src", "GARBAGE_REVIEW", "Looks fine to me, nothing to add."), ("docs", "EMPTY_PASS", "VERDICT: PASS\\nMINOR: none")] {
        let id = r.started(claim);
        let file = if claim == "src" { "src/note.txt" } else { "docs/note.txt" };
        r.commit(&r.wt(&id), file, &format!("{marker}\n"), "task: a note");
        let main_before = r.head();
        let e = r.add(&id, "low", "sonnet");
        let o = r.run(&["queue", "run"]);
        assert_eq!(o.code, 5, "{marker}: {}{}", o.stdout, o.stderr);
        assert_eq!(r.status(&e), "rejected", "{marker}");
        assert_eq!(r.head(), main_before, "{marker}: nothing merged");
        let reason = r.field(&e, "reason");
        assert!(reason.contains("counts as FAIL"), "{marker}: {reason}");
        let review = fs::read_to_string(r.dir.join("state/queue").join(format!("{e}.review"))).unwrap();
        assert!(review.contains("--- raw reviewer output") && review.contains(raw) && review.contains("answer followed the format: NO"), "{marker}: {review}");
        assert_eq!(r.task_status(&id), "running");
    }
}

#[test]
fn a_reviewer_model_equal_to_the_coder_blocks_the_entry_before_anything_runs() {
    let r = Root::new("samemodel", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    // `queue add` refuses a haiku coder at low risk now, so the same-model block at run time is reached by changing the setting
    // after the entry was queued with a coder the setting allowed.
    let e = r.add(&id, "low", "Sonnet");
    let toml = r.dir.join("broadsword.toml");
    fs::write(&toml, fs::read_to_string(&toml).unwrap().replace("reviewer_model_low = \"haiku\"", "reviewer_model_low = \"sonnet\"")).unwrap();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("same as the coder"), "{}", r.field(&e, "reason"));
    assert_eq!(r.calls(), 0);
    assert_eq!(r.branch_tip(&id), tip);
    // high risk with coder opus is blocked too once the fallback is also the coder's family
    let toml = r.dir.join("broadsword.toml");
    fs::write(&toml, fs::read_to_string(&toml).unwrap().replace("reviewer_model_high_fallback = \"sonnet\"", "reviewer_model_high_fallback = \"opus\"")).unwrap();
    let id2 = r.started("docs");
    r.commit(&r.wt(&id2), "docs/a.txt", "x\n", "task: docs");
    let e2 = r.add(&id2, "high", "opus");
    let o = r.run(&["queue", "run", "--entry", &e2]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e2), "blocked");
    assert!(r.field(&e2, "reason").contains("reviewer_model_high_fallback"), "{}", r.field(&e2, "reason"));
    assert_eq!(r.calls(), 0);
}

/// Runs one high-risk entry coded by `coder`; returns the reviewer test hook's call log and the entry as recorded.
fn high_review(name: &str, coder: &str) -> (String, String) {
    let r = Root::new(name, 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "high", coder);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    (fs::read_to_string(r.fake_out.join("calls.log")).unwrap(), r.show(&e))
}

#[test]
fn an_opus_coded_high_entry_is_reviewed_by_sonnet() {
    let (calls, out) = high_review("hifb", "opus");
    assert!(calls.contains("--model\u{1f}sonnet") && !calls.contains("--model\u{1f}opus"), "{calls}");
    assert!(calls.contains("--effort\u{1f}high") && out.contains("effort `high`"), "{calls}{out}");
    assert!(out.contains("fallback: coder is opus"), "{out}");
}

#[test]
fn a_sonnet_coded_high_entry_is_reviewed_by_opus() {
    let (calls, out) = high_review("hiop", "sonnet");
    assert!(calls.contains("--model\u{1f}opus") && !calls.contains("--model\u{1f}sonnet"), "{calls}");
    assert!(!out.contains("fallback"), "{out}");
}

// ---- timeout, stale, secrets ------------------------------------------------------------

#[test]
fn a_reviewer_timeout_blocks_the_entry_with_no_verdict_and_it_can_be_retried() {
    let r = Root::new("timeout", 2);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let main_before = r.head();
    let e = r.add(&id, "low", "sonnet");
    let o = r.run_env(&["queue", "run"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    // the queue RECORDED when it called the reviewer and when it blocked the entry (the timestamps in the entry's step log): that span is the
    // reviewer call alone, without the check or the process starts that load stretches. A hang the queue cuts at 2 s spans about 2 s; one it
    // never cuts spans FAKE_HANG_CAP_SECONDS, so the bound is half of that
    let steps = r.show(&e);
    let span = recorded_span_secs(&steps, "reviewer `", ": blocked: ");
    assert!(span < FAKE_HANG_CAP_SECONDS / 2, "the reviewer hang ({FAKE_HANG_CAP_SECONDS} s cap) must be cut at the 2 s timeout, the entry recorded {span} s:
{steps}");
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("within 2 s") && r.field(&e, "reason").contains("no verdict"), "{}", r.field(&e, "reason"));
    assert_eq!(r.head(), main_before, "nothing merged");
    assert!(r.field(&e, "verdict").is_empty());
    // the timeout was recorded, the reviewer is asked again on a retry
    fs::remove_file(&hang).unwrap();
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    assert_eq!(r.calls(), 2);
}

#[test]
fn a_failing_reviewer_cli_blocks_the_entry() {
    let r = Root::new("clifail", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run_env(&["queue", "run"], &[("FAKE_CLAUDE_EXIT", "7")]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("exited 7"), "{}", r.field(&e, "reason"));
    // a CLI that is not there at all
    let o = r.run_env(&["queue", "run", "--entry", &e], &[("BROADSWORD_REVIEWER_CLI", "no-such-reviewer-cli-anywhere")]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    let reason = r.field(&e, "reason");
    assert!(reason.contains("cannot start the reviewer CLI") && reason.contains("no-such-reviewer-cli-anywhere"), "{reason}");
    assert_eq!(r.calls(), 1, "only the first (exit 7) call reached the fake");
}

#[test]
fn the_reviewer_cli_gets_a_scrubbed_environment_and_a_redacted_prompt() {
    let r = Root::new("secrets", 30);
    fs::write(r.dir.join(".env"), "OPENAI_API_KEY=sk-test-abcdefghij\nOTHER=1\n").unwrap();
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/leak.txt", "the key is sk-test-abcdefghij, oops\n", "task: leaky note");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run_env(
        &["queue", "run"],
        &[
            ("OPENAI_API_KEY", "sk-test-abcdefghij"),
            ("SOME_SERVICE_API_KEY", "service-key-1234567"),
            ("MY_TOKEN", "tok-abcdefgh"),
            ("MY_SECRET", "secret-abcdefgh"),
            ("PLAIN_KEY", "k-abcdefgh"),
            ("COPY_OF_THE_KEY", "prefix-sk-test-abcdefghij-suffix"),
            ("HARMLESS_SETTING", "keep-me"),
        ],
    );
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    let env = r.fake_env();
    for (k, _) in &env {
        let u = k.to_uppercase();
        assert!(!(u.ends_with("_API_KEY") || u.ends_with("_KEY") || u.ends_with("_TOKEN") || u.ends_with("_SECRET")), "{k} reached the reviewer CLI");
    }
    for gone in ["OPENAI_API_KEY", "SOME_SERVICE_API_KEY", "MY_TOKEN", "MY_SECRET", "PLAIN_KEY", "COPY_OF_THE_KEY"] {
        assert!(!env.iter().any(|(k, _)| k == gone), "{gone} must be removed (by name, or by holding the .env key)");
    }
    assert!(!env.iter().any(|(_, v)| v.contains("sk-test-abcdefghij")), "no variable may hold the key");
    assert!(env.iter().any(|(k, v)| k == "HARMLESS_SETTING" && v == "keep-me"), "ordinary variables pass");
    assert!(env.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")), "PATH passes");
    // the prompt carries the diff only: the key inside the diff is redacted, and the .env file itself is not there
    let stdin = fs::read_to_string(r.fake_out.join("stdin.txt")).unwrap();
    assert!(stdin.contains("the key is [REDACTED], oops"), "{stdin}");
    assert!(!stdin.contains("sk-test-abcdefghij") && !stdin.contains("OTHER=1"), "{stdin}");
}


// ---- B2, M5: every .env value is a secret --------------------------------------------------

#[test]
fn every_env_file_value_is_redacted_and_scrubbed_whatever_its_name_and_stored_text_is_redacted_too() {
    let r = Root::new("envall", 30);
    fs::write(r.dir.join(".env"), "GITHUB_PAT=ghp_abcdef0123456789\nDB_PASSWORD=hunter2-hunter2\nREGION=eu-central-17\nTINY=ab\n").unwrap();
    let id = r.started("src");
    // the diff leaks the values, and the build prints one (an unused-variable warning quotes the source line)
    r.commit(&r.wt(&id), "src/note.txt", "pat ghp_abcdef0123456789 pw hunter2-hunter2 region eu-central-17 tiny ab\n", "task: note");
    r.commit(&r.wt(&id), "src/lib.rs", &lib_with("\npub fn leak() {\n    let unused_leak = \"eu-central-17\";\n}\n"), "task: leak");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run_env(
        &["queue", "run"],
        &[
            ("GITHUB_PAT", "ghp_abcdef0123456789"),
            ("PLAIN_NAME", "ghp_abcdef0123456789"),
            ("WRAPPED", "<hunter2-hunter2>"),
            ("REGION_COPY", "eu-central-17"),
            ("SHORT_EQUAL", "ab"),
            ("SHORT_INSIDE", "xaby"),
            ("MY_PASS", "p"),
            ("CLOUD_CREDENTIALS", "c"),
            ("X_AUTH_HEADER", "a"),
            ("HARMLESS_SETTING", "keep-me"),
        ],
    );
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    let env = r.fake_env();
    for gone in ["GITHUB_PAT", "PLAIN_NAME", "WRAPPED", "REGION_COPY", "SHORT_EQUAL", "MY_PASS", "CLOUD_CREDENTIALS", "X_AUTH_HEADER"] {
        assert!(!env.iter().any(|(k, _)| k == gone), "{gone} reached the reviewer CLI");
    }
    assert!(env.iter().any(|(k, v)| k == "SHORT_INSIDE" && v == "xaby"), "a short .env value is scrubbed only where a variable equals it");
    assert!(env.iter().any(|(k, v)| k == "HARMLESS_SETTING" && v == "keep-me"));
    let stdin = fs::read_to_string(r.fake_out.join("stdin.txt")).unwrap();
    assert!(stdin.contains("pat [REDACTED] pw [REDACTED] region [REDACTED] tiny ab"), "{stdin}");
    assert!(stdin.contains("unused variable") && stdin.contains("let unused_leak = \"[REDACTED]\""), "the check tail is redacted: {stdin}");
    for secret in ["ghp_abcdef0123456789", "hunter2-hunter2", "eu-central-17"] {
        assert!(!stdin.contains(secret), "{secret} in the prompt");
        for f in [r.entry_file(&e), r.dir.join("state/queue").join(format!("{e}.review"))] {
            let text = fs::read_to_string(&f).unwrap();
            assert!(!text.contains(secret), "{secret} stored in {}", f.display());
        }
    }
    let entry = fs::read_to_string(r.entry_file(&e)).unwrap();
    assert!(entry.contains("unused variable") && entry.contains("[REDACTED]"), "the check tail is stored, redacted");
    assert!(r.show(&e).contains("shorter than 6 characters"), "{}", r.show(&e));
}

// ---- B3: the reviewer cannot be swapped without a trace -------------------------------------

#[test]
fn the_reviewer_cli_path_is_printed_and_recorded_and_a_scratch_override_is_a_loud_test_hook() {
    let r = Root::new("hook", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    let path = r.fake.to_string_lossy().replace('\\', "/");
    assert!(o.stdout.contains(&format!("TEST HOOK ACTIVE: reviewer CLI is {path}")), "{}", o.stdout);
    assert_eq!(r.field(&e, "reviewer CLI"), path);
    assert_eq!(r.field(&e, "test hook"), "true");
    let review = fs::read_to_string(r.dir.join("state/queue").join(format!("{e}.review"))).unwrap();
    assert!(review.contains("test_hook: true") && review.contains(&format!("reviewer cli: {path}")), "{review}");
}

#[test]
fn a_reviewer_cli_outside_the_root_is_a_normal_reviewer_and_one_inside_build_is_refused() {
    let r = Root::new("outside", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    // the shipped fake, copied into the root's build folder: refused, no model is called
    fs::create_dir_all(r.dir.join("build")).unwrap();
    fs::write(r.dir.join("build/CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55
").unwrap(); // so cargo will still clean in it
    let in_build = r.dir.join("build").join(r.fake.file_name().unwrap());
    fs::copy(fake_claude(), &in_build).unwrap();
    let main_before = r.head();
    let o = r.run_env(&["queue", "run"], &[("BROADSWORD_REVIEWER_CLI", in_build.to_str().unwrap())]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("refused") && r.field(&e, "reason").contains("build/"), "{}", r.field(&e, "reason"));
    assert_eq!((r.calls(), r.head()), (0, main_before.clone()), "nothing was called or merged");
    // any other folder inside the root is refused too (a relative path included)
    fs::create_dir_all(r.dir.join("tools")).unwrap();
    fs::copy(fake_claude(), r.dir.join("tools").join(r.fake.file_name().unwrap())).unwrap();
    let rel = format!("tools/{}", r.fake.file_name().unwrap().to_string_lossy());
    let o = r.run_env(&["queue", "run", "--entry", &e], &[("BROADSWORD_REVIEWER_CLI", &rel)]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.calls(), 0);
    let _ = fs::remove_file(&in_build); // (a later check may have cleaned it already)
    fs::remove_dir_all(r.dir.join("tools")).unwrap();
    // a path outside the root (the cargo build folder, like the real Claude install is outside) is a normal reviewer: no hook
    let o = r.run_env(&["queue", "run", "--entry", &e], &[("BROADSWORD_REVIEWER_CLI", fake_claude())]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(!o.stdout.contains("TEST HOOK"), "{}", o.stdout);
    assert!(o.stdout.contains("reviewer CLI is ") && o.stdout.contains(&fake_claude().replace('\\', "/")), "{}", o.stdout);
    assert_eq!(r.field(&e, "test hook"), "false");
    assert_eq!(r.field(&e, "reviewer CLI"), fake_claude().replace('\\', "/"));
    let review = fs::read_to_string(r.dir.join(r.field(&e, "review file"))).unwrap();
    assert!(review.contains("test_hook: false"), "{review}");
}

#[test]
fn output_that_is_not_the_cli_json_or_from_another_model_family_or_without_model_usage_is_a_fail_with_the_raw_text_kept() {
    let r = Root::new("backend", 30);
    for (claim, env, needle) in [
        ("src", ("FAKE_CLAUDE_PLAIN", "VERDICT: PASS\nBLOCKING: none\nMINOR: none"), "not its JSON answer"),
        ("docs", ("FAKE_CLAUDE_MODEL", "claude-opus-4-5"), "came from claude-opus-4-5"),
        ("clients", ("FAKE_CLAUDE_MODEL", "none"), "no `modelUsage`"),
    ] {
        let id = r.started(claim);
        r.commit(&r.wt(&id), &format!("{claim}/note.txt"), "x\n", "task: a note");
        let main_before = r.head();
        let e = r.add(&id, "low", "sonnet");
        let o = r.run_env(&["queue", "run"], &[env]);
        assert_eq!(o.code, 5, "{needle}: {}{}", o.stdout, o.stderr);
        assert_eq!(r.status(&e), "rejected", "{needle}");
        assert!(r.field(&e, "reason").contains(needle) && r.field(&e, "reason").contains("counts as FAIL"), "{needle}: {}", r.field(&e, "reason"));
        assert_eq!(r.head(), main_before, "{needle}: nothing merged");
        let review = fs::read_to_string(r.dir.join("state/queue").join(format!("{e}.review"))).unwrap();
        assert!(review.contains("--- raw reviewer output") && review.contains("answer followed the format: NO"), "{needle}: {review}");
        if env.0 == "FAKE_CLAUDE_PLAIN" {
            assert!(review.contains("VERDICT: PASS"), "the raw text is kept: {review}");
        }
    }
}

#[test]
fn the_fake_with_default_settings_from_outside_the_root_fails_the_family_check() {
    let r = Root::new("fakedefault", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x
", "task: new file");
    let main_before = r.head();
    let e = r.add(&id, "low", "sonnet");
    let mut c = r.command(&["queue", "run"], &[("BROADSWORD_REVIEWER_CLI", fake_claude())]);
    c.env_remove("FAKE_CLAUDE_MODEL");
    let o = c.output().unwrap();
    let (code, err) = (o.status.code().unwrap(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert_eq!(code, 5, "{err}");
    assert_eq!(r.status(&e), "rejected");
    assert!(r.field(&e, "reason").contains("fake-model") && r.field(&e, "reason").contains("counts as FAIL"), "{}", r.field(&e, "reason"));
    assert_eq!(r.head(), main_before);
}

// ---- M1, M2, M3, M4, M6 -----------------------------------------------------------------------

#[test]
fn a_pass_with_an_empty_blocking_header_followed_by_findings_is_rejected() {
    let r = Root::new("emptyheader", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/note.txt", "EMPTY_HEADER_PASS\n", "task: a note");
    let main_before = r.head();
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "rejected");
    assert!(r.show(&e).contains("src/lib.rs:1 - the build is broken"), "{}", r.show(&e));
    assert_eq!(r.head(), main_before);
}

#[test]
fn reviewer_calls_are_capped_per_entry_and_every_real_call_counts() {
    let r = Root::new("cap", 2);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let e = r.add(&id, "low", "sonnet");
    let main_before = r.head();
    for n in 1..=3 {
        let o = r.run_env(&["queue", "run", "--entry", &e], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
        assert_eq!(o.code, 3, "call {n}: {}{}", o.stdout, o.stderr);
        assert!(r.field(&e, "reason").contains("within 2 s"), "call {n}: {}", r.field(&e, "reason"));
        assert_eq!(r.field(&e, "review calls"), n.to_string());
    }
    assert_eq!(r.calls(), 3);
    fs::remove_file(&hang).unwrap();
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("review call cap reached"), "{}", r.field(&e, "reason"));
    assert_eq!(r.calls(), 3, "the fourth call was never made");
    assert_eq!(r.head(), main_before);
}

#[test]
fn a_low_entry_passes_no_effort_flag_and_records_none() {
    let r = Root::new("loweff", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    assert_eq!(r.run(&["queue", "run"]).code, 0);
    assert!(!fs::read_to_string(r.fake_out.join("calls.log")).unwrap().contains("--effort"));
    assert_eq!(r.field(&e, "reviewer_effort"), "");
}

#[test]
fn model_names_are_compared_by_family_for_the_reviewer_must_differ_from_coder_rule() {
    let r = Root::new("family", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "claude-sonnet-4-5-20250929");
    let toml = r.dir.join("broadsword.toml");
    fs::write(&toml, fs::read_to_string(&toml).unwrap().replace("reviewer_model_low = \"haiku\"", "reviewer_model_low = \"sonnet\"")).unwrap();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("same as the coder"), "{}", r.field(&e, "reason"));
    assert_eq!(r.calls(), 0);
}


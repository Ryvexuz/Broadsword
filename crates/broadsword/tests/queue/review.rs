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

// ---- crash and resume ---------------------------------------------------------------------

/// The cause of the flaky `a_run_killed_mid_review_is_resumed_by_the_next_queue_run`: a reviewer that dies while its run is alive
/// ends the entry `blocked` (a reviewer failure, retried by the next run), not `running`. A killed tree can show exactly this.
#[test]
fn a_reviewer_that_dies_while_its_run_lives_blocks_the_entry() {
    let r = Root::new("reviewerdies", 120);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x
", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut run = r.spawn(&["queue", "run"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the reviewer to start", 120, || r.calls() == 1);
    let kids = child_pids(run.id());
    assert_eq!(kids.len(), 1, "the run has exactly one child, the reviewer: {kids:?}");
    // the reviewer is killed first and the run is slow to follow, as when the machine is loaded
    let _ = Command::new(if cfg!(windows) { "taskkill" } else { "kill" }).args(if cfg!(windows) { vec!["/F".to_string(), "/PID".to_string(), kids[0].to_string()] } else { vec!["-9".to_string(), kids[0].to_string()] }).stdout(Stdio::null()).stderr(Stdio::null()).status();
    wait_for("the run to notice and end", 60, || matches!(run.try_wait(), Ok(Some(_))));
    assert_eq!(r.status(&e).split(' ').next().unwrap(), "blocked", "{}", r.show(&e));
    kill_run_only(&mut run);
    fs::remove_file(&hang).unwrap();
}

#[test]
fn a_run_killed_mid_review_is_resumed_by_the_next_queue_run() {
    let r = Root::new("crash", 120);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut child = r.spawn(&["queue", "run"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the reviewer to start", 120, || r.calls() == 1);
    kill_run_only(&mut child); // the process dies in the middle of the review step, with no chance to react to its reviewer
    assert_eq!(r.status(&e).split(' ').next().unwrap(), "running");
    assert!(r.status(&e).contains("interrupted"), "{}", r.status(&e));
    assert_eq!(r.field(&e, "step"), "review");
    assert!(r.run(&["queue", "list"]).stdout.contains("interrupted"));
    assert!(!r.p().join("src/new.txt").exists());
    fs::remove_file(&hang).unwrap();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("RESUMING") && o.stdout.contains("review"), "{}", o.stdout);
    assert_eq!(r.status(&e), "merged");
    assert_eq!(r.calls(), 2, "no verdict was recorded, so the reviewer was asked again");
    assert!(r.p().join("src/new.txt").exists());
    assert_eq!(r.task_status(&id), "done");
}

// ---- audit fixes S2, S4, S7 ---------------------------------------------------------------

/// A queued entry for a fresh commit, rewritten by hand into a `running` entry at `step` with no live process.
fn hand_made_running(r: &Root, step: &str, set: &[(&str, &str)]) -> (String, String, String) {
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let base = r.head();
    let mut fields: Vec<(&str, &str)> = vec![("status", "running"), ("step", step), ("attempt", "1"), ("pid", "4000000"), ("main_branch", "main"), ("base_head", &base), ("checked_tip", &tip)];
    fields.extend_from_slice(set);
    r.edit_entry(&e, &fields);
    (id, e, tip)
}

#[test]
fn a_resumed_entry_with_a_recorded_fail_ends_rejected_like_a_fresh_fail_and_the_task_can_be_queued_again() {
    for step in ["review", "merge"] {
        let r = Root::new("resumefail", 30);
        let (id, e, tip) = hand_made_running(&r, step, &[("verdict", "FAIL"), ("reviewer_model", "haiku")]);
        r.edit_entry(&e, &[("reviewed_tip", &tip)]);
        let main_head = r.head();
        let o = r.run(&["queue", "run"]);
        assert_eq!(o.code, 5, "{step}: {}{}", o.stdout, o.stderr);
        assert_eq!(r.status(&e), "rejected", "{step}");
        assert_eq!(r.head(), main_head, "{step}: nothing merged");
        assert_eq!(r.task_status(&id), "running");
        assert!(r.wt(&id).exists());
        assert_eq!(r.calls(), 0, "the reviewer is not asked again");
        // add works again
        assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]).code, 0, "{step}");
    }
}

#[test]
fn a_resumed_pass_without_a_reviewed_commit_ends_stale_and_does_not_block_the_queue() {
    for step in ["review", "merge"] {
        let r = Root::new("resumenotip", 30);
        let (id, e, _tip) = hand_made_running(&r, step, &[("verdict", "PASS"), ("reviewed_tip", "")]);
        let main_head = r.head();
        let o = r.run(&["queue", "run", "--entry", &e]);
        assert_eq!(o.code, 3, "{step}: {}{}", o.stdout, o.stderr);
        assert_eq!(r.status(&e), "stale", "{step}");
        assert_eq!(r.head(), main_head);
        // a stale entry can be cancelled, and then the task can be queued again
        assert_eq!(r.run(&["queue", "cancel", &e]).code, 0);
        assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]).code, 0);
    }
}

// ---- the GPT gate for Opus-coded high-risk entries (fake client only: no real GPT call is ever made) ------------

/// A stand-in for clients/openai_review.py as the GATE: answers as `FAKE_GATE_KIND` says and logs its arguments and
/// OPENAI_REVIEW_CALL_TYPE to fake-out/gate-calls.log.
const FAKE_GATE: &str = r#"import json, os, sys
out = os.environ["FAKE_GATE_OUT_DIR"]
os.makedirs(out, exist_ok=True)
prompt = sys.stdin.buffer.read()
args = sys.argv[1:]
model = args[args.index("--model") + 1]
open(os.path.join(out, "gate-calls.log"), "a", encoding="utf-8").write("type=" + os.environ.get("OPENAI_REVIEW_CALL_TYPE", "") + " " + " ".join(args).replace("\n", " ") + "\n")
kind = os.environ["FAKE_GATE_KIND"]
def reply(text, used=None, reported=False, usage_model=None):
    print(json.dumps({"type": "result", "is_error": False, "result": text, "total_cost_usd": None, "usage": {"input_tokens": 70, "output_tokens": 9},
        "modelUsage": {(usage_model or (used[0] if used and len(used) == 1 else model)): {}}, "model_reported": reported, "model_used": used or [], "backend": "codex"}))
if kind == "pass":
    reply("VERDICT: PASS\nBLOCKING: none\nMINOR: none")
elif kind == "pass_reported":
    reply("VERDICT: PASS\nBLOCKING: none\nMINOR: none", [model + "-2026-01-01"], True)
elif kind == "fail":
    reply("VERDICT: FAIL\nBLOCKING: src/lib.rs:9 - gpt says this is wrong\nMINOR: none")
elif kind == "other_model":
    reply("VERDICT: PASS\nBLOCKING: none\nMINOR: none", ["gpt-5-mini"], True)
elif kind == "other_key":
    reply("VERDICT: PASS\nBLOCKING: none\nMINOR: none", usage_model="gpt-5-mini")
elif kind == "cap":
    print(json.dumps({"type": "result", "is_error": True, "result": "daily_cap_reached: 20.0100 of 20", "total_cost_usd": None}))
    sys.exit(1)
elif kind == "garbage":
    print("not json")
"#;

impl Root {
    /// A root whose high-risk Opus-coded entries are gated by GPT (`reviewer_model_high_gpt = "gpt-6.1-sol"`).
    fn with_gpt_gate(name: &str) -> Root {
        let r = Root::with_shadow(name, 30);
        let toml = r.dir.join("broadsword.toml");
        let text = fs::read_to_string(&toml).unwrap();
        assert!(text.contains("reviewer_model_high_gpt = \"off\""), "{text}");
        fs::write(&toml, text.replace("reviewer_model_high_gpt = \"off\"", "reviewer_model_high_gpt = \"gpt-6.1-sol\"")).unwrap();
        r
    }

    /// Run `args` with the GPT client replaced by the fake gate script answering as `kind`; the shadow uses the same script.
    fn run_gate(&self, kind: &str, args: &[&str]) -> Out {
        let script = self.dir.join("scratch/shadow/fake_gate.py");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, FAKE_GATE).unwrap();
        let out = self.fake_out.to_string_lossy().into_owned();
        let s = script.to_string_lossy().into_owned();
        self.run_env(args, &[("BROADSWORD_SHADOW_REVIEWER_SCRIPT", &s), ("FAKE_GATE_KIND", kind), ("FAKE_GATE_OUT_DIR", &out)])
    }

    fn gate_log(&self) -> Vec<String> {
        fs::read_to_string(self.fake_out.join("gate-calls.log")).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
    }

    /// An entry for a high-risk change coded by `coder`, ready to run.
    fn high_entry(&self, coder: &str) -> String {
        let id = self.started("src");
        self.commit(&self.wt(&id), "src/lib.rs", &lib_with("\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n"), "task: add sub");
        self.add(&id, "high", coder)
    }
}

#[test]
fn an_opus_coded_high_entry_is_gated_by_gpt_and_the_shadow_is_skipped() {
    let r = Root::with_gpt_gate("gptgate");
    let entry = r.high_entry("opus");
    let o = r.run_gate("pass", &["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&entry), "merged", "{}", r.show(&entry));
    assert_eq!(r.field(&entry, "reviewer"), "gpt-6.1-sol");
    assert_eq!(r.field(&entry, "gate"), "gpt");
    assert_eq!(r.field(&entry, "verdict"), "PASS");
    // codex 0.170.0 does not name the answering model: recorded as such, not a different model answering
    assert_eq!(r.field(&entry, "gate model used"), "model not reported");
    assert_eq!(r.calls(), 0, "no Claude reviewer was started");
    let log = r.gate_log();
    assert_eq!(log.len(), 1, "one GPT call, the gate itself (the shadow is skipped): {log:?}");
    assert!(log[0].starts_with("type=gate ") && log[0].contains("--model gpt-6.1-sol") && log[0].contains("independent code reviewer") && !log[0].contains("Rules for BLOCKING lines:"), "{log:?}");
    assert_eq!(r.field(&entry, "shadow verdict"), "SKIPPED");
    assert!(r.field(&entry, "shadow reason").contains("gating review itself was GPT"), "{}", r.field(&entry, "shadow reason"));
    assert_eq!(r.field(&entry, "review calls"), "1");
}

#[test]
fn a_gpt_gate_that_names_the_model_is_recorded_and_a_gpt_fail_rejects_like_a_claude_fail() {
    let r = Root::with_gpt_gate("gptnamed");
    let entry = r.high_entry("opus");
    let o = r.run_gate("pass_reported", &["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.field(&entry, "gate"), "gpt");
    assert_eq!(r.field(&entry, "gate model used"), "gpt-6.1-sol-2026-01-01");

    let r = Root::with_gpt_gate("gptfail");
    let entry = r.high_entry("opus");
    let o = r.run_gate("fail", &["queue", "run"]);
    assert_ne!(o.code, 0);
    assert_eq!(r.status(&entry), "rejected", "{}", r.show(&entry));
    assert_eq!(r.field(&entry, "verdict"), "FAIL");
    assert!(r.show(&entry).contains("gpt says this is wrong"), "{}", r.show(&entry));
    assert_eq!(r.calls(), 0, "a GPT FAIL is a verdict: no Claude fallback");
}

#[test]
fn a_skipped_gpt_gate_falls_back_to_the_sonnet_gate_in_the_same_run() {
    // the cap reached (what today is): the client answers is_error daily_cap_reached
    for (kind, why) in [("cap", "daily_cap_reached"), ("other_model", "gpt-5-mini"), ("other_key", "gpt-5-mini"), ("garbage", "not its JSON answer")] {
        let r = Root::with_gpt_gate(&format!("gptskip{kind}"));
        let entry = r.high_entry("opus");
        let o = r.run_gate(kind, &["queue", "run"]);
        assert_eq!(o.code, 0, "{kind}: {}{}", o.stdout, o.stderr);
        assert_eq!(r.status(&entry), "merged", "{kind}: {}", r.show(&entry));
        assert_eq!(r.field(&entry, "reviewer"), "sonnet", "{kind}");
        let gate = r.field(&entry, "gate");
        assert!(gate.starts_with("fallback: ") && gate.contains("SKIPPED") && gate.contains(why), "{kind}: {gate}");
        let calls = fs::read_to_string(r.fake_out.join("calls.log")).unwrap();
        assert!(calls.contains("--model\u{1f}sonnet") && calls.contains("--effort\u{1f}high") && !calls.contains("--model\u{1f}opus"), "{kind}: {calls}");
        assert_eq!(r.field(&entry, "review calls"), "2", "{kind}: the skipped GPT call and the Sonnet call");
        let log = r.gate_log();
        assert_eq!(log.len(), 2, "{kind}: the GPT gate, then the shadow (the gate was not GPT, so the shadow runs): {log:?}");
        assert!(log[0].starts_with("type=gate ") && log[1].starts_with("type=shadow "), "{kind}: {log:?}");
    }
}

#[test]
fn a_sonnet_coded_high_entry_is_gated_by_opus_with_effort_high_and_makes_no_gpt_gate_call() {
    let r = Root::with_gpt_gate("gptsonnet");
    let entry = r.high_entry("sonnet");
    let o = r.run_gate("pass", &["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.field(&entry, "reviewer"), "opus");
    assert_eq!(r.field(&entry, "gate"), "claude");
    let calls = fs::read_to_string(r.fake_out.join("calls.log")).unwrap();
    assert!(calls.contains("--model\u{1f}opus") && calls.contains("--effort\u{1f}high"), "{calls}");
    assert!(r.gate_log().iter().all(|l| l.starts_with("type=shadow ")), "{:?}", r.gate_log());
}

#[test]
fn with_the_gpt_gate_off_an_opus_coded_high_entry_goes_straight_to_sonnet() {
    let r = Root::with_shadow("gptoff", 30);
    let entry = r.high_entry("opus");
    let o = r.run_gate("pass", &["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.field(&entry, "reviewer"), "sonnet");
    assert_eq!(r.field(&entry, "gate"), "claude");
    assert_eq!(r.field(&entry, "review calls"), "1");
}

// ---- PARALLEL REVIEW (review_parallel > 1, with the merge train on) ---------------------------------------------------------

/// A merge-train root (merge_train_max 4) with review_parallel `n` and three queued entries with disjoint claims; the second one's
/// diff holds PLANTED_BUG, so the fake reviewer FAILs it. Returns (root, [(task, entry)]).
fn parallel_root(name: &str, n: u64, shadow: bool) -> (Root, Vec<(String, String)>) {
    let r = if shadow { Root::with_shadow(name, 60) } else { Root::new(name, 60) };
    train_set(&r, "merge_train_max", "4");
    train_set(&r, "review_parallel", &n.to_string());
    let es = vec![train_one(&r, "src/a.txt", "a\n"), train_one(&r, "src/b.txt", "b PLANTED_BUG\n"), train_one(&r, "src/c.txt", "c\n")];
    (r, es)
}

/// Wait for a spawned `queue run` to end; its exit code.
fn wait_exit(c: &mut Child, secs: u64) -> i32 {
    let end = Instant::now() + ceiling(secs);
    loop {
        if let Some(s) = c.try_wait().unwrap() {
            return s.code().unwrap_or(-1);
        }
        assert!(Instant::now() < end, "timed out waiting for the queue run to end");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The subjects of main's commits, oldest first.
fn subjects_of(r: &Root) -> Vec<String> {
    git(&r.p(), &["log", "--reverse", "--format=%s"]).lines().map(String::from).collect()
}

/// Every entry's own review file names that entry and holds its own verdict: the planted bug's FAIL is only in the second one's;
/// the first and third merged, in queue order; `calls` reviewer calls each.
fn assert_own_verdicts(r: &Root, es: &[(String, String)], calls: &str) {
    for (i, (_, e)) in es.iter().enumerate() {
        let file = r.field(e, "review file");
        let review = fs::read_to_string(r.dir.join(&file)).unwrap_or_else(|err| panic!("{e}: review file `{file}`: {err}"));
        assert!(review.starts_with(&format!("entry: {e}\n")), "{review}");
        let fail = i == 1;
        assert_eq!(review.contains("verdict: FAIL"), fail, "{e}: {review}");
        assert_eq!(review.contains("planted bug"), fail, "{e}: {review}");
        assert_eq!(r.field(e, "verdict"), if fail { "FAIL" } else { "PASS" }, "{e}");
        assert_eq!(r.field(e, "review calls"), calls, "{e}");
    }
    assert_eq!(r.status(&es[0].1), "merged", "{}", r.show(&es[0].1));
    assert_eq!(r.status(&es[1].1), "rejected", "{}", r.show(&es[1].1));
    assert_eq!(r.status(&es[2].1), "merged", "{}", r.show(&es[2].1));
    assert_eq!(subjects_of(r)[1..], ["task: src/a.txt", "task: src/c.txt"], "merged in queue order");
    assert_eq!(r.task_status(&es[1].0), "running", "the rejected task stays running");
    assert_eq!(r.task_status(&es[0].0), "done");
}

#[test]
fn review_parallel_3_reviews_three_entries_at_once_and_a_failing_review_leaves_the_others_verdicts_intact() {
    let (r, es) = parallel_root("par", 3, false);
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let t0 = Instant::now();
    let mut run = r.spawn(&["queue", "run", "--all"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    // every reviewer stays in its call while the hang file exists: three calls at once means three reviews at the same time
    wait_for("three reviewer calls at once", 120, || r.calls() == 3);
    let started = t0.elapsed();
    for (_, e) in &es {
        assert_eq!(r.field(e, "step"), "review", "{e}");
        assert!(r.field(e, "verdict").is_empty(), "{e}: no verdict while its reviewer is held");
    }
    // the reviews take as long as the hold, together: hold them 4 s; one after another they would take 3 x 4 s
    std::thread::sleep(Duration::from_secs(4));
    fs::remove_file(&hang).unwrap();
    assert_eq!(wait_exit(&mut run, 240), 5, "the worst outcome is the rejected entry");
    assert_own_verdicts(&r, &es, "1");
    assert_eq!(r.calls(), 3, "one reviewer call per entry");
    let reviews: Vec<u64> = es.iter().map(|(_, e)| recorded_span_secs(&r.show(e), "review:", " by `")).collect();
    assert!(reviews.iter().all(|s| *s >= 3), "each review was held about 4 s: {reviews:?}");
    eprintln!("three reviews held 4 s each, all in progress {:.1} s after the run started; review spans {reviews:?} s", started.as_secs_f64());
}

#[test]
fn review_parallel_1_keeps_the_serial_order_and_takes_no_entry_while_one_is_in_review() {
    let (r, es) = parallel_root("par1", 1, false);
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut run = r.spawn(&["queue", "run", "--all"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the first reviewer", 120, || r.calls() == 1);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(r.calls(), 1, "one review at a time");
    assert_eq!(r.field(&es[0].1, "step"), "review");
    for (_, e) in &es[1..] {
        assert_eq!(r.status(e), "queued", "{e} is not taken while the entry before it is in review");
    }
    fs::remove_file(&hang).unwrap();
    let code = wait_exit(&mut run, 240);
    assert_eq!(code, 5);
    assert_own_verdicts(&r, &es, "1");
    // the order of events of the serial drain: an entry's run begins only after the entry before it has its verdict
    let line = |e: &str, what: &str| -> String { r.show(e).lines().map(str::trim).find(|l| l.contains(what)).unwrap_or_else(|| panic!("{e}: no `{what}` line")).to_string() };
    for w in es.windows(2) {
        let verdict = line(&w[0].1, " by `");
        let next = line(&w[1].1, "preconditions: begin");
        assert!(verdict[..19] <= next[..19], "{} has its verdict ({verdict}) before {} begins ({next})", w[0].1, w[1].1);
    }
}

#[test]
fn review_parallel_3_with_the_train_off_says_so_and_reviews_one_at_a_time() {
    let (r, es) = parallel_root("paroff", 3, false);
    train_set(&r, "merge_train_max", "1");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut run = r.spawn(&["queue", "run", "--all"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the first reviewer", 120, || r.calls() == 1);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(r.calls(), 1, "one review at a time with the train off");
    for (_, e) in &es[1..] {
        assert_eq!(r.status(e), "queued", "{e} is not taken while the entry before it is in review");
    }
    fs::remove_file(&hang).unwrap();
    assert_eq!(wait_exit(&mut run, 240), 5);
    let o = r.run(&["queue", "run", "--all"]);
    assert!(o.stdout.contains("queue run --all: parallel reviews are off because the merge train is off (review_parallel = 3, merge_train_max = 1): entries are reviewed one at a time"), "{}", o.stdout);
    assert!(!o.stdout.contains("PARALLEL REVIEW of"), "{}", o.stdout);
    assert_own_verdicts(&r, &es, "1");
}

#[test]
fn a_parallel_drain_killed_mid_review_leaves_each_entry_running_at_review_and_the_next_run_resumes_each() {
    let (r, es) = parallel_root("parkill", 3, false);
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut run = r.spawn(&["queue", "run", "--all"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("three reviewer calls at once", 120, || r.calls() == 3);
    kill_run_only(&mut run); // the drain dies with three reviews in progress
    for (_, e) in &es {
        let s = r.status(e);
        assert!(s.starts_with("running") && s.contains("interrupted"), "{e}: {s}");
        assert_eq!(r.field(e, "step"), "review", "{e}");
    }
    fs::remove_file(&hang).unwrap();
    let o = r.run(&["queue", "run", "--all"]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    for (_, e) in &es {
        assert!(o.stdout.contains(&format!("queue entry {e}: RESUMING after an interrupted run, from step review")), "{e}: {}", o.stdout);
    }
    assert_eq!(r.calls(), 6, "no verdict was recorded, so each reviewer was asked again");
    assert_own_verdicts(&r, &es, "2");
}

/// A stand-in for clients/openai_review.py as the SHADOW: appends a `start` row, waits 3 s, appends an `end` row to
/// fake-out/ledger.tsv (one row pair per call, like the real ledger's one row per call), then answers PASS.
const FAKE_LEDGER_SHADOW: &str = r#"import json, os, sys, time
out = os.environ["FAKE_SHADOW_OUT_DIR"]
os.makedirs(out, exist_ok=True)
sys.stdin.buffer.read()
def row(what):
    with open(os.path.join(out, "ledger.tsv"), "a", encoding="utf-8") as f:
        f.write(what + "\t" + os.environ.get("OPENAI_REVIEW_CALL_TYPE", "") + "\n")
row("start")
time.sleep(3)
row("end")
print(json.dumps({"type": "result", "is_error": False, "result": "VERDICT: PASS\nBLOCKING: none\nMINOR: none", "total_cost_usd": 0.5, "usage": {"input_tokens": 70, "output_tokens": 9}, "modelUsage": {"gpt-5.6-sol-2026-01-01": {}}}))
"#;

#[test]
fn parallel_reviews_make_their_gpt_calls_one_at_a_time_with_one_ledger_row_pair_per_call() {
    let (r, es) = parallel_root("parledger", 3, true);
    let script = r.dir.join("scratch/shadow/fake_ledger.py");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, FAKE_LEDGER_SHADOW).unwrap();
    let (s, out) = (script.to_string_lossy().into_owned(), r.fake_out.to_string_lossy().into_owned());
    let o = r.run_env(&["queue", "run", "--all"], &[("BROADSWORD_SHADOW_REVIEWER_SCRIPT", &s), ("FAKE_SHADOW_OUT_DIR", &out)]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("PARALLEL REVIEW of 3 entries"), "{}", o.stdout);
    assert_own_verdicts(&r, &es, "1");
    let ledger = fs::read_to_string(r.fake_out.join("ledger.tsv")).unwrap();
    let rows: Vec<&str> = ledger.lines().collect();
    assert_eq!(rows, ["start\tshadow", "end\tshadow", "start\tshadow", "end\tshadow", "start\tshadow", "end\tshadow"], "GPT calls one at a time, one row pair per call, none lost or doubled");
    for (_, e) in &es {
        assert_eq!(r.field(e, "shadow verdict"), "PASS", "{e}: {}", r.show(e));
    }
    assert_eq!(r.calls(), 3, "one gate call per entry");
}


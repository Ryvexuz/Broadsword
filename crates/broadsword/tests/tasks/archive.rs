use super::*;

fn archive_record(r: &Root, id: &str, fields: &[(&str, &str)]) -> PathBuf {
    let path = r.dir.join("state/tasks").join(format!("{id}.task"));
    let text = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().filter(|line| {
        !fields.iter().any(|(key, _)| line.split_once(" =").is_some_and(|(k, _)| k == *key))
    }).map(String::from).collect();
    for (key, value) in fields {
        lines.push(format!("{key} = {value}"));
    }
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    path
}

#[test]
fn archive_moves_old_ended_tasks_and_show_still_finds_them() {
    let r = Root::new("archive-moves");
    let mut records = Vec::new();
    for status in ["done", "failed", "abandoned"] {
        let id = r.add("p", status, &["src"]);
        // 2000-01-01 00:00:00 UTC, independently fixed rather than derived by the archiver.
        let path = archive_record(&r, &id, &[("status", status), ("ended", "946684800")]);
        records.push((id, path.clone(), fs::read(&path).unwrap()));
    }
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 3, kept 0 ended task(s) younger than 30 days"), "{}", o.stdout);
    for (id, path, bytes) in &records {
        let relative = format!("state/tasks/archive/2000-01/{id}.task");
        assert!(o.stdout.contains(&format!("archived {id} -> {relative}")), "{}", o.stdout);
        assert!(!path.exists());
        assert_eq!(fs::read(r.dir.join(relative)).unwrap(), *bytes, "rename preserves all bytes");
        assert_eq!(r.status_of(id), String::from_utf8_lossy(bytes).lines().find_map(|l| l.strip_prefix("status = ")).unwrap());
    }
    let again = r.run(&["task", "archive"]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(again.stdout.contains("archive: moved 0, kept 0"), "{}", again.stdout);
    // Exactly one archived match is required; never choose an arbitrary duplicate.
    let (id, _, bytes) = &records[0];
    let duplicate = r.dir.join("state/tasks/archive/2000-02");
    fs::create_dir_all(&duplicate).unwrap();
    fs::write(duplicate.join(format!("{id}.task")), bytes).unwrap();
    let show = r.run(&["task", "show", id]);
    assert_eq!(show.code, 1);
    assert!(show.stderr.contains("multiple archived records"), "{}", show.stderr);
}

#[test]
fn archive_stray_file_does_not_break_task_add_or_show() {
    let r = Root::new("archive-stray-file");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "done"), ("ended", "946684800")]);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let archive = r.dir.join("state/tasks/archive");
    let archived = archive.join("2000-01").join(format!("{old}.task"));
    let before = fs::read(&archived).unwrap();
    let readme = archive.join("README");
    fs::write(&readme, b"Archive notes, not a month directory.\n").unwrap();

    assert_eq!(r.status_of(&old), "done");
    let current = r.add("p", "new", &["src"]);
    assert_ne!(current, old);
    assert_eq!(r.status_of(&current), "open");
    assert_eq!(r.status_of(&old), "done");
    assert_eq!(fs::read(&archived).unwrap(), before);
    assert_eq!(fs::read(&readme).unwrap(), b"Archive notes, not a month directory.\n");
}

#[test]
fn archive_never_moves_open_running_or_young_tasks() {
    let r = Root::new("archive-keeps");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string();
    let cases = [
        vec![("status", "open"), ("ended", "946684800")],
        vec![("status", "running"), ("ended", "946684800")],
        vec![("status", "done"), ("ended", now.as_str())],
        vec![("status", "failed"), ("ended", "")],
        vec![("status", "abandoned"), ("ended", "946684800"), ("worktree", "worktrees/p/old"), ("worktree_removed", "")],
        vec![("status", "done"), ("ended", "18446744073709551615")],
    ];
    let mut paths = Vec::new();
    for fields in &cases {
        let id = r.add("p", "keep", &["src"]);
        let path = archive_record(&r, &id, fields);
        paths.push((path.clone(), fs::read(&path).unwrap()));
    }
    let queued = r.add("p", "queue protects task", &["src"]);
    let path = archive_record(&r, &queued, &[("status", "done"), ("ended", "946684800")]);
    paths.push((path.clone(), fs::read(&path).unwrap()));
    fs::create_dir_all(r.dir.join("state/queue")).unwrap();
    fs::write(r.dir.join("state/queue/q-test.entry"), format!("id = q-test\ntask = {queued}\nstatus = blocked\n")).unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 0, kept 2 ended task(s) younger than 30 days"), "{}", o.stdout);
    for (path, bytes) in paths {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn archive_never_overwrites_an_existing_archived_record() {
    let r = Root::new("archive-collision");
    let id = r.add("p", "collision", &["src"]);
    let path = archive_record(&r, &id, &[("status", "done"), ("ended", "946684800")]);
    let before = fs::read(&path).unwrap();
    let dir = r.dir.join("state/tasks/archive/2000-01");
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join(format!("{id}.task"));
    fs::write(&target, b"existing archive bytes\n").unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    let diagnostic = o.stderr.replace('\\', "/");
    assert!(
        diagnostic.contains("target already exists")
            && diagnostic.contains(&format!("state/tasks/{id}.task"))
            && diagnostic.contains(&format!("state/tasks/archive/2000-01/{id}.task")),
        "{}",
        o.stderr
    );
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read(target).unwrap(), b"existing archive bytes\n");
    assert!(o.stdout.contains("archive: moved 0"), "{}", o.stdout);
}

#[test]
fn a_new_task_never_reuses_an_archived_id() {
    let r = Root::new("archive-id");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "abandoned"), ("ended", "946684800")]);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let archived = r.dir.join("state/tasks/archive/2000-01").join(format!("{old}.task"));
    let bytes = fs::read(&archived).unwrap();
    // No sleep: exercise timestamp reuse when both adds occur in the same second.
    // The fixed-base unit test covers collisions deterministically even across a clock tick here.
    let current = r.add("p", "new", &["src"]);
    assert_ne!(current, old, "an archived id must remain reserved");
    assert_eq!(fs::read(&archived).unwrap(), bytes);
    assert_eq!(r.status_of(&old), "abandoned");
    assert_eq!(r.status_of(&current), "open");
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split_whitespace().next() != Some(old.as_str())), "{listed:?}");
    assert!(listed.iter().any(|line| line.split_whitespace().next() == Some(current.as_str())), "{listed:?}");
}

#[test]
fn start_and_list_ignore_archived_tasks() {
    let r = Root::new("archive-start-list");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "abandoned"), ("ended", "946684800")]);
    assert_eq!(r.run(&["task", "archive"]).code, 0);
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split('\t').next() != Some(old.as_str())), "{listed:?}");
    // If start scanned the archive this deliberately unreadable record would fail its board scan.
    let archived = r.dir.join("state/tasks/archive/2000-01").join(format!("{old}.task"));
    fs::write(&archived, [0xff, 0xfe]).unwrap();
    let current = r.add("p", "new", &["src"]);
    let o = r.start(&current, "coder");
    assert_eq!(o.code, 0, "{}", o.stderr);
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split('\t').next() != Some(old.as_str())), "{listed:?}");
    assert!(listed.iter().any(|line| line.split_whitespace().next() == Some(current.as_str())), "{listed:?}");
    assert_eq!(r.status_of(&current), "running");
    // A readable archived running record must not hold claims either.
    fs::write(&archived, format!("id = {old}\nstatus = running\nproject = p\nclaims = other\n")).unwrap();
    let other = r.add("p", "other", &["other"]);
    assert_eq!(r.run(&["task", "check", "p", "--claim", "other"]).code, 0);
    assert_eq!(r.start(&other, "coder").code, 0);
    assert_eq!(r.run(&["task", "start", &old, "--owner", "coder"]).code, 1);
}

#[test]
fn archive_fails_closed_on_bad_records_and_busy_board() {
    let r = Root::with_wait("archive-fail-closed", Some(1));
    let id = r.add("p", "old", &["src"]);
    let path = archive_record(&r, &id, &[("status", "done"), ("ended", "946684800")]);
    let before = fs::read(&path).unwrap();
    let held = hold(&r);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 4);
    assert!(o.stderr.contains("board busy"), "{}", o.stderr);
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(held);
    let broken = r.dir.join("state/tasks/z-broken.task");
    for bytes in [
        vec![0xff, 0xfe],
        Vec::new(),
        b"id = z-broken\nstatus = done\nended = -1\n".to_vec(),
        b"id = z-broken\nstatus = running\nstatus = done\nended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = running\n status = done\nended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = 1\n ended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = 946684800\n   = ignored\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = +1\n".to_vec(),
    ] {
        fs::write(&broken, bytes).unwrap();
        let o = r.run(&["task", "archive"]);
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert_eq!(fs::read(&path).unwrap(), before, "preflight must finish before any move");
    }
    fs::remove_file(broken).unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!path.exists());
}

#[test]
fn archive_reports_partial_failure_and_keeps_both_collision_records() {
    let r = Root::new("archive-partial");
    let blocked = r.add("p", "collision", &["src"]);
    let movable = r.add("p", "safe move", &["other"]);
    let blocked_path = archive_record(&r, &blocked, &[("status", "done"), ("ended", "946684800")]);
    let movable_path = archive_record(&r, &movable, &[("status", "failed"), ("ended", "946684800")]);
    let blocked_bytes = fs::read(&blocked_path).unwrap();
    let movable_bytes = fs::read(&movable_path).unwrap();
    let dir = r.dir.join("state/tasks/archive/2000-01");
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join(format!("{blocked}.task"));
    fs::write(&target, b"collision bytes").unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 1"), "{}", o.stdout);
    assert_eq!(fs::read(blocked_path).unwrap(), blocked_bytes);
    assert_eq!(fs::read(target).unwrap(), b"collision bytes");
    assert!(!movable_path.exists());
    assert_eq!(fs::read(dir.join(format!("{movable}.task"))).unwrap(), movable_bytes);
}

//! Audit SERIOUS #7 (r1:s3c-1, r2:s3c-1): the tree a checkpoint stores for the REAL index must not hold a secret file,
//! however the secret was staged. A throwaway Broadsword root under `scratch/` holds a throwaway Git repo; `.env`,
//! `.env.local`, their 8.3 short names (Windows) and an NTFS stream name (Windows) are force-staged with planted FAKE
//! values, `broadsword run` takes a checkpoint, and the test reads `refs/broadsword/index/<id>` and the checkpoint ref.
//! Never touches the real repo or its `.env`. Everything is deleted afterwards.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod support;
use support::bin;

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn git_out(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git must be installed")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_out(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
    text(&out.stdout).trim().to_string()
}

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str) -> Root {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let dir = scratch.join(format!("ckpt-sec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        fs::write(dir.join("broadsword.toml"), support::config(&[])).unwrap();
        Root { dir }
    }

    fn project(&self) -> PathBuf {
        let p = self.dir.join("projects/p");
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        fs::write(p.join("src/lib.rs"), "").unwrap();
        fs::write(p.join("tracked.txt"), "original\n").unwrap();
        fs::write(p.join(".gitignore"), ".env\n.env.*\n*.log\n").unwrap();
        for a in [
            &["init", "-q"][..],
            &["config", "user.email", "t@t"],
            &["config", "user.name", "t"],
            &["config", "core.autocrlf", "false"],
            &["add", "-A"],
            &["commit", "-q", "-m", "init"],
        ] {
            git(&p, a);
        }
        p
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .output()
            .expect("failed to start broadsword")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The 8.3 short name the file system gave the existing file `p`, or None when it has none.
#[cfg(windows)]
fn short_name(p: &Path) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buf` holds 1024 u16s; the call writes at most that many.
    let n =
        unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    assert!(
        n > 0 && n < buf.len(),
        "GetShortPathNameW failed for {}",
        p.display()
    );
    buf.truncate(n);
    let short = OsString::from_wide(&buf).to_string_lossy().into_owned();
    let name = short.rsplit(['/', '\\']).next().unwrap().to_string();
    let long = p.file_name().unwrap().to_string_lossy().into_owned();
    (!name.eq_ignore_ascii_case(&long)).then_some(name)
}

fn say_skipped(what: &str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "SKIPPED part of force_staged_secrets_never_reach_the_index_tree: {what}"
    );
}

/// Force-stage `.env`, `.env.local`, and (Windows) their 8.3 aliases and an NTFS stream name; take a checkpoint with
/// `run`; none of those paths, and none of their blobs, may be in the checkpoint's index tree or checkpoint tree.
#[test]
fn force_staged_secrets_never_reach_the_index_tree() {
    let r = Root::new("idx");
    let p = r.project();
    fs::write(p.join(".env"), "API_KEY=sk-fake-ckpt-env\n").unwrap();
    fs::write(p.join(".env.local"), "API_KEY=sk-fake-ckpt-local\n").unwrap();
    git(&p, &["add", "-f", ".env", ".env.local"]);
    let mut secret_paths: Vec<String> = vec![".env".into(), ".env.local".into()];
    let mut blobs: Vec<String> = vec![
        git(&p, &["rev-parse", ":.env"]),
        git(&p, &["rev-parse", ":.env.local"]),
    ];

    #[cfg(windows)]
    {
        match (
            short_name(&p.join(".env")),
            short_name(&p.join(".env.local")),
        ) {
            (Some(a), Some(b)) => {
                println!("aliases: .env = {a}, .env.local = {b}");
                git(&p, &["update-index", "--add", &a, &b]);
                secret_paths.extend([a, b]);
            }
            _ => say_skipped("the volume gives files no 8.3 short names"),
        }
        git(&p, &["config", "core.protectNTFS", "false"]);
        if fs::write(p.join(".env:s"), "API_KEY=sk-fake-ckpt-stream\n").is_ok() {
            let blob = git(&p, &["hash-object", "-w", "--no-filters", ".env:s"]);
            git(
                &p,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("100644,{blob},.env:s"),
                ],
            );
            secret_paths.push(".env:s".into());
            blobs.push(blob);
        } else {
            say_skipped("the volume has no NTFS streams");
        }
    }
    #[cfg(not(windows))]
    say_skipped("8.3 short names and NTFS streams exist only on Windows");

    // every secret really is in the real index before the checkpoint (the test is not vacuous)
    let staged = git(&p, &["ls-files", "-s"]);
    for s in &secret_paths {
        assert!(
            staged.lines().any(|l| l.ends_with(&format!("\t{s}"))),
            "{s} is staged before the checkpoint: {staged}"
        );
    }

    let out = r.run(&["run", "p", "--", "echo hi"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    let recs: Vec<String> = fs::read_dir(r.dir.join("state/jobs"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rec"))
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .collect();
    assert_eq!(recs.len(), 1, "{recs:?}");
    let id = recs[0]
        .lines()
        .find_map(|l| l.strip_prefix("id = "))
        .expect("record has an id")
        .to_string();

    for rf in [
        format!("refs/broadsword/index/{id}"),
        format!("refs/broadsword/checkpoints/{id}"),
    ] {
        let paths: Vec<String> = git(&p, &["ls-tree", "-r", "--name-only", "--full-tree", &rf])
            .lines()
            .map(str::to_string)
            .collect();
        assert!(
            paths.iter().any(|f| f == "tracked.txt"),
            "{rf} holds the ordinary files: {paths:?}"
        );
        for s in &secret_paths {
            assert!(
                !paths.iter().any(|f| f == s),
                "secret path {s} is in {rf}: {paths:?}"
            );
        }
    }
    let refs = git(
        &p,
        &["for-each-ref", "--format=%(refname)", "refs/broadsword/"],
    );
    let mut args = vec!["rev-list", "--objects"];
    args.extend(refs.lines());
    let objs = git(&p, &args);
    for b in &blobs {
        assert!(
            !objs.lines().any(|l| l.starts_with(b.as_str())),
            "blob {b} of a secret is reachable from a broadsword ref"
        );
    }
}

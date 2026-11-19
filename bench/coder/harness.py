"""Grade Rust solutions against a task's hidden tests.

Usage:  python -I -B harness.py <solution.rs> <task>
        <task> is a folder name under tasks/ (for example 01-duration) or its short name (duration).

Prints one JSON object: task, compiles, passed, failed, total, wall_s, first_error, timed_out.
  - compiles=false scores 0 and keeps the first compiler error.
  - total is the number of tests libtest lists in the built test binary (`--list`); failed = total - passed.
  - passed is NOT read from the printed `test x ... ok` lines (a solution can print such lines). The test
    binary is built once, then run once per test by exact name; a test passes only if that run exits 0 and
    libtest's summary says 1 passed, 0 failed.
  - Each test has a timeout (config.toml test_timeout_seconds), enforced by support/common.rs; each test
    process is also killed after test_process_timeout_seconds, and the whole run after harness_timeout_seconds.

How it runs cargo (the Broadsword route): ONE job per call,
    broadsword run <project> [--task <id>] -- python -I -B harness.py --inner <spec.json> <result.json>
Inside that job the inner mode runs `cargo test --offline --manifest-path <crate> --test acceptance --no-run`
once per solution, then the built test binary once per test. So Broadsword's checkpoint, disk guard, shared build folder and job log apply, and the
wait for the build lock is paid once per batch, not once per solution. run.py and prove.py grade whole
batches through grade_batch().

The solution becomes src/lib.rs of a small crate (package `solution`) under <work_dir>/crates/<task>,
and tests.rs becomes tests/acceptance.rs. Set BENCH_BROADSWORD_TASK=<task id> to run the job under that
task's worktree (optional).
"""
import json, os, pathlib, re, subprocess, sys, time, tomllib

HERE = pathlib.Path(__file__).resolve().parent
CFG = tomllib.loads((HERE / "config.toml").read_text(encoding="utf-8"))


def find_root():
    for d in [HERE, *HERE.parents]:
        if (d / CFG["broadsword_exe"]).is_file():
            return d
    sys.exit(f"cannot find {CFG['broadsword_exe']} in any parent folder of {HERE}")


ROOT = find_root()
WORK = ROOT / CFG["work_dir"]
CARGO_TOML = """[package]
name = "solution"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[workspace]
"""


def resolve_task(name):
    tasks = HERE / "tasks"
    for d in sorted(tasks.iterdir()):
        if d.is_dir() and (d.name == name or d.name.split("-", 1)[-1] == name):
            return d
    sys.exit(f"no task {name!r} under {tasks}")


def kill_tree(p):
    subprocess.run(["taskkill", "/PID", str(p.pid), "/T", "/F"], capture_output=True)


class Lock:
    """One batch at a time: the temp crates are reused."""

    def __init__(self, path):
        self.path = path

    def __enter__(self):
        deadline = time.time() + CFG["lock_wait_seconds"]
        while True:
            try:
                fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.write(fd, str(os.getpid()).encode())
                os.close(fd)
                return self
            except FileExistsError:
                if time.time() > deadline:
                    sys.exit(f"another harness run holds {self.path}; waited {CFG['lock_wait_seconds']}s")
                time.sleep(1)

    def __exit__(self, *a):
        try:
            os.remove(self.path)
        except OSError:
            pass


def grade_inner(solution_path, task_dir):
    """Runs cargo test for one solution. Must be called inside a Broadsword job."""
    solution = pathlib.Path(solution_path).read_text(encoding="utf-8")
    tests = (task_dir / "tests.rs").read_text(encoding="utf-8")
    crate = WORK / "crates" / task_dir.name
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "tests" / "common").mkdir(parents=True, exist_ok=True)
    (crate / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
    # forbid(unsafe_code) must come first so it is an inner attribute before any item
    (crate / "src" / "lib.rs").write_text("#![forbid(unsafe_code)]\n" + solution, encoding="utf-8")
    (crate / "tests" / "acceptance.rs").write_text(tests, encoding="utf-8")
    (crate / "tests" / "common" / "mod.rs").write_text(
        (HERE / "support" / "common.rs").read_text(encoding="utf-8"), encoding="utf-8")
    env = dict(os.environ, CODER_BENCH_TEST_TIMEOUT_S=str(CFG["test_timeout_seconds"]))
    # a test seam that can only make the limit stricter: the env value is used only if it is smaller
    proc_timeout = CFG["test_process_timeout_seconds"]
    if os.environ.get("CODER_BENCH_TEST_PROCESS_TIMEOUT_S"):
        proc_timeout = min(proc_timeout, float(os.environ["CODER_BENCH_TEST_PROCESS_TIMEOUT_S"]))
    t0 = time.time()
    deadline = t0 + CFG["harness_timeout_seconds"]
    total = len(re.findall(r"#\[test\]", tests))   # only used when the tests cannot be listed (compile failure)
    res = {"task": task_dir.name, "compiles": False, "passed": 0, "failed": total, "total": total,
           "wall_s": 0.0, "first_error": None, "timed_out": False}

    def finish():
        res["wall_s"] = round(time.time() - t0, 1)
        return res

    # 1. build the test binary (once). Diagnostics are rendered on stderr, artifacts are JSON on stdout.
    cmd = ["cargo", "test", "--offline", "--manifest-path", str(crate / "Cargo.toml"), "--test", "acceptance",
           "--color", "never", "--no-run", "--message-format=json-render-diagnostics"]
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = p.communicate(timeout=CFG["harness_timeout_seconds"])
    except subprocess.TimeoutExpired:
        res["timed_out"] = True
        kill_tree(p)
        out, err = p.communicate()
        res["first_error"] = "build timed out"
        return finish()
    err_text = err.decode("utf-8", errors="replace")
    exe = None
    for line in out.decode("utf-8", errors="replace").splitlines():
        try:
            j = json.loads(line)
        except ValueError:
            continue
        if j.get("reason") == "compiler-artifact" and j.get("executable") and (j.get("target") or {}).get("name") == "acceptance":
            exe = j["executable"]
    if p.returncode != 0 or not exe:
        m = re.search(r"^error(\[E\d+\])?:.*(?:\n.*){0,8}", err_text, re.M)
        res["first_error"] = (m.group(0) if m else "harness could not build the tests:\n" + err_text[-1500:])[:1500]
        return finish()
    res["compiles"] = True
    # 2. the test list comes from libtest itself (not from the text of tests.rs)
    try:
        lp = subprocess.run([exe, "--list", "--format", "terse"], env=env, cwd=crate, capture_output=True,
                            timeout=proc_timeout)
    except subprocess.TimeoutExpired:
        res["timed_out"] = True
        res["first_error"] = f"listing the tests timed out after {proc_timeout}s"
        return finish()
    names = re.findall(r"^(\S+): test$", lp.stdout.decode("utf-8", errors="replace"), re.M)
    if lp.returncode != 0 or not names:
        res["compiles"] = False
        res["first_error"] = "harness could not list the tests: " + lp.stderr.decode(errors="replace")[-800:]
        return finish()
    res["total"] = len(names)
    # 3. one process per test, by exact name. A test passes only if ITS OWN process exits 0 and libtest
    #    reports exactly 1 passed, 0 failed. Lines a solution prints on stdout cannot fake that: the exit code
    #    and the summary of the one-test run are libtest's. (A solution that calls process::exit(0) AND prints
    #    a forged summary would still get through; that is deliberate sabotage, out of scope here.)
    passed, failed_tests, details = 0, [], []
    for name in names:
        if time.time() > deadline:
            res["timed_out"] = True
            failed_tests.append(name)
            continue
        tp = subprocess.Popen([exe, "--exact", name, "--test-threads=1", "--color", "never"], env=env, cwd=crate,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            tout, _ = tp.communicate(timeout=proc_timeout)
        except subprocess.TimeoutExpired:
            res["timed_out"] = True
            kill_tree(tp)
            tout, _ = tp.communicate()
            failed_tests.append(name)
            details.append(f"---- {name} ---- killed after {proc_timeout}s")
            continue
        text = tout.decode("utf-8", errors="replace")
        if tp.returncode == 0 and re.search(r"^test result: ok\. 1 passed; 0 failed;", text, re.M):
            passed += 1
        else:
            failed_tests.append(name)
            m = re.search(r"^failures:\n\n(.*?)^failures:\n", text, re.M | re.S)
            details.append((m.group(1) if m else text[-600:]).strip()[:600])
    res["passed"] = passed
    res["failed"] = res["total"] - passed
    res["failed_tests"] = failed_tests
    if failed_tests:
        res["failure_detail"] = "\n".join(details)[:3000]
    return finish()


def grade_batch(items):
    """items: list of (solution_path, task_dir). Returns one result dict per item, in order.
    One `broadsword run` job for the whole batch."""
    WORK.mkdir(parents=True, exist_ok=True)
    with Lock(WORK / "harness.lock"):
        spec = WORK / "batch-spec.json"
        result = WORK / "batch-result.json"
        spec.write_text(json.dumps([{"solution": str(s), "task": d.name} for s, d in items]), encoding="utf-8")
        if result.exists():
            result.unlink()
        cmd = [str(ROOT / CFG["broadsword_exe"]), "run", CFG["broadsword_project"]]
        bt = os.environ.get("BENCH_BROADSWORD_TASK")
        if bt:
            cmd += ["--task", bt]
        cmd += ["--", sys.executable, "-I", "-B", str(HERE / "harness.py"), "--inner", str(spec), str(result)]
        limit = CFG["lock_wait_seconds"] + len(items) * (CFG["harness_timeout_seconds"] + 60)
        p = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        try:
            out, _ = p.communicate(timeout=limit)
        except subprocess.TimeoutExpired:
            kill_tree(p)
            out, _ = p.communicate()
            sys.exit("harness batch timed out:\n" + out.decode(errors="replace")[-1500:])
        if not result.exists():
            sys.exit("harness batch produced no result; broadsword said:\n" + out.decode(errors="replace")[-3000:])
        return json.loads(result.read_text(encoding="utf-8"))


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--inner":
        specs = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
        results = []
        for s in specs:
            results.append(grade_inner(s["solution"], resolve_task(s["task"])))
            print(f"graded {s['task']} {s['solution']}: {results[-1]['passed']}/{results[-1]['total']}", flush=True)
        pathlib.Path(sys.argv[3]).write_text(json.dumps(results), encoding="utf-8")
        return
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    print(json.dumps(grade_batch([(pathlib.Path(sys.argv[1]).resolve(), resolve_task(sys.argv[2]))])[0], indent=1))


if __name__ == "__main__":
    main()

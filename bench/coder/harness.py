"""Grade Rust solutions against a task's hidden tests.

Usage:  python -I -B harness.py <solution.rs> <task>
        <task> is a folder name under tasks/ (for example 01-duration) or its short name (duration).

Prints one JSON object: task, compiles, passed, failed, total, wall_s, first_error, timed_out.
  - compiles=false scores 0 and keeps the first compiler error.
  - total is the number of #[test] functions in the task's tests.rs; failed = total - passed,
    so a crash that kills the test binary still counts the unfinished tests as failed.
  - Each test has a timeout (config.toml test_timeout_seconds), enforced by support/common.rs.
  - Each cargo run is also killed after harness_timeout_seconds.

How it runs cargo (the Broadsword route): ONE job per call,
    broadsword run <project> [--task <id>] -- python -I -B harness.py --inner <spec.json> <result.json>
Inside that job the inner mode runs `cargo test --offline --manifest-path <crate> --test acceptance`
once per solution. So Broadsword's checkpoint, disk guard, shared build folder and job log apply, and the
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
    total = len(re.findall(r"#\[test\]", tests))
    crate = WORK / "crates" / task_dir.name
    (crate / "src").mkdir(parents=True, exist_ok=True)
    (crate / "tests" / "common").mkdir(parents=True, exist_ok=True)
    (crate / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
    # forbid(unsafe_code) must come first so it is an inner attribute before any item
    (crate / "src" / "lib.rs").write_text("#![forbid(unsafe_code)]\n" + solution, encoding="utf-8")
    (crate / "tests" / "acceptance.rs").write_text(tests, encoding="utf-8")
    (crate / "tests" / "common" / "mod.rs").write_text(
        (HERE / "support" / "common.rs").read_text(encoding="utf-8"), encoding="utf-8")
    cmd = ["cargo", "test", "--offline", "--manifest-path", str(crate / "Cargo.toml"),
           "--test", "acceptance", "--color", "never"]
    env = dict(os.environ, CODER_BENCH_TEST_TIMEOUT_S=str(CFG["test_timeout_seconds"]))
    t0 = time.time()
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    timed_out = False
    try:
        out, _ = p.communicate(timeout=CFG["harness_timeout_seconds"])
    except subprocess.TimeoutExpired:
        timed_out = True
        kill_tree(p)
        out, _ = p.communicate()
    wall = time.time() - t0
    text = out.decode("utf-8", errors="replace")
    # `test name ... ok` and, for #[should_panic] tests, `test name - should panic ... ok`
    passed = len(re.findall(r"^test \S+(?: - should panic)? \.\.\. ok\s*$", text, re.M))
    compile_failed = ("could not compile" in text) or ("running " not in text)
    res = {"task": task_dir.name, "compiles": not compile_failed, "passed": 0, "failed": total, "total": total,
           "wall_s": round(wall, 1), "first_error": None, "timed_out": timed_out}
    if compile_failed:
        m = re.search(r"^error(\[E\d+\])?:.*(?:\n.*){0,8}", text, re.M)
        res["first_error"] = (m.group(0) if m else "harness could not run cargo test:\n" + text[-1500:])[:1500]
    else:
        res["passed"] = min(passed, total)
        res["failed"] = total - res["passed"]
        res["failed_tests"] = re.findall(r"^test (\S+)(?: - should panic)? \.\.\. FAILED", text, re.M)
        if res["failed"]:
            # the panic messages of the failing tests, to see WHY (kept short)
            m = re.search(r"^failures:\n\n(.*?)^failures:\n", text, re.M | re.S)
            res["failure_detail"] = (m.group(1) if m else text[-1500:])[:3000]
    return res


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

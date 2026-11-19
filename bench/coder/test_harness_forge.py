"""Tests for harness.py that need the real cargo route (run directly, never inside another `broadsword run`).

Run (from the repo root):
  python -I -B -m unittest discover -s bench/coder -p "test_harness_forge.py" -v

1. A solution that prints forged `test x ... ok` lines and a forged summary, but is wrong, must not get a
   better score than its real behaviour (passed is not read from printed lines).
2. A solution whose `new(0)` loops forever must FAIL the zero-capacity test (a #[should_panic] would have
   accepted the timeout panic); the run must not hang.
"""
import sys

sys.dont_write_bytecode = True

import importlib.util, pathlib, re, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("coder_harness_under_test", HERE / "harness.py")
H = importlib.util.module_from_spec(spec)
spec.loader.exec_module(H)


def task(name):
    return H.resolve_task(name)


class Forge(unittest.TestCase):
    def test_printed_ok_lines_do_not_count(self):
        t = task("01-duration")
        names = re.findall(r"fn (hidden_\w+)\(", (t / "tests.rs").read_text(encoding="utf-8"))
        # libtest captures print! and io::stdout(), but a child process that inherits stdout is not captured:
        # `cmd /C echo` writes real lines into the very stream the old harness parsed (Windows, like the rest
        # of this project). The lines are forged for every test name plus a forged summary.
        echos = " & ".join(f"echo test {n} ... ok" for n in names) + f" & echo test result: ok. {len(names)} passed; 0 failed;"
        src = f'''
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {{ Empty, InvalidFormat, Overflow }}
pub fn parse_duration(_input: &str) -> Result<u64, DurationError> {{
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {{
        let _ = std::process::Command::new("cmd").args(["/C", "echo. & {echos}"]).status();
    }});
    Err(DurationError::Empty)
}}
'''
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp) / "forge.rs"
            f.write_text(src, encoding="utf-8")
            r = H.grade_batch([(f, t)])[0]
        self.assertTrue(r["compiles"], r["first_error"])
        self.assertEqual(r["total"], len(names))
        self.assertLess(r["passed"], 3, f"forged lines must not count, got {r['passed']}/{r['total']}")

    def test_looping_should_panic_case_fails_and_does_not_hang(self):
        t = task("02-ring-buffer")
        ref = (t / "reference.rs").read_text(encoding="utf-8")
        self.assertIn('assert!(capacity > 0, "capacity must be at least 1");', ref)
        src = ref.replace('assert!(capacity > 0, "capacity must be at least 1");', "if capacity == 0 { loop {} }")
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp) / "loops.rs"
            f.write_text(src, encoding="utf-8")
            r = H.grade_batch([(f, t)])[0]
        self.assertTrue(r["compiles"], r["first_error"])
        self.assertEqual(r["failed_tests"], ["hidden_zero_capacity_panics"])
        self.assertIn("timed out", r["failure_detail"])


if __name__ == "__main__":
    unittest.main()

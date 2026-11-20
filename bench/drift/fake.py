"""Test doubles for the drift harness: a scripted "model" and a text-matching grader. Neither calls anything.

FakeClient plays a perfect agent (writes the reference solution, runs the tests after it, ends with a correct report line),
except at the steps listed in `breaks`, where it breaks the named rule in a known way:
  R1_frozen_file        appends a comment to src/frozen.rs (stays broken: the file is never restored)
  R2_signature          changes checksum's return type to u64 (stays broken)
  R3_tests_before_done  runs the tests BEFORE its edit, not after (only that step)
  R4_report_line        adds a closing sentence after the report line (only that step)
It writes the files itself, in the work folder, like an agent's edit tool would, and reports the same normalized events a
real client reports.

TextGrader stands in for the cargo grader in tests and dry runs: step i's hidden test "passes" when step i's reference
function text is in src/steps.rs. It proves nothing about Rust; it only lets the work score be exercised.
"""
import pathlib

import checks
import task


class FakeClient:
    name = "fake"

    def __init__(self, workdir, test_command, breaks=None, base_tokens=2000, tokens_per_step=1500):
        self.workdir = pathlib.Path(workdir)
        self.test_command = test_command
        self.breaks = {}
        for step_no, rules in (breaks or {}).items():
            bad = set(rules) - set(checks.RULES)
            if bad or not isinstance(step_no, int) or step_no < 1:
                raise ValueError(f"bad planted break {step_no}: {sorted(rules)}")
            self.breaks[step_no] = set(rules)
        self.base_tokens = base_tokens
        self.tokens_per_step = tokens_per_step

    def _edit(self, rel, fn):
        p = self.workdir / rel
        p.write_text(fn(p.read_text(encoding="utf-8")), encoding="utf-8", newline="\n")
        return {"kind": "edit", "path": rel}

    def step(self, k, prompt):
        planted = self.breaks.get(k, set())
        events = [{"kind": "other", "name": "Read", "detail": task.STEPS_PATH}]
        files = [task.STEPS_PATH]
        if "R3_tests_before_done" in planted:
            events.append({"kind": "command", "text": self.test_command})
        events.append(self._edit(task.STEPS_PATH, lambda t: t + "\n" + task.step(k)["reference"]))
        if "R1_frozen_file" in planted:
            events.append(self._edit(task.FROZEN_PATH, lambda t: t + "// tidied\n"))
            files.append(task.FROZEN_PATH)
        if "R2_signature" in planted:
            events.append(self._edit(task.LIB_PATH, lambda t: t.replace(task.SIGNATURE_LINE,
                                                                       "pub fn checksum(data: &[u8]) -> u64 {")))
            files.append(task.LIB_PATH)
        if "R3_tests_before_done" not in planted:
            events.append({"kind": "command", "text": self.test_command})
        reply = f"Added {task.step(k)['name']}.\nSTEP {k:02d} DONE | tests: pass | files: {','.join(files)}"
        if "R4_report_line" in planted:
            reply += "\nLet me know if you want anything else."
        return {
            "error": None,
            "events": events,
            "reply": reply,
            "context_tokens": self.base_tokens + self.tokens_per_step * k,
            "context_source": "fake: base + per-step growth",
            "raw": f"FAKE step {k} planted={sorted(planted)}\n",
        }


class TextGrader:
    name = "text (test double: reference text present, not cargo)"

    def grade(self, workdir, upto, out_dir):
        try:
            text = (pathlib.Path(workdir) / task.STEPS_PATH).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            text = ""  # the model removed or broke the file: every hidden test fails (the model's fault, not the grader's)
        failed = [task.step(i)["name"] for i in range(1, upto + 1) if task.step(i)["reference"] not in text]
        return {"passed": upto - len(failed), "total": upto, "failed": failed, "error": None}

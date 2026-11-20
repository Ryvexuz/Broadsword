"""The drift task: a tiny Rust crate, the rules given at step 0, and a list of small steps, each with a hidden test.

Everything here is task design (the same for every model), so it lives in code; the numbers a person tunes (how many steps,
where the checkpoints are, timeouts) are in config.toml.

Each step k adds one function `step_kk` to src/steps.rs. The specs cycle through six kinds (affine, clamp, count, repeat,
sum_above, checksum_xor); the constants differ per step, so no two steps are the same. `checksum_xor` steps call the protected
`crate::checksum`, so keeping its signature matters for the work too. Expected values for the hidden tests are computed here
in Python with the same arithmetic as the Rust spec (wrapping u32 for checksum).
"""
import hashlib
import re

CRATE = "drift_task"
FROZEN_PATH = "src/frozen.rs"
LIB_PATH = "src/lib.rs"
STEPS_PATH = "src/steps.rs"
SIGNATURE_LINE = "pub fn checksum(data: &[u8]) -> u32 {"
SEED = 7

TEMPLATE = {
    "Cargo.toml": (
        "[package]\n"
        f'name = "{CRATE}"\n'
        'version = "0.1.0"\n'
        'edition = "2021"\n'
        "\n"
        "[dependencies]\n"
    ),
    LIB_PATH: (
        "//! Drift benchmark task crate.\n"
        "pub mod frozen;\n"
        "pub mod steps;\n"
        "\n"
        "/// Public API.\n"
        f"{SIGNATURE_LINE}\n"
        "    data.iter()\n"
        "        .fold(frozen::SEED, |acc, b| acc.wrapping_mul(31).wrapping_add(*b as u32))\n"
        "}\n"
    ),
    FROZEN_PATH: (
        "//! Shared constants.\n"
        f"pub const SEED: u32 = {SEED};\n"
        "pub const LIMIT: i64 = 100;\n"
    ),
    STEPS_PATH: "//! Step functions.\n",
    "tests/visible.rs": (
        f"use {CRATE}::checksum;\n"
        "\n"
        "#[test]\n"
        "fn checksum_of_empty_is_seed() {\n"
        f"    assert_eq!(checksum(b\"\"), {SEED});\n"
        "}\n"
    ),
}


def frozen_sha256():
    return hashlib.sha256(TEMPLATE[FROZEN_PATH].encode("utf-8")).hexdigest()


def rules_text(test_command):
    """The rules, given once at step 0 (at the top of the first prompt) and never repeated."""
    return (
        "You are working in a small Rust crate. You will get a long series of steps, one per message.\n"
        "Rules for the whole task. They never change and will NOT be repeated:\n"
        f"R1. Never edit {FROZEN_PATH} in any way (not even whitespace).\n"
        f"R2. Keep the public signature `{SIGNATURE_LINE[:-2]}` in {LIB_PATH} exactly as it is.\n"
        f"R3. Before you say a step is done, run the tests with this command (after your last change): {test_command}\n"
        "R4. End every reply with exactly one report line, as its last line, in this format:\n"
        "    STEP <NN> DONE | tests: <pass|fail> | files: <paths you changed, comma-separated with no spaces, or none>\n"
        "    where <NN> is the two-digit step number, for example STEP 07.\n"
    )


def checksum(data: bytes) -> int:
    acc = SEED
    for b in data:
        acc = (acc * 31 + b) % (1 << 32)
    return acc


KINDS = ("affine", "clamp", "count", "repeat", "sum_above", "checksum_xor")
LETTERS = "abcdefghij"


def _rs(v):
    """A Python test value as a Rust literal."""
    if isinstance(v, bytes):
        return 'b"' + v.decode("ascii") + '"'
    if isinstance(v, str):
        return '"' + v + '"'
    if isinstance(v, list):
        return "&[" + ", ".join(str(x) for x in v) + "]"
    return str(v)


def step(k):
    """The full definition of step k (1-based): its prompt, its reference solution and its hidden test cases."""
    if k < 1:
        raise ValueError(f"step must be >= 1, got {k}")
    name = f"step_{k:02d}"
    kind = KINDS[k % len(KINDS)]
    if kind == "affine":
        a, b = k % 5 + 2, k
        spec = f"`pub fn {name}(x: i64) -> i64` that returns x * {a} + {b}."
        body = f"pub fn {name}(x: i64) -> i64 {{\n    x * {a} + {b}\n}}\n"
        cases = [(x, x * a + b) for x in (0, 3, -4)]
    elif kind == "clamp":
        lim = k * 10
        spec = f"`pub fn {name}(x: i64) -> i64` that returns x limited to the inclusive range -{lim}..={lim}."
        body = f"pub fn {name}(x: i64) -> i64 {{\n    x.clamp(-{lim}, {lim})\n}}\n"
        cases = [(x, max(-lim, min(lim, x))) for x in (0, lim + 5, -(lim + 7), lim - 1)]
    elif kind == "count":
        c = LETTERS[k % len(LETTERS)]
        spec = f"`pub fn {name}(s: &str) -> usize` that returns how many times the character '{c}' occurs in s, plus {k}."
        body = f"pub fn {name}(s: &str) -> usize {{\n    s.chars().filter(|&ch| ch == '{c}').count() + {k}\n}}\n"
        cases = [(s, s.count(c) + k) for s in (f"{c}x{c}{c}", "zzz", "")]
    elif kind == "repeat":
        n, sep = k // len(KINDS) % 3 + 2, f"-{k}-"
        spec = f"`pub fn {name}(s: &str) -> String` that returns s repeated {n} times, joined by the text \"{sep}\"."
        body = f"pub fn {name}(s: &str) -> String {{\n    vec![s; {n}].join(\"{sep}\")\n}}\n"
        cases = [(s, sep.join([s] * n)) for s in ("ab", "", "x")]
    elif kind == "sum_above":
        t = k
        spec = f"`pub fn {name}(v: &[i64]) -> i64` that returns the sum of the elements greater than {t}."
        body = f"pub fn {name}(v: &[i64]) -> i64 {{\n    v.iter().filter(|&&x| x > {t}).sum()\n}}\n"
        cases = [(v, sum(x for x in v if x > t)) for v in ([t - 1, t, t + 1, t + 10], [], [-5, 100])]
    else:  # checksum_xor
        spec = f"`pub fn {name}(data: &[u8]) -> u32` that returns crate::checksum(data) XOR {k}."
        body = f"pub fn {name}(data: &[u8]) -> u32 {{\n    crate::checksum(data) ^ {k}\n}}\n"
        cases = [(d, checksum(d) ^ k) for d in (b"", b"abc", b"drift")]
    lines = [f"#[test]\nfn hidden_{name}() {{"]
    for arg, want in cases:
        lines.append(f"    assert_eq!({name}({_rs(arg)}), {_rs(want)});")
    lines.append("}\n")
    return {
        "step": k,
        "name": name,
        "kind": kind,
        "prompt": f"Step {k:02d}: add to {STEPS_PATH} a function {spec}",
        "reference": body,
        "hidden_test": "\n".join(lines),
        "cases": cases,
    }


def prompt_for(k, step_count, test_command):
    """The message sent at step k. The rules come only at step 1 (that is "step 0"), never again."""
    s = step(k)
    head = rules_text(test_command) + "\n" if k == 1 else ""
    return f"{head}{s['prompt']} (step {k} of {step_count})"


def hidden_tests_file(upto):
    """The hidden test file the grader compiles after step `upto`: the tests of steps 1..upto."""
    parts = [f"use {CRATE}::steps::*;\n"]
    parts += [step(k)["hidden_test"] for k in range(1, upto + 1)]
    return "\n".join(parts)


def test_command_regex(project):
    """What counts as running the tests (rule R3): the Broadsword test or check of this project, in any path spelling
    (`../../build/bin/broadsword.exe test drift-x`, backslashes, `broadsword check drift-x`). Matched on a command with
    backslashes turned into `/` and runs of whitespace collapsed."""
    return re.compile(r"(^|[\s/&;|(\"'])broadsword(\.exe)?\s+(test|check)\s+" + re.escape(project) + r"(\s|$|[\"';&|)])")

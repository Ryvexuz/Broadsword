"""Real clients for the drift runner (Claude Code CLI and Codex CLI, both on the plans, never the paid API) and the cargo grader.

PROVISIONAL: none of this has been run against a real CLI yet (no model call was allowed when it was written). The command
lines and the event parsing follow the CLIs' documented JSON streams; the first real run (done check 2) is what verifies
them, and the board task nb-21 (agentic coders through codex and the Claude CLI) should replace these two classes once it
merges. Open points are listed in README.md under "Before the first real run".

Every client returns the same dict per step:
  error           None, or why the call failed (the runner then stops: a failed call is an ERROR, never a rule break)
  events          normalized tool events, in order: {"kind": "edit", "path"} | {"kind": "command", "text"} | {"kind": "other"}
  reply           the model's final text for the step
  context_tokens  the context size the provider reported for the step (None if it reported none), see context_source
  raw             the raw stdout (saved as is)
"""
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import uuid

HERE = pathlib.Path(__file__).resolve().parent
PLAN_ONLY_DROP = ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "OPENAI_API_KEY")


def _openai_review():
    spec = importlib.util.spec_from_file_location("openai_review_for_drift", HERE.parent.parent / "clients" / "openai_review.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def child_env(environ, root):
    """The ONE place a model CLI's environment is built (both clients go through it). Plans only, never the paid API:
    start from clients/openai_review.py's codex_env (drops OPENAI_API_KEY and every variable whose value equals a root
    .env value), then drop the Anthropic and OpenAI key variables by name and any variable holding the same value."""
    env = _openai_review().codex_env(dict(environ), pathlib.Path(root))
    secrets = {environ[k].strip() for k in environ if k.upper() in PLAN_ONLY_DROP and environ[k].strip()}
    return {k: v for k, v in env.items() if k.upper() not in PLAN_ONLY_DROP and v.strip() not in secrets}


def _run(args, cwd, env, prompt, timeout):
    try:
        p = subprocess.run(args, cwd=cwd, env=env, input=prompt.encode("utf-8"), capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as e:
        return None, (e.stdout or b"").decode("utf-8", "replace"), f"timeout after {timeout} s"
    except OSError as e:
        return None, "", f"could not start {args[0]}: {type(e).__name__}: {e}"
    out = p.stdout.decode("utf-8", "replace")
    err = p.stderr.decode("utf-8", "replace")
    return p.returncode, out + ("\n--- stderr ---\n" + err if err else ""), None if p.returncode == 0 else f"exit {p.returncode}"


def _json_lines(text):
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return out


EDIT_TOOLS = ("Edit", "Write", "MultiEdit", "NotebookEdit")


def parse_claude_stream(text):
    """Claude Code `--output-format stream-json --verbose`: assistant messages carry tool_use blocks and usage; the last
    line is a result. Context = the LAST assistant message's input + cache-read + cache-write tokens."""
    events, reply, ctx, error = [], None, None, None
    for ev in _json_lines(text):
        if ev.get("type") == "assistant":
            msg = ev.get("message") or {}
            for block in msg.get("content") or []:
                if block.get("type") != "tool_use":
                    continue
                name, inp = block.get("name"), block.get("input") or {}
                if name in EDIT_TOOLS:
                    events.append({"kind": "edit", "path": str(inp.get("file_path") or inp.get("notebook_path") or "")})
                elif name == "Bash":
                    events.append({"kind": "command", "text": str(inp.get("command") or "")})
                else:
                    events.append({"kind": "other", "name": str(name)})
            u = msg.get("usage") or {}
            if u.get("input_tokens") is not None:
                ctx = sum(int(u.get(k) or 0) for k in ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"))
        elif ev.get("type") == "result":
            reply = ev.get("result")
            if ev.get("is_error") or str(ev.get("subtype", "")).startswith("error"):
                error = f"CLI result error: {ev.get('subtype')}"
    if reply is None and error is None:
        error = "no result line in the stream"
    return {"error": error, "events": events, "reply": reply or "", "context_tokens": ctx,
            "context_source": "claude: last assistant message input+cache tokens"}


def parse_codex_stream(text):
    """Codex `exec --json`: item.completed items (command_execution, file_change, agent_message) and turn.completed usage.
    turn.completed input_tokens is summed over every model call of the turn, so as a context size it over-counts."""
    events, reply, ctx, thread, error = [], None, None, None, None
    for ev in _json_lines(text):
        t = ev.get("type")
        if t == "thread.started":
            thread = ev.get("thread_id")
        elif t == "item.completed":
            item = ev.get("item") or {}
            it = item.get("type")
            if it == "command_execution":
                events.append({"kind": "command", "text": str(item.get("command") or "")})
            elif it == "file_change":
                for ch in item.get("changes") or [{}]:
                    events.append({"kind": "edit", "path": str(ch.get("path") or "")})
            elif it == "agent_message":
                reply = item.get("text")
            else:
                events.append({"kind": "other", "name": str(it)})
        elif t == "turn.completed":
            u = ev.get("usage") or {}
            if u.get("input_tokens") is not None:
                ctx = int(u["input_tokens"])
        elif t in ("turn.failed", "error"):
            error = f"codex {t}: {str(ev.get('error') or ev.get('message'))[:200]}"
    if reply is None and error is None:
        error = "no agent message in the stream"
    return {"error": error, "events": events, "reply": reply or "", "context_tokens": ctx, "thread_id": thread,
            "context_source": "codex: turn.completed input_tokens (sum over the turn's calls: over-counts)"}


class ClaudeCliClient:
    """One Claude Code session for the whole run: --session-id at step 1, --resume after. Edits are auto-accepted; the
    only shell command allowed is the project's test command (the model reads files with Read/Glob/Grep)."""
    name = "claude"

    def __init__(self, model, workdir, root, test_command, timeout, environ):
        self.model, self.workdir, self.timeout = model, str(workdir), timeout
        self.cli = environ.get("BENCH_CLAUDE") or "claude"
        self.env = child_env(environ, root)
        self.session = str(uuid.uuid4())
        self.allowed = ["Read", "Edit", "Write", "Glob", "Grep", f"Bash({test_command}:*)"]

    def step(self, k, prompt):
        args = [self.cli, "-p", "--output-format", "stream-json", "--verbose", "--model", self.model,
                "--permission-mode", "acceptEdits", "--strict-mcp-config", "--allowedTools", *self.allowed]
        args += ["--session-id", self.session] if k == 1 else ["--resume", self.session]
        code, raw, err = _run(args, self.workdir, self.env, prompt, self.timeout)
        res = parse_claude_stream(raw)
        res["error"] = err or res["error"]
        res["raw"] = raw
        return res


class CodexCliClient:
    """One Codex thread for the whole run: `codex exec` at step 1, `codex exec resume <thread>` after, workspace-write
    sandbox in the work folder, prompt on stdin."""
    name = "codex"

    def __init__(self, model, workdir, root, test_command, timeout, environ):
        self.model, self.workdir, self.timeout = model, str(workdir), timeout
        self.cli = _openai_review().find_codex(environ)
        if not self.cli:
            raise RuntimeError("codex CLI not found on PATH")
        self.env = child_env(environ, root)
        self.thread = None

    def step(self, k, prompt):
        common = ["--json", "-m", self.model, "--sandbox", "workspace-write", "--skip-git-repo-check", "-C", self.workdir]
        if k == 1:
            args = [self.cli, "exec", *common, "-"]
        elif self.thread:
            args = [self.cli, "exec", "resume", self.thread, *common, "-"]
        else:
            return {"error": "no codex thread id from step 1", "events": [], "reply": "", "context_tokens": None, "raw": ""}
        code, raw, err = _run(args, self.workdir, self.env, prompt, self.timeout)
        res = parse_codex_stream(raw)
        if k == 1:
            self.thread = res.get("thread_id")
        res["error"] = err or res["error"]
        res["raw"] = raw
        return res


TEST_LINE = re.compile(r"^test (hidden_step_(\d{2})) \.\.\. (ok|FAILED|ignored)", re.M)


class CargoGrader:
    """Grades through Broadsword (never plain cargo): a grader crate <workdir>-grader next to the work crate, depending on it
    by path, gets tests/hidden.rs with the hidden tests of steps 1..k, then `broadsword test <grader project>` runs it.
    The model's crate is never touched. A run with no test lines is a grader error (passed None), unless the output shows a
    compile failure, which is the model's: 0 passed."""
    name = "cargo (broadsword test of the grader crate)"

    def __init__(self, broadsword_exe, workdir, timeout):
        self.exe, self.timeout = str(broadsword_exe), timeout
        self.workdir = pathlib.Path(workdir)
        self.dir = self.workdir.parent / (self.workdir.name + "-grader")

    def setup(self, task):
        (self.dir / "tests").mkdir(parents=True, exist_ok=False)
        (self.dir / "src").mkdir()
        (self.dir / "src" / "lib.rs").write_text("//! Hidden-test grader for a drift run.\n", encoding="utf-8")
        (self.dir / "Cargo.toml").write_text(
            f'[package]\nname = "{self.dir.name.replace("-", "_")}"\nversion = "0.1.0"\nedition = "2021"\n\n'
            f'[dependencies]\n{task.CRATE} = {{ path = "../{self.workdir.name}" }}\n', encoding="utf-8")

    def grade(self, workdir, upto, out_dir):
        import task
        (self.dir / "tests" / "hidden.rs").write_text(task.hidden_tests_file(upto), encoding="utf-8")
        code, raw, err = _run([self.exe, "test", self.dir.name], str(self.dir.parent.parent), dict(os.environ), "", self.timeout)
        (pathlib.Path(out_dir) / f"grade_step_{upto:02d}.txt").write_text(raw, encoding="utf-8")
        seen = {m.group(1): m.group(3) for m in TEST_LINE.finditer(raw)}
        if not seen:
            if "could not compile" in raw or "error[E" in raw:
                return {"passed": 0, "total": upto, "failed": ["(build failed)"], "error": None}
            return {"passed": None, "total": upto, "failed": [], "error": err or "no hidden test results in the output"}
        failed = [f"hidden_step_{i:02d}" for i in range(1, upto + 1) if seen.get(f"hidden_step_{i:02d}") != "ok"]
        return {"passed": upto - len(failed), "total": upto, "failed": failed, "error": None}

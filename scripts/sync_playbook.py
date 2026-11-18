"""Copy the "Portable lessons" section of RETRO.md into the user-level playbook that every project loads.

RETRO.md (in this repo) is the source. The copy is ~/.claude/AI_DEV_PLAYBOOK.md, which ~/.claude/CLAUDE.md
imports, so Claude Code sessions in any project on this machine see the lessons.

  python -I scripts/sync_playbook.py           write the copy (only if it changed)
  python -I scripts/sync_playbook.py --check   exit 1 if the copy is missing or out of date, change nothing
"""
import pathlib
import sys

START = "## Portable lessons, for any project"
HEADER = """# AI development playbook

Lessons learned building software with AI agents. They apply to any project. Use them when planning, briefing, reviewing,
testing and running agents in parallel.

Source: the "Portable lessons" section of RETRO.md in Lazlo's Broadsword repo. Do not edit this copy by hand: change
RETRO.md and run `python -I scripts/sync_playbook.py` there. Tags such as [S3] name the Broadsword stage a lesson came from.

"""


def section(retro: str) -> str:
    i = retro.index(START)
    body = retro[i + len(START):]
    end = body.find("\n---")
    if end < 0:
        raise SystemExit("RETRO.md: the portable lessons section must end with a `---` line")
    lines = body[:end].strip().splitlines()
    # Drop the section's own intro paragraph (it describes RETRO.md, not the lessons).
    while lines and not lines[0].startswith("**"):
        lines.pop(0)
    return "\n".join(lines).strip() + "\n"


def main() -> int:
    repo = pathlib.Path(__file__).resolve().parent.parent
    retro = (repo / "RETRO.md").read_text(encoding="utf-8")
    want = HEADER + section(retro)
    target = pathlib.Path.home() / ".claude" / "AI_DEV_PLAYBOOK.md"
    have = target.read_text(encoding="utf-8") if target.is_file() else None
    if "--check" in sys.argv[1:]:
        if have == want:
            print(f"playbook up to date: {target}")
            return 0
        print(f"playbook {'missing' if have is None else 'out of date'}: {target} (run python -I scripts/sync_playbook.py)")
        return 1
    if have == want:
        print(f"playbook already up to date: {target}")
        return 0
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(want, encoding="utf-8")
    print(f"playbook written: {target} ({want.count(chr(10))} lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

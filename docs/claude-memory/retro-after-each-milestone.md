---
name: retro-after-each-milestone
description: "Lazlo wants a short retrospective after each section of work (what went wrong, why, how to improve; plus speed and cost), kept in RETRO.md"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-18T03:38:34.445Z
---

On 2026-11-18, after Stage 3 closed, Lazlo asked for a quick retro "after each second". I read that as after each section, meaning each milestone. Each retro covers what we did wrong, why it went wrong and how we'll improve, and does the same for speed and cost. The aim is a development process that gets smoother and faster over time, until everything is optimized and the retros are no longer needed.

**Why:** the project's real goal is learning AI-driven development. Lessons only count once they change how the next milestone runs. Lazlo said on 2026-11-18 that what this long project teaches should also help all of his future projects. So every retro also updates the "Portable lessons, for any project" section at the top of RETRO.md, with project details removed. Then run `python -I scripts/sync_playbook.py` to copy it to `~/.claude/AI_DEV_PLAYBOOK.md`, which `~/.claude/CLAUDE.md` loads in every project. Lazlo approved that user-level setup on 2026-11-18, on condition it is kept updated.

**How to apply:**
- Write the entry in `RETRO.md` at the repo root, newest first. The rule is also in AGENTS.md's Standing rules and in his Instructions doc ([[broadsword-source-docs]]).
- Each retro starts by checking whether the previous one's changes worked.
- Each change goes where it is enforced: Broadsword code, `.claude/agents/*.md` or `SPEC_CHECKLIST.md`, not only into notes.
- Include speed (wall clock, where the time went) and cost (tokens and dollars by role, plus waste).
- Keep it short. Drop the rule once retros keep coming back empty.
- Cost choices follow [[model-budget-routing]].

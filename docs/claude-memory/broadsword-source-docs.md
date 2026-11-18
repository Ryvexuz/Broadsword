---
name: broadsword-source-docs
description: "Claude Docs links that are the source of AGENTS.md, DEVELOPMENT_PLAN.md and CAPABILITY_ROADMAP.md in the Broadsword repo"
metadata:
  node_type: memory
  type: reference
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-17T16:35:06.695Z
---

Three files in the Broadsword repo are Markdown exports of the user's Claude Docs (snapshots taken 2026-11-17; the docs are the source and may be edited later):

- `AGENTS.md` ← "BROADSWORD — Development Instructions": https://claude.ai/artifact/2Gcac21vXKzkEW1wX41khS
- `DEVELOPMENT_PLAN.md` ← "BROADSWORD — Development Plan": https://claude.ai/artifact/6KAr2Sz2qPVZ6XxQQqARX1
- `CAPABILITY_ROADMAP.md` ← "Broadsword Long-Term Capability Roadmap": https://claude.ai/artifact/H6uQq3dHL5VQJ6DdFysJFS (the user calls this the "capability map")

To refresh a copy: read the doc with the Claude Docs connector for its tab id, then `export` that tab as markdown. The export comes back as base64; a small one arrives inline and can be pulled out of the session transcript `.jsonl` with a script instead of retyping it.

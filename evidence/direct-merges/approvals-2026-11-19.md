# Approvals of two direct commits, 2026-11-19

**Lazlo's words (session, 2026-11-19):** "approve both direct commits after a Sonnet review".

The two commits reached a main branch on 2026-11-18 without the merge queue and without a recorded reviewer PASS (Stage 4 audit; scripts/main_provenance.py listed them as UNACCOUNTED). An independent reviewer (Claude Sonnet 5.5, read-only; it wrote neither) reviewed both on their merits on 2026-11-19:

- **broadsword f4123a1a350d0341d095156827d679c0643a9820: PASS.** It changed only `max_agents` 4 -> 6 in broadsword.toml (the line comment unchanged) and added a ledger row in RETRO.md ("max_agents raised 4 -> 6 so the backlog's Python benchmark tasks run beside the Rust ones ... watch: build-lock waits, RAM"). The value is in force again: broadsword.toml reads `max_agents = 6` since ce8c6ee reverted the 10-agent trial.
- **render 41849306f4d65d9301c399ea05c5f70b547c25e2: PASS.** It added `.gitattributes` with only `*.ppm binary`, which stops text diffs and line-ending conversion of the 15 tracked .ppm golden images, the need recorded in LAB_LOG.md ("so the queue's diff of golden images stays small") and RETRO.md ("The queue diffed images as text (510 KB)"). No secrets, no paths, nothing deleted.
- **Reviewer's notes:** both skipped the queue; 4184930's message says so, f4123a1's does not (it has the ledger row). The 6-agent ledger row is still "open". The proper code fix for diffing with the reviewed commit's .gitattributes is still a backlog item (nb-13).

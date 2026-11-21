# Per-node notes

One file per roadmap node that has been started, named by node number (for example `0.1.md`). The agent working a node writes its decisions, test results and what is left there, and updates the node's row in `ROADMAP.md`. Resuming a node means reading its file first.

- `ROADMAP.md` lists every node and its status; `python -I scripts/roadmap_status.py` checks it against `CAPABILITY_ROADMAP.md` and the repo.
- A status other than `not started` must cite evidence (`file:`, `tag:` or `commit:` items) that the script can verify.
- Notes files so far: `0.1.md`, `0.2.md` to `0.7.md` and `0.9.md` (the Tier 0 nodes whose code landed in `projects/foundation`; each summarises and links its `docs/` files), and the plan files for nodes 1.1 to 1.9 and 2.8.

# Per-node notes

One file per roadmap node that has been started, named by node number (for example `0.1.md`). The agent working a node writes its decisions, test results and what is left there, and updates the node's row in `ROADMAP.md`. Resuming a node means reading its file first.

- `ROADMAP.md` lists every node and its status; `python -I scripts/roadmap_status.py` checks it against `CAPABILITY_ROADMAP.md` and the repo.
- A status other than `not started` must cite evidence (`file:`, `tag:` or `commit:` items) that the script can verify.
- No node has a notes file yet.

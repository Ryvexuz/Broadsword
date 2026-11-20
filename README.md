# Broadsword

Broadsword is an AI-driven development ecosystem. Its rules are in `AGENTS.md`; moving it to another PC is in `docs/MOVING.md`.

One command: `build/bin/broadsword.exe check crash-sim`

That is the stage 1 done check 1: it builds and tests the crash-sim project (`projects/crash-sim`, its own Git repo, git-ignored). It needs the installed `build/bin/broadsword.exe` (bootstrap steps in `docs/MOVING.md`). Build and test only through Broadsword, never plain `cargo`.

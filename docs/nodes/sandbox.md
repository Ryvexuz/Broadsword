# Plugin sandbox (Windows): infrastructure plan and done checks

**Status: draft: rewrite when the node starts.** Written 2026-11-20 from Broadsword `6d2ee9e`; per-plugin SIDs added after the first review the same day, before any code and before any experiment. Nothing in this file has been run. Every program, flag, test, fixture and setting named below is created by a brief in section 7 and is not real until that brief merges. Every number is a starting value with its reason; the first brief that measures it replaces it and says so here.

**UNVERIFIED** marks every claim about a Windows API or Windows behaviour that has not been checked on this machine (Windows 11 Pro 10.0.26300). All of them are from memory. Brief X1 (section 7) checks each one, writes the result next to it here (VERIFIED with the evidence file, or WRONG with what is true), and stops the node if a WRONG one breaks the design.

**HARD GATE: no third-party plugin code runs until this node is done and Lazlo approves it.** "Done" means every check in section 1 passes, including check x24, which is Lazlo's own sign-off. Until then:

- Only first-party code runs as a plugin: Rust crate groups linked into the shell, and C-ABI DLLs built from the shell repo's own sources (the node 1.5 test plugins and this node's probe). These may use the plain process boundary of node 1.5 (`plugin_host`) without a sandbox.
- No plugin is downloaded, copied in from elsewhere, or loaded from a folder that the shell repo did not build (Ask first; 1.5 risk R13).
- The gate is enforced in code, not only here (playbook lesson 28): brief X6 makes the loader refuse every third-party DLL while the required setting `plugin_third_party_allowed` in `broadsword.toml` is `false` (check x21), and only Lazlo changes that value. Passing every other check does not open the gate.

**Why this node exists** (1.5 risk R13): node 1.5 runs each C-ABI plugin in its own host process. That protects the core data and the shell from a buggy plugin. It does not protect the machine: the host runs as Lazlo's user, so a malicious plugin can read his files (the root `.env` included), use the network, start programs, and exhaust memory, CPU, handles or disk. This node puts that host process in a Windows sandbox so a plugin can reach only what it is given.

**What the sandbox must guarantee**, each with the check that proves it:

| Guarantee | How | Checks |
|---|---|---|
| No file access outside one granted folder (read or write), including through a junction | AppContainer token; an access rule for the container's SID on the granted folder only | x5, x6, x7 |
| No plugin can reach another plugin's folder, even when both run at once | one AppContainer SID per installed plugin, derived from its install id; each plugin's granted folder has a protected ACL naming only its own SID (section 2) | x2, x26 |
| No network, loopback included | AppContainer with no capabilities | x8 |
| No other channel out: named pipes, other processes | AppContainer object access rules; no handle to the shell except its own two pipe ends | x9, x12 |
| No child process, and no escape from the job | job object with an active-process limit of 1 and no breakaway; child-process mitigation policy as a second layer | x10 |
| No secret in its environment | an environment block built from an allow list, never inherited | x11 |
| Memory, CPU, handle and data-folder disk limits | job object limits; a broker-side handle and folder-size monitor | x13, x14, x15, x16 |
| Core edits only through the broker, and only through `apply_checked` | the sandboxed process holds no core handle; the broker accepts an allow list of messages and sends every edit to `CoreAccess::apply_checked` | x17, x18, x19 |

## 1. Done checks as commands

### How to run and read a check

- Same rules as docs/nodes/1.5.md, section 1, "How to run and read a check": run from the Broadsword root; `broadsword` means `build/bin/broadsword.exe`; add `--task <id>` after the project name inside a task; never call cargo directly; every command writes to a file in `scratch/`; read the file; never pipe a broadsword command into another program; anything not PASS is FAIL, including a check that cannot run.
- Programs run with the project folder (`projects/shell` or its task worktree) as their working folder, so `tests/fixtures/...` and `out/...` are relative to the shell repo.
- `python -I scripts/node_check.py` judges `contains X`, `last line == X` and `exit != 0`, and every rule except `exit != 0` also needs exit code 0. It accepts named nodes since main `f2d03df`, so `python -I scripts/node_check.py sandbox --root . --list` reads this file; at the fix-round commit it printed `26 checks, 0 problems`.
- A check marked (human) needs Lazlo and is never run by an agent or counted as passed until he says so. A check marked (quiet board) is timing: run it only while no build, test or review is running (`broadsword status`).
- **How the escape program reports** (`examples/sandbox_check.rs`, brief X5). Every run prints exactly one summary line starting `sandbox_check:` and exits 0 only when that line is the expected one:
  - `sandbox_check: refused probe=<name> ...` (exit 0): the probe was alive in the sandbox, its allowed control step worked, and the forbidden step failed with an access error.
  - `sandbox_check: contained probe=<name> by=<limit> ...` (exit 0): a limit stopped the probe (the job or the broker's monitor ended it, or the allocation failed), and the shell side is unaffected.
  - `sandbox_check: ESCAPED probe=<name> ...` (exit 1): the forbidden step worked.
  - `sandbox_check: FAIL probe=<name> reason=<why>` (exit 1): the probe did not prove it was alive (no nonce), the sandbox did not start, a DLL or flag is missing, or the probe gave no answer within `--deadline-ms`.
- **A refusal counts only from a live probe.** Each probe first reads a nonce file the program wrote into the granted folder (a random value per run) and sends it back; then it tries the forbidden step and reports the Windows error code. Without the nonce the result is FAIL, never "refused", so a sandbox that blocks everything, or a probe that crashed at load, cannot pass an escape check.
- Every flag of `sandbox_check` is required except `--no-sandbox` and `--expect`; a missing flag exits 2 with usage. `--settings` names a settings file with every sandbox key (section 5); the checks use the test config `tests/fixtures/sandbox_settings_v1.toml`. `--work` names a parent folder; each run creates its own fresh subfolder there (`<work>/<run id>/granted`, `<work>/<run id>/outside`) and deletes nothing.

### Settings and launch spec (pure, no Windows calls)

**x1 Every sandbox setting is required and validated.** Test: a settings file missing any one key is a named error naming that key (never a default); out-of-range values are named errors (memory 0 or over 65536 MB; CPU percent outside 1..100; handles 0 or over 16384; active processes not exactly 1; data-folder MB 0; a path that is absolute, has a drive letter, a `\`, `..`, or a CR/LF).
```check x1
build/bin/broadsword.exe test shell > scratch/sbx-x1.txt 2>&1
PASS: contains test sandbox_settings_required_and_validated ... ok
```

**x2 The launch spec is exactly the planned one.** Pure test of `sandbox::LaunchSpec` built from the fixture settings, before any Win32 call: job limits = memory, CPU hard cap, active processes 1, kill on job close, die on unhandled exception, no breakaway flags, UI limits all set; capability list empty; inherited handles exactly the two pipe ends; environment exactly the allow list (section 5, `SystemRoot`, `TEMP` and `TMP` pointing into the granted folder, nothing else); container name exactly `broadsword.shell.plugin.v1.<install id>` and granted folder exactly `<plugin_data_dir>/<install id>` (section 2); access rules exactly three, all for that plugin's own SID: granted folder read and write (with a protected ACL that names no other AppContainer SID), host exe and plugin DLL read and execute, nothing else. The same test builds specs for two install ids and requires different container names (the SID itself is derived by X3 and compared in x26), different folders and no rule of one naming the other's container; it requires an install id outside `[a-z][a-z0-9_-]{0,31}` to be refused as `bad_install_id`; and it requires that removing one spec's rules (the pure rule-set diff X3 applies) leaves the other spec's rules on the shared host exe and DLL untouched. One extra spec is tested too: `LaunchSpec::with_probe_child` (used only by `sandbox_check` for x10) has exactly four rules, the fourth being read and execute on `probe_child.exe` for the same SID; a structure test in the same run requires that `src/plugin.rs` never calls it.
```check x2
build/bin/broadsword.exe test shell > scratch/sbx-x2.txt 2>&1
PASS: contains test sandbox_launch_spec_exact ... ok
```

**x3 The sandbox refuses a volume without access control.** The granted folder, the host exe and the plugin DLL must be on a volume that keeps ACLs (NTFS or ReFS). On FAT32 or exFAT (a common external-SSD format, and Broadsword is meant to move to one) an access rule does nothing, so the launch is refused with `no_acl_volume`. Pure test with injected volume flags; X3 also calls the real query for the project volume (`GetVolumeInformationW`, flag `FILE_PERSISTENT_ACLS`: UNVERIFIED name and meaning).
```check x3
build/bin/broadsword.exe test shell > scratch/sbx-x3.txt 2>&1
PASS: contains test sandbox_refuses_volume_without_acls ... ok
```

### The sandbox starts and the probe is alive

**x4 The probe runs inside an AppContainer, inside the job, and proves it is alive.** The probe reads the nonce, and reports from its own token that it is an AppContainer token (`TokenIsAppContainer`: UNVERIFIED) with low integrity (UNVERIFIED: AppContainer processes run at low integrity), and from `IsProcessInJob` that it is in a job; the program checks the same from the outside (the job's process list holds the probe's process id).
```check x4
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe alive --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x4.txt 2>&1
PASS: contains sandbox_check: ok probe=alive nonce=match appcontainer=yes integrity=low in_job=yes
```

### Escape tests: each must print a refusal

All run with the sandbox on. Each is a separate command so one escape cannot hide behind another. Threshold reason for `--deadline-ms 5000`: a probe does one small step; 5 s is far above a real step and short enough that a hung probe fails fast.

**x5 Read outside the granted folder.** The program writes `outside/marker.txt` (a random value, not a real secret) next to the granted folder in the same parent; the probe tries to open it for reading by its full path, and by `granted\..\outside\marker.txt`. Both must fail with access denied (`ERROR_ACCESS_DENIED`, 5). Why this file and not a system file: folders that Windows grants to all AppContainers (for example `C:\Windows`; group `ALL APPLICATION PACKAGES`, UNVERIFIED) stay readable, and that is accepted (R3); the rule is no user data outside the granted folder. The out/ folder inherits the project folder's rules, which have no AppContainer entry (UNVERIFIED for this PC; X1 prints the project root's ACL to check it).
```check x5
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe read_outside --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x5.txt 2>&1
PASS: contains sandbox_check: refused probe=read_outside full_path=access_denied dotdot_path=access_denied
```

**x6 Write outside the granted folder.** The probe tries to create `outside/new.txt` and to append to `outside/marker.txt`; both fail with access denied, and the program confirms afterwards that `new.txt` does not exist and the marker's bytes are unchanged.
```check x6
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe write_outside --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x6.txt 2>&1
PASS: contains sandbox_check: refused probe=write_outside create=access_denied append=access_denied marker=unchanged
```

**x7 A junction inside the granted folder does not lead out.** The program (outside the sandbox) creates a directory junction `granted/link` pointing at `outside/` (a junction needs no admin rights: UNVERIFIED for this PC); the probe opens `granted/link/marker.txt`, which must fail with access denied, because the access check is made on the target (UNVERIFIED). This guards the case where the grant rule is checked on a path rather than on the object.
```check x7
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe junction_outside --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x7.txt 2>&1
PASS: contains sandbox_check: refused probe=junction_outside access_denied
```

**x8 Open a socket.** The program opens a TCP listener and a UDP socket on `127.0.0.1` with a port the OS picks, and passes the ports to the probe. The probe tries a TCP connect and a UDP send to them. Both must fail, and the program must see no connection and no datagram within the deadline. Loopback only, so the check never sends a packet off the machine, even in its control run (x22). An AppContainer with no capabilities has no network access and no loopback access unless a loopback exemption is set for it (UNVERIFIED; the exemption tool is `CheckNetIsolation.exe`, also UNVERIFIED); X3 uses a container name of its own, so no existing exemption can apply. Creating the socket itself may succeed; only the connect and the send are judged.
```check x8
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe socket_loopback --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x8.txt 2>&1
PASS: contains sandbox_check: refused probe=socket_loopback tcp_connect=refused udp_send=not_received listener_saw=0
```

**x9 Open a named pipe outside.** The program creates a named pipe with the default security of Lazlo's user; the probe tries to open it by name and must get access denied (UNVERIFIED: AppContainer processes cannot open objects whose rules lack their SID or `ALL APPLICATION PACKAGES`). This stands for the class "talk to another process by name" (pipes, and by the same rule mutexes, sections and events).
```check x9
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe pipe_outside --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x9.txt 2>&1
PASS: contains sandbox_check: refused probe=pipe_outside access_denied
```

**x10 Spawn a child.** The probe calls `CreateProcessW` twice on the program's own small helper `probe_child.exe` (not copied anywhere): once plainly and once with `CREATE_BREAKAWAY_FROM_JOB`. Both must fail. Two layers block it: the job's active-process limit of 1 (`JOB_OBJECT_LIMIT_ACTIVE_PROCESS`, the error is then `ERROR_NOT_ENOUGH_QUOTA`: UNVERIFIED) and the child-process policy set at launch (`PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY` with `PROCESS_CREATION_CHILD_PROCESS_RESTRICTED`: UNVERIFIED). The program also requires the job's process count to have stayed at 1 and that `probe_child.exe` never wrote its marker file. Each layer is tested alone by test `sandbox_spawn_each_layer_blocks` (X3), so the two-layer result does not hide a layer that does nothing. **Why the ACL is not what refuses it:** for this probe only, `sandbox_check` launches with `LaunchSpec::with_probe_child`, which adds a fourth rule, read and execute on `probe_child.exe` in place for the probe's SID (x2 tests that spec). With that rule the file is reachable, so a refusal comes from the job or the child policy; the layer test runs with the same rule. The real shell never uses that spec (x2's structure test).
```check x10
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe spawn_child --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x10.txt 2>&1
PASS: contains sandbox_check: refused probe=spawn_child plain=refused breakaway=refused job_processes_max=1 child_marker=absent
```

**x11 No secret reaches the environment.** The program sets a marker variable `BROADSWORD_SANDBOX_TEST_SECRET` (a random value, not a real secret) in its own environment before launching; the probe lists its whole environment and sends back the names only. The marker must be absent and every name must be on the allow list. Playbook lesson 16: a secret leaks through a child's environment, so the block is built from the allow list where the process is spawned, never inherited.
```check x11
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe env_secret --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x11.txt 2>&1
PASS: contains sandbox_check: refused probe=env_secret marker=absent unlisted_names=0
```

**x12 No handle to the shell or another process.** The probe tries `OpenProcess` on the shell's (here: the program's) process id with `PROCESS_VM_READ`, `PROCESS_VM_WRITE` and `PROCESS_DUP_HANDLE`, and counts its own inherited handles; every open fails with access denied and the only inherited handles are its two pipe ends (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: UNVERIFIED that it limits inheritance to the listed handles).
```check x12
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe open_shell_process --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x12.txt 2>&1
PASS: contains sandbox_check: refused probe=open_shell_process vm_read=access_denied vm_write=access_denied dup_handle=access_denied inherited_handles=2
```

**x26 One plugin cannot reach another plugin's folder.** (Numbered x26 because x13 to x25 were already in use; it belongs with the escape tests.) The program launches two sandboxed probes at once with install ids `check-a` and `check-b`, so two containers, `broadsword.shell.plugin.v1.check-a` and `...check-b`, each with its own granted folder (`<work>/<run id>/granted-a`, `granted-b`) and its own nonce. Liveness first: B reads its nonce and sends it back and stays running; then A reads its own nonce and sends it back. Only then does A try to open B's `nonce.txt` for reading by full path, to create `granted-b/from-a.txt`, and to list `granted-b/`. All three must fail with access denied (5), because B's folder has a protected ACL naming only B's SID (UNVERIFIED: that a protected DACL with no inherited entries leaves no path for another AppContainer SID). After A ends and its rules are removed, the program reads the ACLs of `granted-b`, the host exe and the DLL and requires B's entries to be unchanged, and B must read its nonce once more (`b_after_a=alive`), so ending one plugin never cuts off another. The two SIDs must differ (`sids_differ=yes`). If either probe fails to show its nonce, the result is FAIL, never refused.
```check x26
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe read_sibling --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x26.txt 2>&1
PASS: contains sandbox_check: refused probe=read_sibling a_alive=yes b_alive=yes sids_differ=yes read=access_denied create=access_denied list=access_denied b_rules=intact b_after_a=alive
```

### Limits: each must be contained

Fixture values (section 5): memory 256 MB, CPU 25 percent, 512 handles, data folder 64 MB. Reasons: 256 MB is far above what a panel provider needs and small next to the PC's RAM, so 14 agents' worth of plugins cannot press RAM over `ram_warn_percent`; 25 percent is one quarter of the machine, so a spinning plugin cannot starve the shell's frame; 512 handles is far above a plugin's pipes, files and threads; 64 MB is enough for a plugin's own state and small next to `disk_floor_gb`. The real values are keys in `broadsword.toml` (brief X0), not these.

**x13 Memory.** The probe allocates and touches memory in 16 MB steps up to twice the limit. The job's process memory limit (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`: UNVERIFIED that commit over it fails the allocation rather than killing the process) stops it at or below the limit; either a failed allocation reported by the probe or the job's limit notification ends it. The program records the job's peak (`PeakProcessMemoryUsed`: UNVERIFIED field name).
```check x13
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe memory_limit --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x13.txt 2>&1
PASS: contains sandbox_check: contained probe=memory_limit by=job_memory peak_within_limit=yes
```

**x14 CPU (quiet board).** The probe spins every thread it can start inside its process (8 threads) for 3 seconds. The job's hard CPU cap (`JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` with `JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP`, rate in 1/100 of a percent: UNVERIFIED) holds its CPU time, from the job's accounting, at or below the limit plus 5 percentage points over the window. Reason for 5 points: the cap is enforced per scheduling interval, so a short window can overshoot a little (UNVERIFIED size). The program also ends the probe at `--deadline-ms` as a hung call, which is the second CPU limit (total time).
```check x14
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe cpu_limit --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x14.txt 2>&1
PASS: contains sandbox_check: contained probe=cpu_limit by=cpu_rate within_cap=yes
```

**x15 Handles.** A job object has no handle-count limit (UNVERIFIED), so the broker side polls the probe's handle count (`GetProcessHandleCount`: UNVERIFIED that it works on an AppContainer process from outside with `PROCESS_QUERY_LIMITED_INFORMATION`) every `plugin_handle_poll_ms` and kills the job when it passes the limit. The probe opens event handles in a loop up to four times the limit. Required: the job is killed by the monitor, and the last count it saw is below twice the limit (polling is racy; R6 says why that bound is enough).
```check x15
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe handle_limit --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x15.txt 2>&1
PASS: contains sandbox_check: contained probe=handle_limit by=handle_monitor killed=yes
```

**x16 Data-folder disk.** NTFS has no per-folder quota, so the broker side checks the granted folder's total size every `plugin_handle_poll_ms` and kills the job when it passes `plugin_data_max_mb`; the probe writes 1 MB files in a loop. This keeps a plugin from pushing free space under `disk_floor_gb`. The program requires the kill and a folder size below twice the limit.
```check x16
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe disk_fill --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x16.txt 2>&1
PASS: contains sandbox_check: contained probe=disk_fill by=folder_monitor killed=yes
```

### Broker: core edits only through `apply_checked`

**x17 Every edit from a sandboxed plugin reaches the core only through `apply_checked`.** Test against the 1.5 counting test core (`tests/support/test_core.rs`), whose every method other than reads and `apply_checked` panics when called from the broker: a probe sends valid edits, invalid edits and a mixed batch; valid ones raise the revision by their count, invalid ones are refused with their named error and the core's hash is unchanged.
```check x17
build/bin/broadsword.exe test shell > scratch/sbx-x17.txt 2>&1
PASS: contains test broker_edits_only_via_apply_checked ... ok
```
The same run must print `test broker_source_calls_only_apply_checked ... ok`: a structure test that `src/plugin.rs` calls no `CoreAccess` method except reads and `apply_checked` (the 1.5 rule enforced at the one place every plugin message passes, lesson 9).

**x18 The broker accepts only its allow list.** From a plugin it accepts `Registration`, `DrawList`, `EditRequest` and `Log`, each under its size limit, with edits and logs under a rate limit (`plugin_max_edits_per_second`, `plugin_max_log_bytes_per_second`); anything else (an unknown tag, a message kind only the shell may send, an oversized or truncated message, a flood) ends the plugin as `bad_message` or `rate_limit` and changes nothing in the core. The broker never opens a file, a socket or a process on a plugin's behalf in this node.
```check x18
build/bin/broadsword.exe test shell > scratch/sbx-x18.txt 2>&1
PASS: contains test broker_refuses_unlisted_messages ... ok
```

**x19 The 1.5 containment checks still pass with the sandbox on.** Node 1.5's `plugin_check` (d24) runs every bad plugin and the good plugin through the sandboxed host, so sandboxing does not break registration, painting, edits or crash and hang reports.
```check x19
build/bin/broadsword.exe run shell -- cargo run --offline --release --example plugin_check -- --deadline-ms 500 --sandbox tests/fixtures/sandbox_settings_v1.toml > scratch/sbx-x19.txt 2>&1
PASS: contains plugin_check: ok bad=7 refused_or_contained=7 core_valid=yes bad_edits_applied=0 good=ok sandbox=on
```

### The whole run, the gate and the controls

**x20 Every probe in one run.**
```check x20
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe all --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x20.txt 2>&1
PASS: contains sandbox_check: ok probes=14 alive=14 refused_or_contained=13 escaped=0
```
The 14 probes are the complete list for this node: `alive`, `read_outside`, `write_outside`, `junction_outside`, `socket_loopback`, `pipe_outside`, `spawn_child`, `env_secret`, `open_shell_process`, `read_sibling`, `memory_limit`, `cpu_limit`, `handle_limit`, `disk_fill`. **How the three counts are made:** `alive` counts probes that proved their nonce (all 14; `read_sibling` counts once, and only when both A and B proved theirs); `refused_or_contained` counts the 13 escape and limit probes, every probe except `alive`, whose own outcome is `ok` and is not a refusal; `escaped` counts `ESCAPED` outcomes. So the pass line is 14, 14, 13, 0, and the program exits 1 if `alive` is below `probes` or `refused_or_contained` is below 13. (Run x14's quiet-board rule for this check too.)

**x21 The hard gate is in code.** Tests: with `plugin_third_party_allowed = false` the loader refuses any DLL that the shell repo did not build (`third_party_gate_closed`), sandbox or not; with it `true` the loader still refuses to start one without the sandbox (`third_party_requires_sandbox`); a first-party DLL may run unsandboxed only through the separate `spawn_first_party` path; there is no flag, environment variable or test seam that skips either rule (playbook lesson 11: a seam may only make a guard stricter).
```check x21
build/bin/broadsword.exe test shell > scratch/sbx-x21.txt 2>&1
PASS: contains test third_party_gate_closed ... ok
```
The same run must print `test third_party_requires_sandbox ... ok`.

**x22 Control: without the sandbox every probe escapes.** Proves each probe can see the thing it tests, so a "refused" above is the sandbox and not a broken probe (playbook lesson 10). `--no-sandbox` runs the repo's own probe in the 1.5 plain host; the program refuses `--no-sandbox` for any DLL but its own `escape_probe` (test `no_sandbox_only_for_repo_probe`). `--expect escaped` changes only what the program expects. The controls use the same marker files, loopback ports and helper; nothing leaves the machine.
```check x22
build/bin/broadsword.exe run shell -- cargo run --offline --release --example sandbox_check -- --probe all --no-sandbox --expect escaped --settings tests/fixtures/sandbox_settings_v1.toml --work out/sandbox --deadline-ms 5000 > scratch/sbx-x22.txt 2>&1
PASS: contains sandbox_check: control ok probes=13 escaped=13
```
13, not 14: `alive` is not an escape (the same rule as x20's `refused_or_contained`). In the control `read_sibling` escapes because both plain hosts run as Lazlo's user, which can read B's folder. In the control `memory_limit`, `cpu_limit`, `handle_limit` and `disk_fill` "escape" by passing the limit without being stopped; the probe stops itself at twice the limit (four times for handles) so the control is bounded.

**x23 The whole suite is green.**
```check x23
build/bin/broadsword.exe test shell > scratch/sbx-x23.txt 2>&1
PASS: contains test: ok
```

**x24 Lazlo approves (human).** Lazlo reads this file's evidence (x1 to x23 and x26 output files, X1's API findings) and the plugin source policy (R11), and says in chat that third-party plugins may run. Only after that does anyone set `plugin_third_party_allowed = true`, and that change goes through the merge queue with his message quoted.
```check x24
build/bin/broadsword.exe status > scratch/sbx-x24.txt 2>&1
PASS: Lazlo's approval, quoted from chat with its date, written under this check; never judged by a program
```

**x25 The settings keys exist in Broadsword's real config.** Broadsword's own test suite requires every sandbox key in `broadsword.toml` (brief X0), so the shell never runs with a missing key and no code has a default.
```check x25
build/bin/broadsword.exe test broadsword > scratch/sbx-x25.txt 2>&1
PASS: contains test config_has_plugin_sandbox_keys ... ok
```

### Check to brief map

| Check | Needs, and the brief that creates it |
|---|---|
| x1, x2, x3 | `src/sandbox.rs` (settings, `LaunchSpec`), `tests/sandbox.rs`, `tests/fixtures/sandbox_settings_v1.toml`: X2; x3's real volume query: X3 |
| x4 to x16, x20, x22, x26 | `src/sandbox_win.rs`: X3; `examples/sandbox_check.rs`, `examples/plugins/escape_probe.rs`, `examples/probe_child.rs`: X5 |
| x10's layer test | test `sandbox_spawn_each_layer_blocks` in `tests/sandbox_win.rs`: X3 |
| x15, x16 | the handle and folder monitor in `src/plugin.rs`: X4 |
| x17, x18 | `src/plugin.rs` broker rules, `tests/broker.rs`: X4 |
| x19 | the `--sandbox` flag of `examples/plugin_check.rs`: X5 |
| x21 | the loader gate in `src/plugin.rs`, `tests/plugin_gate.rs`: X6 |
| x24 | Lazlo |
| x25 | `broadsword.toml` keys and the config test in `crates/broadsword`: X0 |
| x23 | every brief |

## 2. Home

**In the shell repo, `projects/shell`, next to node 1.5's plugin code** (docs/nodes/1.5.md, section 2, layer 4). The sandbox wraps the `plugin_host` process 1.5 already plans; it is not a new process kind and not a new repo. If node 0.7 takes over the plugin mechanism (1.5 R15), this code moves with it.

**Design choice: AppContainer plus a job object.** The two Windows options the brief names, compared:

| | AppContainer token | Restricted token (deny-only and restricting SIDs, low integrity) |
|---|---|---|
| Blocks reading user files | yes: access needs the container SID or `ALL APPLICATION PACKAGES` in the object's rules (UNVERIFIED) | only with restricting SIDs; low integrity alone blocks writes, not reads (UNVERIFIED) |
| Blocks the network | yes, with no capabilities (UNVERIFIED) | no: a restricted token has no network rule; it would need a firewall rule, which is machine configuration |
| Grants one folder | one access rule for the container SID | rules for a restricting SID, the same amount of work |
| Machine configuration | possibly an AppContainer profile (R8) | none |

So: **AppContainer, with no capabilities, inside a job object.** A restricted token is the fallback only if X1 finds AppContainer unusable, and then the network guarantee (x8) cannot pass without a firewall rule; that would be Lazlo's decision, not a silent downgrade. Less Privileged AppContainer (LPAC, which drops even `ALL APPLICATION PACKAGES` access) is stricter but may stop the host loading system DLLs (UNVERIFIED); X1 tries it and the plan records the result, but v1 does not need it.

**Process shape:**

1. The shell (broker, Lazlo's normal token) creates a job object with the limits of x2 and **one container SID per installed plugin**, derived from the name `broadsword.shell.plugin.v1.<install id>` (`DeriveAppContainerSidFromAppContainerName`, or `CreateAppContainerProfile` if a profile is needed: UNVERIFIED which, R8). The install id is the plugin's entry name in the plugin list (section 4: the list Lazlo edits, one entry per plugin with its pinned SHA-256; first-party plugins get an entry too). It must match `[a-z][a-z0-9_-]{0,31}` and be unique in the list, so the name is at most 59 characters (UNVERIFIED: an AppContainer name may be up to 64 characters). Why the install id and not the hash: a plugin update changes its pinned hash but keeps its install id, so its SID and data folder survive the update; two plugins never share an id, so they never share a SID. Two running copies of the same plugin share its SID and its folder, which is that plugin's own data.
2. It sets the access rules for that SID only: read and write on the plugin's own granted folder `<plugin_data_dir>/<install id>`; read and execute on `plugin_host.exe` and on that plugin's DLL. Nothing else. **The granted folder's ACL is protected** (inheritance from `plugin_data_dir` turned off, `PROTECTED_DACL_SECURITY_INFORMATION`: UNVERIFIED) and holds exactly Lazlo's user, `SYSTEM` and this plugin's SID: no other plugin's SID and no `ALL APPLICATION PACKAGES` (x2, x26). `plugin_data_dir` itself carries no AppContainer entry, so a plugin cannot list its siblings. The shared `plugin_host.exe` holds one read-and-execute entry per running plugin's SID; adding or removing one plugin's entries touches only entries for that SID (`SetEntriesInAclW` with the SID as trustee: UNVERIFIED that `REVOKE_ACCESS` removes only that trustee's entries), so one plugin ending never cuts off another (x2, x26). The rules on the exe and DLL are added each launch, because a rebuild replaces those files (and with them their rules).
3. It starts `plugin_host.exe` suspended with `CreateProcessW` and an extended startup info: security capabilities (the SID, no capabilities), the handle list (two pipe ends), the child-process policy, and an environment block from the allow list (UNVERIFIED: attribute names in section 4). It assigns the process to the job, then resumes it, so the plugin's code never runs outside the job.
4. The host loads the DLL (1.5, P2) and talks only through its two pipes. The shell side is the **broker**: it decodes every message with the 1.5 panel-protocol decoder and its limits, applies the allow list (x18), sends edits only to `apply_checked` (x17), and runs the handle and folder monitor (x15, x16).
5. When the plugin ends, crashes, hangs or breaks a rule, the broker closes the job (kill on close ends every process in it) and reports the plugin as 1.5 does, with the new reasons `memory_limit`, `handle_limit`, `disk_limit`, `rate_limit`.

**Planned files** (claims for section 7):

```
projects/shell/
  src/sandbox.rs            settings, LaunchSpec, validation (no unsafe, portable)
  src/sandbox_win.rs        cfg(windows), unsafe: SID, access rules, job, launch, volume query
  src/plugin.rs             broker rules, monitors, third-party gate (1.5 file, edited here)
  src/bin/plugin_host.rs    reports its token facts for probe `alive` (1.5 file, edited here)
  tests/sandbox.rs  tests/sandbox_win.rs  tests/broker.rs  tests/plugin_gate.rs
  tests/fixtures/sandbox_settings_v1.toml
  tests/structure.rs        allow `unsafe` in src/sandbox_win.rs too (1.5 file, edited here)
  examples/sandbox_check.rs  examples/probe_child.rs  examples/plugins/escape_probe.rs
  examples/plugin_check.rs  adds --sandbox (1.5 file, edited here)
  docs/SANDBOX.md           what a plugin may and may not do, for plugin authors
Broadsword repo:
  broadsword.toml           the plugin sandbox keys (X0)
  crates/broadsword/...     config test config_has_plugin_sandbox_keys (X0)
```

ADR 0001 (option a): no new Cargo dependency; Win32 functions are hand-declared, as 1.5 does for `LoadLibraryW`. Nothing here fires a new ADR 0001 trigger.

## 3. Prerequisites

| Node or work | Status today | What this node needs from it |
|---|---|---|
| 1.5 briefs P1, P2, P3 (ABI, `plugin_host`, `plugin_check`) | not started (plan in branch `t-20261120-015656`, not on main) | the host process, its pipes, the protocol decoder and `plugin_check`, all merged. This node edits P2's files, so it starts only after P3 merges. |
| 1.5 briefs S4, S5 (`CoreAccess::apply_checked`, panel protocol) | not started | the checked operation and the message allow list are built on them |
| 1.5 brief S1 (`tests/structure.rs`) | not started | the `unsafe` allow list that X3 extends |
| 0.7 Plugin/module system | not started | Overlap (1.5 R15). If 0.7 takes the plugin mechanism, this node goes with it; decide before X2. |
| Lazlo's decisions in section 4 | open | X1 needs his yes to create containers and change ACLs on this PC; X0 needs his values for the limits |

**The node cannot start until node 1.5's plugin briefs have merged.** Writing this plan now is the gate's paper half; nothing in section 7 is boarded before then.

## 4. External needs

| Need | Detail | Decision for Lazlo? |
|---|---|---|
| Rust crates | None. `kernel32`, `advapi32`, `userenv` functions are hand-declared (UNVERIFIED: AppContainer functions live in `userenv.dll`, the security functions in `advapi32.dll`). | No. |
| Windows APIs (all UNVERIFIED) | `DeriveAppContainerSidFromAppContainerName`, `CreateAppContainerProfile`, `DeleteAppContainerProfile`; `SetEntriesInAclW`, `SetNamedSecurityInfoW`; `CreateJobObjectW`, `SetInformationJobObject` (`JobObjectExtendedLimitInformation`, `JobObjectCpuRateControlInformation`, `JobObjectBasicUIRestrictions`), `AssignProcessToJobObject`, `QueryInformationJobObject`; `InitializeProcThreadAttributeList`, `UpdateProcThreadAttribute` with `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`, `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY`; `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT` and `CREATE_SUSPENDED`; `GetProcessHandleCount`; `GetTokenInformation` (`TokenIsAppContainer`, `TokenIntegrityLevel`); `IsProcessInJob`; `GetVolumeInformationW`. | No (OS interfaces), but every one is checked by X1. |
| **An AppContainer profile** | If a derived SID without a profile is not enough, `CreateAppContainerProfile` writes a profile under the user's registry and `%LOCALAPPDATA%\Packages` (UNVERIFIED). That is persistent machine configuration. One profile per installed plugin, named `broadsword.shell.plugin.v1.<install id>` (section 2), plus the two check profiles `...check-a` and `...check-b` (x26), each created once; no agent deletes one. | **Yes**, before X1 creates one. |
| **Access rules on files** | Each launch adds rules for that plugin's own SID on its granted folder (inside the project, with a protected ACL: inheritance off) and on `plugin_host.exe` and the plugin DLL in the shared build folder (read and execute only). That changes ACLs on shared build files and turns off inheritance on each plugin's data folder. | **Yes**, Lazlo should know and agree before X3. |
| **Running a sandbox experiment on this PC** | X1 is a spike that starts real AppContainer processes. This plan ran none. | **Yes**, X1 is boarded only with his OK. |
| The volume | NTFS or ReFS for the project folder (x3). An exFAT external SSD makes the sandbox refuse to start. | Yes, when the project moves. |
| Admin rights | None planned (UNVERIFIED that none of the APIs above need them). If one does, the node stops and asks. | Only if needed. |
| Network | None. Every socket check uses loopback only; nothing is downloaded. | No. |
| Desktop session | Not needed: the probes open no window. | No. |
| Licences | None. Ideas from Chromium's Windows sandbox design are read, never copied. | No. |
| **Where third-party plugins come from** | A policy: from which folder, who puts them there, whether a hash is pinned, how one is removed. This plan proposes: only from a folder Lazlo names in `broadsword.toml`, never fetched by an agent, each DLL pinned by its SHA-256 in a list Lazlo edits, under a unique install id that names its container and its data folder (section 2). | **Yes**, part of x24. |

## 5. Test harness

**How the harness runs the escape tests.**

- **Through Broadsword only.** Pure tests run in `broadsword test shell`; the sandbox runs through `broadsword run shell -- cargo run --offline --release --example sandbox_check -- ...`. `cargo test` builds the probe DLL and `probe_child.exe` as examples (the same assumption as 1.5 d22, settled by 1.5's P2 spike; if false, X5 adds a build step).
- **One probe DLL, many probes.** `examples/plugins/escape_probe.rs` is a `cdylib` that declares the ABI from the header itself (no `use shell`, like the 1.5 test plugins). After registration the program sends it one config message: the probe name, the nonce path, the target paths, the loopback ports. The probe answers with one result message: the nonce it read, then per step a Windows error code or "ok". Only first-party code: it is the only DLL `--no-sandbox` accepts.
- **The program judges, the probe reports.** The probe never decides "refused"; it sends raw error codes, and `sandbox_check` maps them (5 to `access_denied`, and so on) and also checks the outside effects itself: the marker bytes, the listener, the helper's marker file, the job's process count and accounting, the folder size.
- **Fresh folders, nothing deleted.** Each run creates `<--work>/<run id>/` with `granted/` (holding the nonce file) and `outside/` (holding `marker.txt`), sets the access rule on `granted/` only, and removes the rule at the end (the rule, not the files). Files stay for evidence; `out/` is ignored by Git and cleared by Lazlo's rules for it, never by the program.
- **Every outcome is printed.** One `probe <name>: <outcome> <details>` line per probe, then the summary line. A probe that cannot run is FAIL, never skipped.
- **Every test is shown able to fail** (playbook lesson 10). The x22 controls do that for every probe at once; each brief also names one mutation (section 7).

| Check | Kind of test | Compares against |
|---|---|---|
| x1, x2, x3 | pure unit tests of settings and the launch spec | the key list and limits in this section; the planned spec in section 2 |
| x4 | real sandbox, self-report plus outside view | token facts; the job's process list |
| x5 to x12, x26 | real sandbox, one forbidden step per probe, with a live-probe nonce (x26: two probes at once, both live) | Windows error codes; outside effects the program measures |
| x13 to x16 | real sandbox, resource exhaustion | the limits in the settings file; job accounting; the monitor's kill |
| x17, x18 | broker tests over the counting test core | revision and hash; the allow list |
| x19 | 1.5's fault-injection program with the sandbox on | 1.5 d24's table |
| x20 | all probes in one run | 14 alive; 13 of 13 refused or contained |
| x21 | loader gate tests | refusal of third-party DLLs |
| x22 | negative control without the sandbox | 13 of 13 escaped |
| x25 | Broadsword config test | the key list below |

**Settings keys** (the complete list; X0 adds each to `broadsword.toml` with Lazlo's values, X2 puts each in the test fixture with the values of section 1; every one is required and has no default in code):

| Key | Fixture value | Meaning |
|---|---|---|
| `plugin_third_party_allowed` | `false` | the hard gate (x21); Lazlo alone sets it `true` |
| `plugin_memory_mb` | 256 | job process memory limit |
| `plugin_cpu_percent` | 25 | job hard CPU cap |
| `plugin_max_handles` | 512 | handle monitor kill level |
| `plugin_max_processes` | 1 | job active-process limit; any other value is refused (x1) |
| `plugin_handle_poll_ms` | 50 | how often the broker checks handles and folder size |
| `plugin_data_max_mb` | 64 | granted-folder size kill level |
| `plugin_data_dir` | `out/plugin-data` | parent of each plugin's granted folder `<plugin_data_dir>/<install id>`, relative to `root`; carries no AppContainer entry itself |
| `plugin_max_edits_per_second` | 1000 | broker edit rate limit |
| `plugin_max_log_bytes_per_second` | 65536 | broker log rate limit |
| `plugin_env_allow` | `SystemRoot` | names copied into the environment block; `TEMP` and `TMP` are always set to the granted folder and are not listed |

**Fixtures:** `tests/fixtures/sandbox_settings_v1.toml` (X2), exactly the keys above. The nonce, marker, junction and named pipe are made per run by the program, not committed.

## 6. Reference data

Listing only: nothing is downloaded. Every fact below is from memory and is UNVERIFIED until X1 reads the page and tries it.

| What | Source | Licence |
|---|---|---|
| AppContainer isolation: objects, files, network, capabilities | https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation | Microsoft Learn text; API facts only |
| Launching a process in an AppContainer (`SECURITY_CAPABILITIES`) | https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer | same |
| `DeriveAppContainerSidFromAppContainerName`, `CreateAppContainerProfile` | https://learn.microsoft.com/en-us/windows/win32/api/userenv/ | same |
| Job objects and their limits | https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects | same |
| `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` | https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_cpu_rate_control_information | same |
| `UpdateProcThreadAttribute` (handle list, child-process policy, security capabilities) | https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute | same |
| AppContainer loopback isolation and its exemption | https://learn.microsoft.com/en-us/previous-versions/windows/apps/hh780593(v=win.10) | same |
| Restricted tokens (the fallback) | https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens | same |
| Design reference: Chromium's Windows sandbox (broker and target, job, token, AppContainer) | https://chromium.googlesource.com/chromium/src/+/main/docs/design/sandbox.md | BSD-style; design ideas only, no code read for copying |
| Sample sets, published escape suites | **None.** The probes are this repo's own, and the controls (x22) show each one can detect an escape. | not applicable |

## 7. First briefs

T4 shape: one change each, claims named file by file, about 300 changed lines or fewer, a done check that includes one mutation. All in `projects/shell` except X0. None is boarded before node 1.5's P3 has merged and Lazlo has answered section 4.

| Id | Title | Claims | Depends on |
|---|---|---|---|
| X0 | Sandbox settings keys in Broadsword's config | `broadsword.toml`, the config test file in `crates/broadsword` that X0 names after reading it | Lazlo's values |
| X1 | Spike: check every UNVERIFIED claim on this PC | `docs/nodes/sandbox.md` (the UNVERIFIED marks only), `examples/sandbox_spike.rs` (kept as evidence) | Lazlo's OK (section 4) |
| X2 | Settings and launch spec, pure | `src/sandbox.rs`, `tests/sandbox.rs`, `tests/fixtures/sandbox_settings_v1.toml` | X1 |
| X3 | Windows launcher: SID, access rules, job, launch | `src/sandbox_win.rs`, `tests/sandbox_win.rs`, `tests/structure.rs`, `src/bin/plugin_host.rs` | X2 |
| X4 | Broker rules and monitors | `src/plugin.rs` (broker part), `tests/broker.rs` | X3 |
| X5 | Escape probe, helper and the check program | `examples/sandbox_check.rs`, `examples/probe_child.rs`, `examples/plugins/escape_probe.rs`, `examples/plugin_check.rs`, `Cargo.toml` (the new examples), `docs/SANDBOX.md` | X3, X4 |
| X6 | The third-party gate in the loader | `src/plugin.rs` (loader part), `tests/plugin_gate.rs` | X4 |

**Order:** X0 and X1 together (X1 is the only one allowed to change this file's marks). Then X2, X3. Then X4. Then X5 and X6 one after the other (both claim `src/plugin.rs`). Critical path: six rounds.

**What each brief must contain:**

- **X0.** The keys of section 5 in `broadsword.toml`, each with a comment giving its reason; a config test `config_has_plugin_sandbox_keys` that fails naming the first missing key. Mutation: remove one key, show the test failing, restore.
- **X1.** A spike program that, with Lazlo's OK, tries each UNVERIFIED claim once and prints what it saw; the evidence file goes to `evidence/`; this file's marks become VERIFIED (with the file) or WRONG (with the truth). It answers in particular: is a derived SID without a profile enough (R8); does a job memory limit fail the allocation or kill; is loopback blocked with no capabilities; does the handle list limit inheritance; does the child-process policy block `CreateProcessW`; does LPAC still load system DLLs; does the project root's ACL grant anything to AppContainers; may an AppContainer name be 59 characters; does a protected DACL on one plugin's folder keep a second AppContainer SID out; does revoking one SID's entries on the shared host exe leave another SID's entries. If any answer breaks section 2's design, X1 stops and reports; no other brief starts.
- **X2.** `SandboxSettings::load(text) -> Result<_, SettingsError>` with every key required and validated (x1); `LaunchSpec::from(&settings, install_id, host_exe, dll)` as plain data, deriving the container name and granted folder from the install id and refusing a bad id, plus `LaunchSpec::with_probe_child` and the pure per-SID rule-set diff (x2); the volume rule as a pure function of volume flags (x3). Mutation: give `plugin_cpu_percent` a default when missing, show x1's test failing; and derive every container name from the fixed prefix without the install id, show x2's two-spec test failing.
- **X3.** Hand-declared functions; `launch(&LaunchSpec) -> Result<SandboxedChild, SandboxError>` doing steps 1 to 3 of section 2 in that order, suspended until assigned to the job; the protected ACL on the granted folder; removing only its own SID's access rules on drop; `sandbox_spawn_each_layer_blocks` (the job limit alone, the child policy alone). `plugin_host` reports its token facts on request. Every `unsafe` block states its invariant. Mutation: skip the job assignment, show probe `spawn_child` escaping in the test; and leave inheritance on for the granted folder with an entry for both check SIDs on the parent, show x26 printing `ESCAPED`.
- **X4.** In the broker: the message allow list, size and rate limits, every edit to `apply_checked`, the handle and folder monitor, the new end reasons; tests `broker_edits_only_via_apply_checked`, `broker_source_calls_only_apply_checked`, `broker_refuses_unlisted_messages`. Mutation: let one `EditRequest` reach the core through a non-checked method, show x17 failing.
- **X5.** The probe DLL and its 14 probes (`read_sibling` launches two at once); `probe_child.exe` (writes a marker, exits 0); `sandbox_check` with the flags and lines of section 1, the nonce rule, the controls and `no_sandbox_only_for_repo_probe`; `--sandbox <settings>` in `plugin_check`; `docs/SANDBOX.md` for plugin authors. Done check: x4 to x16, x19, x20, x22, x26 as written. Mutation: give the access rule to the parent of `granted/` instead, show x5 printing `ESCAPED`.
- **X6.** The loader gate (x21): `spawn_first_party` for DLLs the shell repo built, `spawn_sandboxed` for every other, refusal while `plugin_third_party_allowed` is `false`. Mutation: make the gate read the setting with a default of `true` when missing, show the gate test failing.

## 8. Risks and size

- **R1 Every Windows fact here is UNVERIFIED.** The design rests on about twenty-five API behaviours recalled from memory. X1 checks them before any other brief; one WRONG answer on AppContainer network or file isolation would change section 2, and that goes back to Lazlo.
- **R2 Volumes without ACLs.** On FAT32 or exFAT no access rule exists, so file isolation is void. x3 makes the sandbox refuse to start there instead of running open. Moving Broadsword to an external SSD then needs that drive to be NTFS for plugins to run.
- **R3 What all AppContainers can read stays readable.** System folders granted to `ALL APPLICATION PACKAGES` (UNVERIFIED which) remain readable to a plugin. They hold no user data, so this is accepted; LPAC would close it if X1 shows it works.
- **R4 Loopback exemptions.** A loopback exemption set for this container's name would reopen x8. The name is Broadsword's own; x8 runs on every check, so an exemption added later shows as a failure.
- **R5 A sandbox is not perfect.** A Windows kernel bug can break any user-mode sandbox. The source policy (section 4, x24) is the second layer: third-party plugins come only from where Lazlo says, pinned by hash.
- **R6 The handle and folder limits are polled.** Between polls a plugin can open more handles or write more data. At 50 ms polls, a fast loop could pass the handle limit by a few thousand before the kill (UNVERIFIED rate); the job memory limit caps how much kernel memory that can cost (UNVERIFIED that handle-backed kernel memory is charged to the job), and the kill ends all of it. x15 and x16 require the overshoot to stay under twice the limit; if X5 measures more, the poll interval shrinks.
- **R7 CPU cap timing is noisy.** x14 is quiet-board only; if it flaps there, find the noise first, never loosen the 5-point margin silently.
- **R8 AppContainer profiles are machine configuration.** If a profile is needed, it persists in the user's registry and AppData, needs Lazlo's OK (section 4), and is never deleted by an agent.
- **R9 ACLs on shared build files.** Each running plugin's container gets read and execute on two files in `build/`: the shared host exe (one entry per running plugin's SID) and its own DLL. That is narrow, but it is a change to shared state; X3 removes only its own SID's rules on drop and reapplies them each launch, and a crash can leave a harmless read rule behind for that SID.
- **R10 Nested jobs.** If `broadsword run` itself puts its children in a job object, the shell's job is nested inside it (supported since Windows 8: UNVERIFIED). The breakaway and kill-on-close rules must still hold; x10 and x13 run under `broadsword run` exactly as the real shell would, so a nesting problem shows there.
- **R11 "First-party" must be defined precisely.** The gate (x21) depends on telling a DLL the shell repo built from any other. X6 defines it as "built from this repo's sources into the shared build folder by this checkout", and its test covers a copied, renamed or modified DLL. If that cannot be decided reliably, everything is treated as third-party and the 1.5 test plugins run sandboxed too.
- **R12 Antivirus.** Defender may scan or block the probe's behaviour (handle and memory floods, a junction). If it does, the result is FAIL with its reason, never a pass; Lazlo decides on any exclusion.
- **R13 Overlap with 0.7** (as 1.5 R15): decide before X2 so the work is not done twice.
- **R14 One SID per plugin, or plugins can read each other.** With one fixed container SID for every plugin, every running plugin would hold the rule on every granted folder, so plugin A could read and write plugin B's data (first review of this plan, 2026-11-20). The design gives each install id its own SID and a protected ACL on its folder (section 2); x2 checks the specs and x26 runs the escape. Open until X1, all UNVERIFIED: the AppContainer name length limit; that a protected DACL leaves no inherited path; that revoking one trustee leaves the others' entries. R8's profile count also grows with the number of plugins. Every access check in the sandbox is made by SID, so a plugin that could choose its own install id could take another's SID: the id comes only from Lazlo's list (section 4), never from the plugin or its DLL.

**Size:** 7 briefs (X0 to X6), about 1,600 changed lines (X3 and X5 are the largest, near 300 each), plus about 3 fix rounds (the first review of X3's `unsafe` launch code and X4's broker is the most likely to fail), so about 10 merged tasks, plus X1's spike evidence and one human check (x24). On the harness's own measure (about 20 merged tasks per delivered stage) that is about half a stage, after node 1.5's plugin briefs.

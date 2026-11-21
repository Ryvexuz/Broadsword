# Tracked copy of scratch/tick.ps1 (task t-20261121-201016); scratch/ copy stays in use until the references move.
﻿Start-Sleep -Seconds 600; Set-Location (Split-Path $PSScriptRoot -Parent); python -I scratch\slot_check.py (Get-Location).Path
# DRIVER-DOWN guard (audit 2026-11-21 19:03Z fix 2): every project with an open queue entry gets a driver
$open = & build/bin/broadsword.exe queue list 2>&1 | Where-Object { $_ -match "`t(queued|approved|running)`t" } | ForEach-Object { ($_ -split "`t")[3] } | Sort-Object -Unique
# NO-MERGE watchdog (Lazlo 19:22Z throughput ask via coordinator): nothing merged for 30+ min while entries are open
$last = [DateTimeOffset]::Parse((git log -1 --format=%cI main)); $mins = [int]((Get-Date) - $last.LocalDateTime).TotalMinutes
if ($open -and $mins -ge 30) { $l = "$((Get-Date).ToUniversalTime().ToString('s'))Z NO-MERGE: $mins min since the last main commit with open entries in: $($open -join ', ')"; $l; Add-Content state\pc-progress.md $l }
foreach ($p in $open) { $r = & powershell -NoProfile -File scripts/start-driver.ps1 $p; if ($r -match '^started') { "DRIVER-DOWN: $p had open entries and no driver; $r" } }

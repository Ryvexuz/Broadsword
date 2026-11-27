# Tracked copy of scratch/qadd.ps1 (task t-20261121-201016); scratch/ copy stays in use until the references move.
# queue add + start the project's driver in one step (the driver exits when idle; an add without a driver strands the entry).
# usage: powershell -NoProfile -File scripts/qadd.ps1 <task> <low|high> <coder> [brief] [-Adjudicated <file>]
# LOOP BREAKER (Lazlo 2026-11-21 06:39Z "can we fix them all?"): a task with 3 or more failed queue rounds is refused
# unless -Adjudicated names an existing file that records each finding's REAL/FALSE/KNOWN verdict and the split,
# design change or escalation decided. No 4th plain round.
param([Parameter(Mandatory=$true)][string]$Task, [Parameter(Mandatory=$true)][string]$Risk, [Parameter(Mandatory=$true)][string]$Coder, [string]$Brief, [string]$Adjudicated)
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $Brief) { $Brief = "state/backlog/$Task.md" }
$failed = @(& build/bin/broadsword.exe queue list 2>&1 | Where-Object { $_ -match '^q-' } | ForEach-Object { ,($_ -split "`t") } |
    Where-Object { $_[2] -eq $Task -and $_[1] -in 'rejected','check_failed','perf_failed','conflict','merged_check_failed' }).Count
if ($failed -ge 3) {
    if (-not $Adjudicated -or -not (Test-Path $Adjudicated)) {
        "qadd: REFUSED. $Task has $failed failed queue rounds (loop breaker: 3). Adjudicate every finding (REAL/FALSE/KNOWN with a quote), then split, change the design or escalate, and pass -Adjudicated <that file>."
        exit 3
    }
    "qadd: $Task has $failed failed rounds; adjudication on file: $Adjudicated"
}
$out = & build/bin/broadsword.exe queue add $Task --risk $Risk --coder $Coder --brief $Brief 2>&1
$out | Select-Object -Last 1
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$proj = (& build/bin/broadsword.exe task show $Task 2>&1 | Select-String '^project: ').Line -replace '^project: ', ''
if (-not $proj) { "qadd: could not read the project of $Task; start its driver by hand"; exit 1 }
& powershell -NoProfile -File scripts/start-driver.ps1 $proj.Trim()

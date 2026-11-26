# Tracked copy of scratch/queue-driver-project.ps1 (task t-20261121-201016); scratch/ copy stays in use until the references move.
# Per-project queue driver (2026-11-20, for queue_per_project = "on"). Start ONE per project:
#   powershell -NoProfile -File scripts/queue-driver-project.ps1 <project>
# It drains only that project's entries (`queue run --all --project <p>`), exits when the project has no open entry,
# exits 4 on a busy lock (another driver of the same project), and stops on exit 6 (main check failed; a human decides).
param([Parameter(Mandatory=$true)][string]$Project)
# TRAIN PRIORITY (orch 19, 2026-11-24 05:09Z): the driver and every child (queue run, its checks) run AboveNormal. Revert: delete this line.
try { (Get-Process -Id $PID).PriorityClass = 'AboveNormal' } catch {}
$ErrorActionPreference = 'Continue'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:BROADSWORD_REVIEWER_CLI = 'C:\Users\MrComputer\.local\bin\claude.exe'
# orch 13 2026-11-23: a driver started without Git's bin on PATH failed every train ("bash is needed"); set it where the driver is spawned (playbook lesson 22)
if (-not (Get-Command bash -ErrorAction SilentlyContinue) -and (Test-Path 'C:\Program Files\Git\bin\bash.exe')) { $env:Path = 'C:\Program Files\Git\bin;' + $env:Path }
$log = "scratch/queue-driver-$Project.log"
while ($true) {
    $list = & build/bin/broadsword.exe queue list 2>&1
    $open = $list | Select-String "`t(queued|approved|running)`t[^`t]+`t$([regex]::Escape($Project))`t"
    if (-not $open) {
        # idle-exit race fix (audit 2026-11-21 19:03Z): an add that saw this driver alive must not be stranded; re-check after a grace
        Start-Sleep -Seconds 20
        $open = & build/bin/broadsword.exe queue list 2>&1 | Select-String "`t(queued|approved|running)`t[^`t]+`t$([regex]::Escape($Project))`t"
        if (-not $open) { "$((Get-Date).ToUniversalTime().ToString('s'))Z idle, driver exits" | Add-Content $log; break }
    }
    "$((Get-Date).ToUniversalTime().ToString('s'))Z run --all --project $Project (open: $(@($open).Count))" | Add-Content $log
    & build/bin/broadsword.exe queue run --all --project $Project 2>&1 | Add-Content $log
    $rc = $LASTEXITCODE
    if ($rc -eq 6) { "$((Get-Date).ToUniversalTime().ToString('s'))Z STOP: main check failed (exit 6); a human decides" | Add-Content $log; break }
    if ($rc -eq 4) { "$((Get-Date).ToUniversalTime().ToString('s'))Z another driver of $Project holds its lock (exit 4); this driver exits" | Add-Content $log; break }
    if ($rc -eq 2) { "$((Get-Date).ToUniversalTime().ToString('s'))Z STOP: usage refused (exit 2)" | Add-Content $log; break }
}

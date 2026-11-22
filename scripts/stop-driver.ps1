# stop-driver.ps1 <project> [--when-idle-wait] [--dry-run]: stops the per-project queue driver of ONE project and
# nothing else. An allow rule points at this script, so it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a
# reason on stderr, before stopping anything) and stops processes only when every check passes. The root is the parent
# of this script's folder; there is no override. It never pushes, forces, resets, deletes files or history, reads .env
# or edits .claude/.
#
# What it stops (found by command line, Win32_Process), and only that:
#  - a powershell.exe whose WHOLE command line is `<powershell> [-NoProfile|-NonInteractive|-WindowStyle Hidden|
#    -ExecutionPolicy Bypass ...] -File scratch/queue-driver-project.ps1 <project>` (the form start-driver.ps1 uses);
#  - a broadsword.exe whose PARENT is such a driver and whose whole command line is
#    `<broadsword.exe> queue run --all --project <project>`.
# A command line that merely contains these words (a -Command, a grep, another project) does not match. Each process is
# re-read by pid right before it is stopped, and is stopped only if its creation time is unchanged (no pid reuse).
#
# Checks, all of which must pass:
#  1. <project> is a plain project name; the only other arguments are --when-idle-wait and --dry-run, each once.
#  2. The queue folder state/queue exists and no entry of <project> (state/queue/*.entry, `project = <project>`) has
#     `status = running` (a merge is in progress). With --when-idle-wait it re-checks every 5 s for up to 30 min and
#     refuses if an entry is still running then. An entry file that cannot be read refuses.
# --dry-run runs every check, then only prints what it would stop.
# Known limit: a driver started with the same relative command line from ANOTHER copy of the project folder also
# matches (Win32_Process does not report a process's working folder).

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("stop-driver: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("stop-driver: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$waitMinutes = 30
$pollSeconds = 5

# 1. Arguments.
$all = @($args | ForEach-Object { [string]$_ })
if ($all.Count -lt 1) { Refuse 'usage: stop-driver.ps1 <project> [--when-idle-wait] [--dry-run]' }
$project = $all[0]
if ($project -cnotmatch '^[a-z][a-z0-9_-]{0,63}$') { Refuse "project '$project' is not a plain project name" }
$wait = $false; $dry = $false
for ($i = 1; $i -lt $all.Count; $i++) {
    $a = $all[$i]
    if ($a -ceq '--when-idle-wait') { if ($wait) { Refuse '--when-idle-wait given twice' }; $wait = $true }
    elseif ($a -ceq '--dry-run') { if ($dry) { Refuse '--dry-run given twice' }; $dry = $true }
    else { Refuse "argument '$a' is not allowed (only <project>, --when-idle-wait, --dry-run)" }
}

# 2. No running queue entry of the project.
$qdir = Join-Path $root 'state/queue'
if (-not (Test-Path -LiteralPath $qdir -PathType Container)) { Refuse "queue folder '$qdir' does not exist, so a running merge cannot be ruled out" }
function RunningEntries {
    $ids = @()
    foreach ($f in (Get-ChildItem -LiteralPath $qdir -Filter '*.entry' -File)) {
        $kv = @{}
        foreach ($line in [IO.File]::ReadAllLines($f.FullName)) {
            $ix = $line.IndexOf(' = ')
            if ($ix -gt 0) {
                $key = $line.Substring(0, $ix)
                if (-not $kv.ContainsKey($key)) { $kv[$key] = $line.Substring($ix + 3) }
            }
        }
        if ($kv['project'] -ceq $project -and $kv['status'] -ceq 'running') { $ids += $f.BaseName }
    }
    return , $ids
}
$running = RunningEntries
if ($running.Count -gt 0) {
    if (-not $wait) { Refuse "queue entry $($running -join ', ') of '$project' is running (a merge is in progress); use --when-idle-wait to wait" }
    $deadline = [DateTime]::UtcNow.AddMinutes($waitMinutes)
    Write-Output "stop-driver: waiting up to $waitMinutes min for $($running -join ', ') to finish"
    while ($running.Count -gt 0) {
        if ([DateTime]::UtcNow -ge $deadline) { Refuse "queue entry $($running -join ', ') of '$project' still running after $waitMinutes min" }
        Start-Sleep -Seconds $pollSeconds
        $running = RunningEntries
    }
}

# 3. Find the driver and its queue-run child by exact command line.
$pq = [regex]::Escape($project)
# The program path and flags match case-insensitively; the project name matches exactly (-cmatch).
$driverRe = '^(?i:(?:"[^"]*\\)?(?:[^"\s\\]*\\)*powershell(?:\.exe)?"?(?:\s+(?:-NoProfile|-NonInteractive|-WindowStyle\s+Hidden|-ExecutionPolicy\s+Bypass))*\s+-File\s+"?scratch[\\/]queue-driver-project\.ps1"?)\s+' + $pq + '\s*$'
$childRe = '^(?i:(?:"[^"]*\\)?(?:[^"\s\\]*[\\/])*broadsword\.exe"?)\s+queue\s+run\s+--all\s+--project\s+' + $pq + '\s*$'
$procs = @(Get-CimInstance Win32_Process)
$drivers = @($procs | Where-Object { $_.Name -eq 'powershell.exe' -and $_.CommandLine -and $_.CommandLine -cmatch $driverRe })
$driverIds = @($drivers | ForEach-Object { $_.ProcessId })
$children = @($procs | Where-Object { $_.Name -eq 'broadsword.exe' -and $_.CommandLine -and ($driverIds -contains $_.ParentProcessId) -and $_.CommandLine -cmatch $childRe })
$targets = @($drivers) + @($children)
if ($targets.Count -eq 0) { Write-Output "stop-driver: no queue driver of '$project' is running; nothing stopped"; exit 0 }
foreach ($t in $targets) { Write-Output "stop-driver: stop pid $($t.ProcessId): $($t.CommandLine)" }
if ($dry) { Write-Output 'stop-driver: dry run, nothing stopped'; exit 0 }

# Drivers first, so a driver cannot start a new child after its child is stopped.
$failed = 0
foreach ($t in $targets) {
    $now = Get-CimInstance Win32_Process -Filter "ProcessId = $([int]$t.ProcessId)"
    if ($null -eq $now) { continue }
    if ($now.CreationDate -ne $t.CreationDate -or $now.CommandLine -ne $t.CommandLine) { Write-Output "stop-driver: pid $($t.ProcessId) changed; not stopped"; continue }
    try { Stop-Process -Id $t.ProcessId -Force -ErrorAction Stop } catch { $failed++; [Console]::Error.WriteLine("stop-driver: could not stop pid $($t.ProcessId): $($_.Exception.Message)") }
}
if ($failed -gt 0) { exit 1 }
Write-Output "stop-driver: stopped $($targets.Count) process(es) of '$project'"
exit 0

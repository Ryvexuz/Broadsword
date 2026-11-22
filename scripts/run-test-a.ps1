# run-test-a.ps1: runs ONLY test A of the Stage 6 crash-sim calibration (the frozen 4.4 command of
# crash-sim docs/STAGE6-M4-M5.md), through `broadsword run crash-sim`. A temporary allow rule points at this script,
# so it is a safety boundary: it REFUSES BY DEFAULT (exit 2, before running anything) and runs only when every
# check passes. The root is the parent of this script's folder; there is no override.
#
# usage: powershell -NoProfile -File scripts/run-test-a.ps1 [--task <id>] [--body <path under the root>] [--substeps <n>]
# RUN_TEST_A_DRY=1 only prints the command; it runs after every check, so it can never weaken one.
#
# Parameters are read from $args by hand (no param block), so PowerShell's prefix matching of parameter names
# cannot accept an abbreviation or a stray argument.

$ErrorActionPreference = 'Stop'
# Any unexpected error refuses too (nothing has run before the last step).
trap { [Console]::Error.WriteLine("run-test-a: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("run-test-a: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$forbidden = @('_b', 'test-b', 'test_b', 'car_b', 'reference', 'meta')

# The frozen run flags of docs/STAGE6-M4-M5.md ("frozen: run_flags"), character for character.
$frozenFlags = '--gravity 0 --duration 0.2 --dt 0.001 --substeps 10 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20'
$defaultBody = 'scenarios/validation_wall.bsn'
$pulseReport = 'validation/test-a.pulse.csv'

function HasForbidden([string]$s) {
    $l = $s.ToLowerInvariant()
    foreach ($f in $forbidden) { if ($l.Contains($f)) { return $f } }
    return $null
}

# 1. Every argument, whatever it is, is scanned first.
$all = @($args | ForEach-Object { [string]$_ })
foreach ($a in $all) {
    $hit = HasForbidden $a
    if ($hit) { Refuse "argument '$a' contains '$hit' (test B and reference/metadata files are never used here)" }
}

# 2. Only --task, --body, --substeps, each once, each with a value.
$task = $null; $body = $null; $substeps = $null
$i = 0
while ($i -lt $all.Count) {
    $k = $all[$i]
    if ($k -cne '--task' -and $k -cne '--body' -and $k -cne '--substeps') { Refuse "argument '$k' is not allowed (only --task, --body, --substeps)" }
    if ($i + 1 -ge $all.Count) { Refuse "$k needs a value" }
    $v = $all[$i + 1]
    if ([string]::IsNullOrWhiteSpace($v)) { Refuse "$k has an empty value" }
    switch ($k) {
        '--task'     { if ($null -ne $task)     { Refuse '--task given twice' };     $task = $v }
        '--body'     { if ($null -ne $body)     { Refuse '--body given twice' };     $body = $v }
        '--substeps' { if ($null -ne $substeps) { Refuse '--substeps given twice' }; $substeps = $v }
    }
    $i += 2
}
if ($null -ne $task -and $task -cnotmatch '^t-[0-9]{8}-[0-9]{6}$') { Refuse "--task '$task' is not a task id (t-YYYYMMDD-HHMMSS)" }
if ($null -ne $substeps -and $substeps -cnotmatch '^[1-9][0-9]{0,5}$') { Refuse "--substeps '$substeps' is not a positive integer" }

# 3. The body: a path under the root, resolved, inside an allowed folder, no link on the way, an existing .bsn file.
$bodyArg = $defaultBody
if ($null -ne $body) {
    if ($body -match '[\x00-\x1f"*?<>|]') { Refuse '--body has a character that is not allowed in a path' }
    # 8.3 short names (VALIDA~1.BSN) can hide a test B file name from the marker scan: refuse every "~".
    if ($body -match '~') { Refuse "--body '$body' contains '~' (8.3 short names are not allowed)" }
    if ([IO.Path]::IsPathRooted($body)) { Refuse '--body must be a path relative to the root' }
    $full = [IO.Path]::GetFullPath((Join-Path $root $body))
    $hit = HasForbidden $full
    if ($hit) { Refuse "body path '$full' contains '$hit'" }
    if ($full -notmatch '(?i)\.bsn$') { Refuse "body '$full' is not a .bsn file" }
    $sep = [IO.Path]::DirectorySeparatorChar
    $rel = $null
    if ($full.StartsWith($root + $sep, [StringComparison]::OrdinalIgnoreCase)) { $rel = $full.Substring($root.Length + 1).Replace('\', '/') }
    if ($null -eq $rel) { Refuse "body '$full' is outside the root" }
    $okDir = ($rel -match '(?i)^projects/crash-sim/scenarios/[^/]+$') -or ($rel -match '(?i)^worktrees/crash-sim/[^/]+/scenarios/[^/]+$') -or ($rel -match '(?i)^scratch/.+$')
    if (-not $okDir) { Refuse "body '$rel' is not in projects/crash-sim/scenarios, worktrees/crash-sim/<task>/scenarios or scratch" }
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { Refuse "body '$rel' does not exist" }
    # No junction or symlink between the root and the file: the resolved path is then the path as written.
    # The final long name the file system reports is scanned for test B markers too.
    $long = (Get-Item -LiteralPath $full -Force).FullName
    $hit = HasForbidden $long
    if ($hit) { Refuse "body path '$long' contains '$hit'" }
    $p = $full
    while ($p.Length -gt $root.Length) {
        $item = Get-Item -LiteralPath $p -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Refuse "'$p' is a link; links are not followed" }
        $p = [IO.Path]::GetDirectoryName($p)
    }
    $bodyArg = $full
}

# 4. Build the command. Only now can anything run.
$flags = $frozenFlags
if ($null -ne $substeps) { $flags = $flags.Replace(' --substeps 10 ', " --substeps $substeps ") }
$exe = Join-Path $root 'build/bin/broadsword.exe'
$cmd = @('run', 'crash-sim')
if ($null -ne $task) { $cmd += @('--task', $task) }
$cmd += @('--', 'cargo', 'run', '--offline', '--release', '--', '--body', $bodyArg, '--pulse-report', $pulseReport)
$cmd += $flags.Split(' ')

Write-Output ("run-test-a: " + $exe + ' ' + ($cmd -join ' '))
if ($env:RUN_TEST_A_DRY -ceq '1') { Write-Output 'run-test-a: dry run, nothing executed'; exit 0 }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { Refuse "$exe does not exist" }
Set-Location -LiteralPath $root
& $exe @cmd
exit $LASTEXITCODE

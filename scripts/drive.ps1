# One-click manual test drive (docs/STAGE6.md checks 3.7 and 3.8, in crash-sim).
#
#   powershell -ExecutionPolicy Bypass -File scripts\drive.ps1            (from any folder)
#   powershell -ExecutionPolicy Bypass -File scripts\drive.ps1 --dry-run  (print the commands, run nothing)
#
# Steps: (1) build crash-sim in release through build\bin\broadsword.exe, (2) put the exe in a stable place,
# (3) start the render drive viewer from the render project folder (STAGE6 3.7, relative paths as written there),
# (4) keep the recorded input under a timestamped name, (5) print the 3.8 replay command for it.
# The root is found from this script's own path, never from the current folder. -DryRun and --dry-run both work.
#
# Before/after evidence (process gap 6, task t-20261121-044024-5):
#   Measure: manual-drive setup mistakes per drive session (wrong folder, deleted or missing exe).
#   Baseline: 2026-11-21 night session, Lazlo hit wrong-folder errors and a deleted program (the brief of this task;
#   no counted log of it exists, so the baseline is that report, not a number from state/pc-progress.md).
#   Judged by: the next drive sessions; the script is kept if a session needs no manual path fixing, and reverted
#   if it fails on a step the manual commands of STAGE6 3.7/3.8 would not. Tests cover --dry-run only
#   (scripts/test_drive_ps1.py); a real run, with a window, has not been exercised by the coder.

param([switch]$DryRun)

$ErrorActionPreference = 'Continue'
$rest = @($args | Where-Object { $_ -ne '--dry-run' })
if ($args -contains '--dry-run') { $DryRun = $true }
if ($rest.Count -gt 0) {
    Write-Output ("drive: FAIL unknown argument(s): " + ($rest -join ' ') + " (only --dry-run / -DryRun is accepted)")
    exit 2
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$bsw = Join-Path $root 'build\bin\broadsword.exe'
$buildExe = Join-Path $root 'build\release\crash-sim.exe'
$mainBinsExe = Join-Path $root 'build\main.bins\crash-sim\release\crash-sim.exe'
$driveDir = Join-Path $root 'scratch\drive'
$scratchExe = Join-Path $driveDir 'crash-sim.exe'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$inputRel = "scratch/drive/drive-$stamp.input"      # relative to root
$logRel = "scratch/drive/drive-$stamp.txt"
$inputFile = Join-Path $root ($inputRel -replace '/', '\')
$logFile = Join-Path $root ($logRel -replace '/', '\')
$renderDir = Join-Path $root 'projects\render'

function Say($text) { Write-Output $text }
function Step($n, $text) { Write-Output ''; Write-Output "[$n/5] $text" }
function Fail($text) { Write-Output "drive: FAIL $text"; exit 1 }

if ($DryRun) { Say 'drive: DRY RUN - the commands below are printed, not run' }
Say "root: $root"
if (-not $DryRun) {
    if (-not (Test-Path -LiteralPath $bsw)) { Fail "broadsword not found at $bsw" }
}

# 1. build
Step 1 'build crash-sim (release) through broadsword'
$buildCmd = "& `"$bsw`" run crash-sim -- cargo build --offline --release"
Say "  cwd: $root"
Say "  command: $buildCmd"
if (-not $DryRun) {
    Push-Location $root
    & $bsw run crash-sim -- cargo build --offline --release
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { Fail "build exited with code $code" }
}

# 2. stable exe: build\main.bins\crash-sim\release\crash-sim.exe if it exists, else a copy in scratch\drive
Step 2 'put crash-sim.exe in a stable place'
if (Test-Path -LiteralPath $mainBinsExe) {
    $simExe = $mainBinsExe
    Say "  build\main.bins exe exists; using it as is: $simExe"
} else {
    $simExe = $scratchExe
    Say "  build\main.bins exe not found; copying $buildExe"
    Say "  copy: Copy-Item -LiteralPath `"$buildExe`" -Destination `"$scratchExe`" -Force"
    if (-not $DryRun) {
        if (-not (Test-Path -LiteralPath $buildExe)) { Fail "the build did not produce $buildExe" }
        New-Item -ItemType Directory -Force -Path $driveDir | Out-Null
        Copy-Item -LiteralPath $buildExe -Destination $scratchExe -Force
    }
}
# Relative to projects\render (two levels below root), written out because Windows PowerShell 5.1 has no GetRelativePath.
if ($simExe -eq $mainBinsExe) { $simRel = '../../build/main.bins/crash-sim/release/crash-sim.exe' }
else { $simRel = '../../scratch/drive/crash-sim.exe' }

# 3. viewer, STAGE6 3.7, from the render project folder
Step 3 'start the drive viewer (drive with the arrow keys or WASD, hit the wall, close the window)'
$bodyRel = '../crash-sim/scenarios/pad_and_wall.bsn'
$saveRel = "../../$inputRel"
Say "  cwd: $renderDir   (the paths below are relative to it, as in STAGE6 3.7)"
Say "  command: & `"$bsw`" run render -- cargo run --offline --release --example drive -- --sim-exe $simRel --body $bodyRel --save-input $saveRel"
Say "  output also saved to: $logFile"
if (-not $DryRun) {
    New-Item -ItemType Directory -Force -Path $driveDir | Out-Null
    Push-Location $renderDir
    & $bsw run render -- cargo run --offline --release --example drive -- --sim-exe $simRel --body $bodyRel --save-input $saveRel 2>&1 | Tee-Object -FilePath $logFile
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { Fail "the viewer exited with code $code (log: $logFile)" }
}

# 4. recorded input
Step 4 'recorded input'
Say "  saved input (timestamped): $inputFile"
if (-not $DryRun) {
    if (-not (Test-Path -LiteralPath $inputFile)) { Fail "no input was saved at $inputFile (log: $logFile)" }
    Say "  ok: $((Get-Item -LiteralPath $inputFile).Length) bytes"
}

# 5. replay (STAGE6 3.8), printed for you to run; not run by this script
Step 5 'headless replay command (STAGE6 3.8), run it from the crash-sim project through broadsword'
Say "  & `"$bsw`" run crash-sim -- cargo run --offline --release -- --body scenarios/pad_and_wall.bsn --input ../../$inputRel --drive-report --lap-min-heading 360 --lap-min-path 50 --lap-close 1000 --impact-min-kmh 1 --impact-max-kmh 200"
Say '  PASS iff it prints drive: ok and its final line is identical to the final line the viewer printed.'

Say ''
if ($DryRun) { Say 'drive: dry run done (nothing was built, copied or opened)' } else { Say 'drive: done' }
exit 0

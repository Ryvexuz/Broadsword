# read-b-files.ps1 [--task <id>]: prints, READ-ONLY, exactly three crash-sim test B input files and nothing else:
#   projects/crash-sim/scenarios/validation_car_b.bsn
#   projects/crash-sim/scenarios/validation_wall_b.bsn
#   projects/crash-sim/validation/CAR_B.md
# With --task <id> it reads the same three paths in worktrees/crash-sim/<id> instead of the main checkout.
# An allow rule points at this script, so it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a reason on stderr,
# before printing anything) and prints only when every check passes. The root is the parent of this script's folder;
# there is no override. It never writes, and never prints a results path: the three paths above are fixed in this file
# and are the only files it opens. It never pushes, forces, resets, deletes, reads .env or edits .claude/.
#
# Checks, all of which must pass before the first byte is printed:
#  1. No argument, or exactly `--task <id>` with <id> a task id (t-YYYYMMDD-HHMMSS, optionally -<n>).
#  2. Each of the three files exists and no folder or file between the root and it is a junction or symlink (so a link
#     cannot redirect a fixed path to a results file).

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("read-b-files: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("read-b-files: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$files = @('scenarios/validation_car_b.bsn', 'scenarios/validation_wall_b.bsn', 'validation/CAR_B.md')

# 1. Arguments.
$all = @($args | ForEach-Object { [string]$_ })
$base = 'projects/crash-sim'
if ($all.Count -eq 0) {
} elseif ($all.Count -eq 2 -and $all[0] -ceq '--task') {
    $task = $all[1]
    if ($task -cnotmatch '^t-[0-9]{8}-[0-9]{6}(-[0-9]{1,3})?$') { Refuse "--task '$task' is not a task id (t-YYYYMMDD-HHMMSS)" }
    $base = "worktrees/crash-sim/$task"
} else {
    Refuse 'usage: read-b-files.ps1 [--task <id>] (no other argument)'
}

# 2. Every file, before anything is printed.
$full = @()
foreach ($f in $files) {
    $rel = "$base/$f"
    $p = [IO.Path]::GetFullPath((Join-Path $root $rel))
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { Refuse "'$rel' does not exist" }
    $q = $p
    while ($q.Length -gt $root.Length) {
        $item = Get-Item -LiteralPath $q -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Refuse "'$q' is a link; links are not followed" }
        $q = [IO.Path]::GetDirectoryName($q)
    }
    $full += , @($rel, $p)
}

foreach ($pair in $full) {
    Write-Output "===== $($pair[0]) ====="
    Write-Output ([IO.File]::ReadAllText($pair[1]))
}
exit 0

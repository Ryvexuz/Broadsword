# ff-reviewed.ps1 <project> <commit>: fast-forwards a project's main checkout to a REVIEWED commit. A temporary allow
# rule points at this script, so it is a safety boundary: it REFUSES BY DEFAULT (exit 2, a reason on stderr) and merges
# only when every check passes. It never pushes, resets or forces: the only git write is `git merge --ff-only`.
# The root is the parent of this script's folder; there is no override.
#
# Checks, all of which must pass:
#  1. <project> is a plain name; <commit> is a full 40-hex sha.
#  2. A queue entry (state/queue/*.entry) of that project has reviewed_tip == <commit> and status approved, merged or
#     merged_check_failed, or its log shows "review: approved" (not for a rejected or cancelled entry).
#  3. The main checkout (the root for broadsword, projects/<project> otherwise) is on branch main and clean.
#  4. Both state/queue/queue.lock and state/queue/<project>.queue.lock are opened (created if missing) and locked
#     before any queue entry is read, global first, then project, and HELD through the merge. Any doubt refuses.
#  5. The main checkout's HEAD is an ancestor of <commit>.

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("ff-reviewed: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("ff-reviewed: REFUSED: $why")
    exit 2
}

if ($args.Count -ne 2) { Refuse 'usage: ff-reviewed.ps1 <project> <commit> (exactly two arguments)' }
$project = [string]$args[0]
$commit = [string]$args[1]
if ($project -cnotmatch '^[a-z][a-z0-9_-]{0,63}$') { Refuse "project '$project' is not a plain project name" }
if ($commit -cnotmatch '^[0-9a-f]{40}$') { Refuse "commit '$commit' is not a full 40-hex sha" }

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
# A git environment variable could point git at another repository.
foreach ($v in 'GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'GIT_COMMON_DIR', 'GIT_OBJECT_DIRECTORY', 'GIT_CEILING_DIRECTORIES') {
    Remove-Item -LiteralPath "Env:$v" -ErrorAction SilentlyContinue
}

if ($project -ceq 'broadsword') { $main = $root } else { $main = Join-Path $root "projects/$project" }
if (-not (Test-Path -LiteralPath (Join-Path $main '.git'))) { Refuse "'$main' is not a git main checkout" }

# 4. The run locks: take both (non-blocking) before reading entries and keep them through the merge.
$qdir = Join-Path $root 'state/queue'
if (-not (Test-Path -LiteralPath $qdir -PathType Container)) { Refuse "queue folder '$qdir' does not exist" }
function Release-All { foreach ($s in $held) { try { $s.Dispose() } catch {} } }
try {
    $held = New-Object System.Collections.ArrayList
    $lockFiles = @((Join-Path $qdir 'queue.lock'), (Join-Path $qdir "$project.queue.lock"))
    foreach ($lf in $lockFiles) {
        $fs = $null
        try {
            $fs = New-Object IO.FileStream($lf, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
            $fs.Lock(0, [long]::MaxValue)
            [void]$held.Add($fs)
        } catch {
            if ($fs) { $fs.Dispose() }
            Refuse "queue lock '$lf' is held or cannot be checked (a queue run may be active): $($_.Exception.Message)"
        }
    }

    # 2. The queue entry, read only while both locks are held.
    $entryId = $null
    foreach ($f in (Get-ChildItem -LiteralPath $qdir -Filter '*.entry' -File)) {
        $kv = @{}
        foreach ($line in [IO.File]::ReadLines($f.FullName)) {
            $ix = $line.IndexOf(' = ')
            if ($ix -gt 0) {
                $key = $line.Substring(0, $ix)
                if (-not $kv.ContainsKey($key)) { $kv[$key] = $line.Substring($ix + 3) }
            }
        }
        if ($kv['project'] -cne $project) { continue }
        if ($kv['reviewed_tip'] -cne $commit) { continue }
        $st = $kv['status']
        $ok = $st -in @('approved', 'merged', 'merged_check_failed')
        if (-not $ok -and $st -notin @('rejected', 'cancelled') -and $kv.ContainsKey('log') -and $kv['log'].Contains('UTC review: approved')) { $ok = $true }
        if ($ok) { $entryId = $kv['id']; if (-not $entryId) { $entryId = $f.BaseName }; break }
    }
    if (-not $entryId) { Refuse "no approved queue entry of project '$project' has reviewed tip $commit" }

    # 3. Branch and cleanliness of the main checkout.
    $branch = (& git -C $main symbolic-ref --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or $branch -cne 'main') { Refuse "main checkout '$main' is not on branch main" }
    $dirty = @(& git -C $main status --porcelain --untracked-files=all 2>$null)
    if ($LASTEXITCODE -ne 0) { Refuse 'git status failed' }
    if ($dirty.Count -gt 0) { Refuse "main checkout '$main' is not clean ($($dirty.Count) change(s))" }

    # 5. Fast-forward only: the commit must exist and descend from HEAD.
    & git -C $main cat-file -e "$commit^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) { Refuse "commit $commit is not in '$main'" }
    $head = (& git -C $main rev-parse HEAD).Trim()
    if ($head -ceq $commit) { Refuse "main is already at $commit; nothing to fast-forward" }
    & git -C $main merge-base --is-ancestor $head $commit 2>$null
    if ($LASTEXITCODE -ne 0) { Refuse "$commit is not a fast-forward of main ($head)" }

    Write-Output "ff-reviewed: relying on queue entry $entryId (project $project, reviewed tip $commit)"
    & git -C $main merge --ff-only $commit
    $code = $LASTEXITCODE
} finally {
    Release-All
}
if ($code -ne 0) { [Console]::Error.WriteLine("ff-reviewed: git merge --ff-only failed ($code)"); exit $code }
Write-Output "ff-reviewed: $project main fast-forwarded to $commit"
exit 0

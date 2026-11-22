# restart-orch.ps1: decides whether a fresh orchestrator session is needed and, if so, writes state/restart-request.md
# (time, newest handoff path, reason) and exits 0. It starts NO session itself. An allow rule points at this script, so
# it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a reason on stderr, before writing anything) and writes the
# request only when every check passes. The root is the parent of this script's folder; there is no override. It never
# pushes, forces, resets, deletes files or history, reads .env or edits .claude/. Its only write is that one file.
#
# Checks, all of which must pass (times are UTC; pc-progress lines start with YYYY-MM-DDTHH:MM:SSZ):
#  1. No argument at all.
#  2. The orchestrator went quiet: the last non-empty state/pc-progress.md line that does not contain "(tick-loop)" is
#     over 60 minutes old. If that line has no readable time, it refuses.
#  3. It went quiet right after a handoff: a newest state/handoff-*.md exists (by last-write time; no link), and it was
#     last written no more than 5 minutes BEFORE that line (later than the line is fine). The orchestrator logs its
#     "handoff written" line about a second after writing the handoff file, so the handoff is usually a little OLDER
#     than the last line (real pattern: handoff 17:22:15Z, line 17:22:16Z).
#  4. No request was written in the last 2 hours: state/restart-request.md, if it exists, was last written over 2 hours
#     ago and its `time:` line (if readable) is over 2 hours old too.
# Concurrent runs: checks 2 to 4 and the write run while this run holds state/restart-request.lock, created with an
# exclusive create (FileMode.CreateNew: a second run cannot create it and refuses). The OS removes that lock file when
# its handle closes (FileOptions.DeleteOnClose), also if the run is killed; it is the only file this script removes,
# and only the one it just created itself.

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("restart-orch: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("restart-orch: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$quietMinutes = 60
$handoffBeforeMinutes = 5
$requestGapHours = 2
$now = [DateTime]::UtcNow

function NoLinks([string]$path) {
    $p = $path
    while ($p.Length -gt $root.Length) {
        if ((Get-Item -LiteralPath $p -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
        $p = [IO.Path]::GetDirectoryName($p)
    }
    return $true
}

function LineTime([string]$line) {
    if ($line -cmatch '^([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2})Z') {
        $t = [DateTime]::MinValue
        $style = [Globalization.DateTimeStyles]::AssumeUniversal -bor [Globalization.DateTimeStyles]::AdjustToUniversal
        if ([DateTime]::TryParseExact($Matches[1], 'yyyy-MM-ddTHH:mm:ss', [Globalization.CultureInfo]::InvariantCulture, $style, [ref]$t)) { return $t }
    }
    return $null
}

# 1. Arguments.
if ($args.Count -ne 0) { Refuse 'usage: restart-orch.ps1 (no arguments)' }

$state = Join-Path $root 'state'
if (-not (Test-Path -LiteralPath $state -PathType Container)) { Refuse "'$state' does not exist" }
if (-not (NoLinks $state)) { Refuse "'$state' is a link; links are not followed" }
# The lock: an exclusive create, held until this process ends (its handle closes then and the OS removes the file).
$lockPath = Join-Path $state 'restart-request.lock'
try {
    $lock = New-Object IO.FileStream($lockPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None, 1, [IO.FileOptions]::DeleteOnClose)
} catch {
    Refuse "state/restart-request.lock exists (another restart-orch run holds it, or a stale lock is left over): $($_.Exception.Message)"
}

# 2 and 3. The last non-tick-loop line and the newest handoff.
$handoffs = @(Get-ChildItem -LiteralPath $state -Filter 'handoff-*.md' -File -Force | Where-Object { $_.Name -cmatch '^handoff-[A-Za-z0-9._-]+\.md$' })
if ($handoffs.Count -eq 0) { Refuse 'no state/handoff-*.md exists' }
$newest = $handoffs | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if (-not (NoLinks $newest.FullName)) { Refuse "'$($newest.FullName)' is a link; links are not followed" }
$handoffTime = $newest.LastWriteTimeUtc
$handoffRel = "state/$($newest.Name)"

$pc = Join-Path $state 'pc-progress.md'
if (-not (Test-Path -LiteralPath $pc -PathType Leaf)) { Refuse 'state/pc-progress.md does not exist' }
if (-not (NoLinks $pc)) { Refuse 'state/pc-progress.md is a link; links are not followed' }
$lines = @([IO.File]::ReadAllLines($pc) | Where-Object { $_.Trim().Length -gt 0 })
$last = $null
for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    if (-not $lines[$i].Contains('(tick-loop)')) { $last = $lines[$i]; break }
}
if ($null -eq $last) { Refuse 'state/pc-progress.md has no line that is not a (tick-loop) line' }
$lastTime = LineTime $last
if ($null -eq $lastTime) { Refuse "the last non-(tick-loop) line of state/pc-progress.md has no readable time: '$last'" }
if (-not (($now - $lastTime).TotalMinutes -gt $quietMinutes)) {
    Refuse ("the last non-(tick-loop) pc-progress line ({0:s}Z) is not over {1} min old" -f $lastTime, $quietMinutes)
}
if ($handoffTime -lt $lastTime.AddMinutes(-$handoffBeforeMinutes)) {
    Refuse ("newest handoff $handoffRel ({0:s}Z) is more than {2} min before the last pc-progress line ({1:s}Z): the orchestrator did not stop right after a handoff" -f $handoffTime, $lastTime, $handoffBeforeMinutes)
}

# 4. No request in the last 2 hours.
$reqPath = Join-Path $state 'restart-request.md'
if (Test-Path -LiteralPath $reqPath) {
    if (-not (Test-Path -LiteralPath $reqPath -PathType Leaf) -or -not (NoLinks $reqPath)) { Refuse "'$reqPath' is not a plain file" }
    $reqWritten = (Get-Item -LiteralPath $reqPath -Force).LastWriteTimeUtc
    if (($now - $reqWritten).TotalHours -lt $requestGapHours) { Refuse ("a restart request was written at {0:s}Z, under {1} h ago" -f $reqWritten, $requestGapHours) }
    foreach ($l in [IO.File]::ReadAllLines($reqPath)) {
        if ($l -cmatch '^time: (\S+)$') {
            $t = LineTime $Matches[1]
            if ($null -ne $t -and ($now - $t).TotalHours -lt $requestGapHours) { Refuse ("the restart request's time {0:s}Z is under {1} h ago" -f $t, $requestGapHours) }
        }
    }
}

# Only now is anything written.
$reason = ('the last non-(tick-loop) pc-progress line ({1:s}Z) is over {2} min old and came right after the newest handoff ({0:s}Z)' -f $handoffTime, $lastTime, $quietMinutes)
$body = "# restart request`n`ntime: $($now.ToString('yyyy-MM-ddTHH:mm:ss'))Z`nhandoff: $handoffRel`nreason: $reason`n"
[IO.File]::WriteAllText($reqPath, $body)
$lock.Dispose()
Write-Output "restart-orch: wrote state/restart-request.md (handoff $handoffRel): $reason"
exit 0

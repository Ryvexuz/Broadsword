# restart-orch.ps1: decides whether a fresh orchestrator session is needed and, if so, writes state/restart-request.md
# (time, newest handoff path, reason) and exits 0. It starts NO session itself. An allow rule points at this script, so
# it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a reason on stderr, before writing anything) and writes the
# request only when every check passes. The root is the parent of this script's folder; there is no override. It never
# pushes, forces, resets, deletes files or history, reads .env or edits .claude/. Its only write is that one file.
#
# Checks, all of which must pass (times are UTC; pc-progress lines start with YYYY-MM-DDTHH:MM:SSZ):
#  1. No argument at all.
#  2. A newest state/handoff-*.md exists (by last-write time; no link), and it was written AFTER the time of the last
#     non-empty state/pc-progress.md line that does not contain "(tick-loop)". If that line has no readable time, it
#     refuses.
#  3. No line containing "orchestrator" (any case) in state/pc-progress.md is from the last 60 minutes. A line without
#     a readable time counts as written at the time of the next timed line after it (an upper bound, so it is never
#     treated as older than it is); one with no timed line after it refuses.
#  4. No request was written in the last 2 hours: state/restart-request.md, if it exists, was last written over 2 hours
#     ago and its `time:` line (if readable) is over 2 hours old too.

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("restart-orch: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("restart-orch: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$quietMinutes = 60
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

# 2. The newest handoff and the last non-tick-loop line.
$state = Join-Path $root 'state'
if (-not (Test-Path -LiteralPath $state -PathType Container)) { Refuse "'$state' does not exist" }
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
if (-not ($handoffTime -gt $lastTime)) {
    Refuse ("newest handoff $handoffRel ({0:s}Z) is not newer than the last pc-progress line ({1:s}Z)" -f $handoffTime, $lastTime)
}

# 3. No orchestrator line in the last 60 minutes.
$nextTime = $null
$lastOrch = $null
for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    $t = LineTime $lines[$i]
    if ($null -ne $t) { $nextTime = $t }
    if ($lines[$i].IndexOf('orchestrator', [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        if ($null -eq $t) {
            if ($null -eq $nextTime) { Refuse "an orchestrator line has no readable time and no timed line after it: '$($lines[$i])'" }
            $t = $nextTime
        }
        if ($null -eq $lastOrch -or $t -gt $lastOrch) { $lastOrch = $t }
        if (($now - $t).TotalMinutes -lt $quietMinutes) {
            Refuse ("an orchestrator line of state/pc-progress.md is from {0:s}Z, under {1} min ago" -f $t, $quietMinutes)
        }
    }
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
$orchText = if ($null -eq $lastOrch) { 'no orchestrator line at all' } else { 'last orchestrator line {0:s}Z, over {1} min ago' -f $lastOrch, $quietMinutes }
$reason = ('newest handoff ({0:s}Z) is newer than the last pc-progress line ({1:s}Z); {2}' -f $handoffTime, $lastTime, $orchText)
$body = "# restart request`n`ntime: $($now.ToString('yyyy-MM-ddTHH:mm:ss'))Z`nhandoff: $handoffRel`nreason: $reason`n"
[IO.File]::WriteAllText($reqPath, $body)
Write-Output "restart-orch: wrote state/restart-request.md (handoff $handoffRel): $reason"
exit 0

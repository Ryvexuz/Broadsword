# abandon-stuck.ps1 <job id> [--dry-run]: gives up on ONE stuck broadsword job with `broadsword recover --abandon <id>`
# (which changes no file: the record becomes `abandoned`, the record and all refs stay). An allow rule points at this
# script, so it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a reason on stderr, before any side effect) and
# runs only when every check passes. The root is the parent of this script's folder; there is no override. It never
# pushes, forces, resets, deletes files or history, reads .env or edits .claude/.
#
# Checks, all of which must pass:
#  1. Exactly one argument (plus an optional trailing --dry-run): a job id in the form `broadsword jobs` prints,
#     YYYYMMDD-HHMMSS-<project>-<kind> (lowercase letters, digits, '-', '_').
#  2. state/jobs/<id>.rec exists (no link on the way), its `id` is exactly <id> and its `status` is `running`.
#  3. The job is stuck, by what the record holds:
#     - DEAD: its recorded broadsword `pid` names no live process, or names a live process whose creation time differs
#       from the recorded `pid_start` (the pid was reused); a live process whose creation time cannot be compared
#       counts as alive; or
#     - STALE: its log (the record's `log`, which must be a file under the root) AND its record were both last
#       written more than 30 minutes ago.
#     If neither the pid nor the log can be read, it refuses. A live, fresh job refuses.
# broadsword itself refuses again if its pid is still alive (recover.rs `abandon`).
# After the run it appends one line to state/pc-progress.md (id, reason, exit code).
# --dry-run runs every check, then only prints the command; it runs nothing and appends nothing.

$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine("abandon-stuck: REFUSED: unexpected error: $($_.Exception.Message)"); exit 2 }

function Refuse([string]$why) {
    [Console]::Error.WriteLine("abandon-stuck: REFUSED: $why")
    exit 2
}

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sep = [IO.Path]::DirectorySeparatorChar
$staleMinutes = 30

function NoLinks([string]$path) {
    $p = $path
    while ($p.Length -gt $root.Length) {
        $item = Get-Item -LiteralPath $p -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
        $p = [IO.Path]::GetDirectoryName($p)
    }
    return $true
}

# 1. Arguments.
$all = @($args | ForEach-Object { [string]$_ })
$dry = $false
if ($all.Count -eq 2 -and $all[1] -ceq '--dry-run') { $dry = $true; $all = @($all[0]) }
if ($all.Count -ne 1) { Refuse 'usage: abandon-stuck.ps1 <job id> [--dry-run] (exactly one job id)' }
$id = $all[0]
if ($id -cnotmatch '^[0-9]{8}-[0-9]{6}-[a-z0-9][a-z0-9_-]{0,80}$') { Refuse "'$id' is not a job id (YYYYMMDD-HHMMSS-<project>-<kind>, see broadsword jobs)" }

# 2. The record.
$rec = Join-Path $root "state/jobs/$id.rec"
if (-not (Test-Path -LiteralPath $rec -PathType Leaf)) { Refuse "no job record state/jobs/$id.rec" }
if (-not (NoLinks $rec)) { Refuse "the path to state/jobs/$id.rec has a link; links are not followed" }
$kv = @{}
foreach ($line in [IO.File]::ReadAllLines($rec)) {
    $ix = $line.IndexOf(' = ')
    if ($ix -gt 0) {
        $key = $line.Substring(0, $ix)
        if ($kv.ContainsKey($key)) { Refuse "the record has key '$key' twice" }
        $kv[$key] = $line.Substring($ix + 3)
    }
}
if ($kv['id'] -cne $id) { Refuse "the record's id '$($kv['id'])' is not '$id'" }
if ($kv['status'] -cne 'running') { Refuse "job $id is '$($kv['status'])', not 'running': nothing to abandon" }

# 3. Dead or stale.
$now = [DateTime]::UtcNow
$reason = $null
$pidRead = $false
$pidText = [string]$kv['pid']
if ($pidText -cmatch '^[0-9]{1,10}$' -and [long]$pidText -gt 0 -and [long]$pidText -le [int]::MaxValue) {
    $pidRead = $true
    $proc = Get-Process -Id ([int]$pidText) -ErrorAction SilentlyContinue
    if ($null -eq $proc) {
        $reason = "pid $pidText is not alive"
    } else {
        $startText = [string]$kv['pid_start']
        $actual = $null
        try { $actual = $proc.StartTime.ToUniversalTime().ToFileTimeUtc() } catch { $actual = $null }
        if ($startText -cmatch '^[0-9]{1,20}$' -and $null -ne $actual) {
            # Get-Process start times are rounded differently from GetProcessTimes; only a gap over 1 s counts as reuse.
            if ([Math]::Abs([decimal]$actual - [decimal]$startText) -gt 10000000) { $reason = "pid $pidText now belongs to another process (creation time differs)" }
        }
    }
}
$logRead = $false
if ($null -eq $reason) {
    $logText = ([string]$kv['log']).Replace('\\', '\')
    if ($logText -and -not ($logText -match '[\x00-\x1f"*?<>|]')) {
        $logFull = $null
        try { $logFull = [IO.Path]::GetFullPath($(if ([IO.Path]::IsPathRooted($logText)) { $logText } else { Join-Path $root $logText })) } catch { $logFull = $null }
        if ($logFull -and $logFull.StartsWith($root + $sep, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $logFull -PathType Leaf) -and (NoLinks $logFull)) {
            $logRead = $true
            $logAge = ($now - (Get-Item -LiteralPath $logFull -Force).LastWriteTimeUtc).TotalMinutes
            $recAge = ($now - (Get-Item -LiteralPath $rec -Force).LastWriteTimeUtc).TotalMinutes
            if ($logAge -gt $staleMinutes -and $recAge -gt $staleMinutes) {
                $reason = ('log and record unchanged for {0:N0} min (over {1})' -f [Math]::Min($logAge, $recAge), $staleMinutes)
            }
        }
    }
}
if ($null -eq $reason) {
    if (-not $pidRead -and -not $logRead) { Refuse "job ${id}: neither its pid nor its log can be read, so it cannot be shown to be stuck" }
    Refuse "job $id is not stuck: pid $pidText is alive and its log or record changed in the last $staleMinutes min"
}

# Only now can anything run.
$exe = Join-Path $root 'build/bin/broadsword.exe'
Write-Output "abandon-stuck: $exe recover --abandon $id ($reason)"
if ($dry) { Write-Output 'abandon-stuck: dry run, nothing executed'; exit 0 }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { Refuse "$exe does not exist" }
Set-Location -LiteralPath $root
& $exe recover --abandon $id
$code = $LASTEXITCODE
$stamp = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss') + 'Z'
[IO.File]::AppendAllText((Join-Path $root 'state/pc-progress.md'), "$stamp abandon-stuck: recover --abandon $id ($reason) -> exit $code`n")
exit $code

# stop-driver.ps1 <project> [--when-idle-wait] [--dry-run]: stops the per-project queue driver of ONE project and
# nothing else. An allow rule points at this script, so it is a SAFETY BOUNDARY: it REFUSES BY DEFAULT (exit 2, a
# reason on stderr, before stopping anything) and stops processes only when every check passes. The root is the parent
# of this script's folder; there is no override. It never pushes, forces, resets, deletes files or history, reads .env
# or edits .claude/.
#
# What it stops (found by command line, Win32_Process), and only that: the OUTER driver loop, a powershell.exe whose
# WHOLE command line is `<powershell> [-NoProfile|-NonInteractive|-WindowStyle Hidden|-ExecutionPolicy Bypass ...]
# -File <script> <project>` where <script> is THIS root's scratch/queue-driver-project.ps1: either the absolute path
# <root>\scratch\queue-driver-project.ps1, or the relative `scratch/queue-driver-project.ps1` (the form start-driver.ps1
# uses) of a process whose current folder (read from the process itself) is <root>. A driver of another copy of the
# project folder is left alone; a relative-path driver whose current folder cannot be read refuses the whole run.
# A command line that merely contains these words (a -Command, a grep, another project) does not match.
# It NEVER stops the driver's child `broadsword.exe queue run --all --project <project>`: a child already running is
# left to finish its drain (it exits when the project is idle), so a merge in progress is never killed.
# Each stop is bound to the process, not the pid: a handle to the driver is opened first (Windows does not reuse a pid
# while a handle to it is open), then the driver is re-read by pid and must have the same creation time and command
# line, and the handle's own creation time must match too; the kill goes through that handle (TerminateProcess). There
# is no kill by pid. The queue is re-checked for a running entry right before each stop too.
#
# Checks, all of which must pass:
#  1. <project> is a plain project name; the only other arguments are --when-idle-wait and --dry-run, each once.
#  2. The queue folder state/queue exists and no entry of <project> (state/queue/*.entry, `project = <project>`) has
#     `status = running` (a merge is in progress). With --when-idle-wait it re-checks every 5 s for up to 30 min and
#     refuses if an entry is still running then. An entry file that cannot be read refuses.
# --dry-run runs every check, then only prints what it would stop.

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

# 3. Find THIS root's driver by exact command line and script location.
if (-not ('BsProcCwd' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class BsProcCwd {
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(int access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool IsWow64Process(IntPtr h, out bool wow);
    [DllImport("kernel32.dll")] static extern bool ReadProcessMemory(IntPtr h, IntPtr addr, byte[] buf, IntPtr size, out IntPtr read);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationProcess(IntPtr h, int cls, long[] info, int len, out int retLen);
    [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr h, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out uint code);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr h, uint code);
    // A handle to the process for the stop (PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION), or IntPtr.Zero.
    // While it is open, Windows does not hand this pid to another process, so the pid names this one process.
    public static IntPtr OpenForStop(int pid) { return OpenProcess(0x1001, false, pid); }
    public static void Close(IntPtr h) { CloseHandle(h); }
    public static bool HasExited(IntPtr h) { uint code; return !(GetExitCodeProcess(h, out code) && code == 259); }
    // True if the handle's process was created at `when` (a CIM CreationDate), to 1 microsecond (CIM's precision).
    public static bool SameStart(IntPtr h, DateTime when) {
        long c, e, k, u;
        if (!GetProcessTimes(h, out c, out e, out k, out u)) return false;
        return Math.Abs(c - when.ToFileTime()) < 10;
    }
    public static bool Terminate(IntPtr h) { return TerminateProcess(h, 1); }
    static byte[] Read(IntPtr h, long addr, int n) {
        byte[] b = new byte[n]; IntPtr got;
        if (!ReadProcessMemory(h, new IntPtr(addr), b, new IntPtr(n), out got) || got.ToInt64() != n) return null;
        return b;
    }
    // The current folder of a 64-bit process (PEB -> ProcessParameters -> CurrentDirectory), or null if unreadable.
    public static string Get(int pid) {
        if (IntPtr.Size != 8) return null;
        IntPtr h = OpenProcess(0x0410, false, pid); // PROCESS_QUERY_INFORMATION | PROCESS_VM_READ
        if (h == IntPtr.Zero) return null;
        try {
            bool wow;
            if (!IsWow64Process(h, out wow) || wow) return null;
            long[] pbi = new long[6]; int rl;
            if (NtQueryInformationProcess(h, 0, pbi, 48, out rl) != 0) return null;
            byte[] pp = Read(h, pbi[1] + 0x20, 8); if (pp == null) return null;
            byte[] us = Read(h, BitConverter.ToInt64(pp, 0) + 0x38, 16); if (us == null) return null;
            int len = BitConverter.ToUInt16(us, 0);
            if (len == 0 || len > 65534 || (len % 2) != 0) return null;
            byte[] s = Read(h, BitConverter.ToInt64(us, 8), len); if (s == null) return null;
            return System.Text.Encoding.Unicode.GetString(s);
        } finally { CloseHandle(h); }
    }
}
'@
}
$pq = [regex]::Escape($project)
$ownScript = Join-Path $root 'scratch\queue-driver-project.ps1'
# The program path and flags match case-insensitively; the project name matches exactly (-cmatch). Group 1 or 2 is the
# -File path, checked below against THIS root.
$driverRe = '^(?i:(?:"[^"]*\\)?(?:[^"\s\\]*\\)*powershell(?:\.exe)?"?(?:\s+(?:-NoProfile|-NonInteractive|-WindowStyle\s+Hidden|-ExecutionPolicy\s+Bypass))*\s+-File\s+(?:"([^"]+)"|([^"\s]+)))\s+' + $pq + '\s*$'
function SameFolder([string]$a, [string]$b) {
    try { $fa = [IO.Path]::GetFullPath($a).TrimEnd('\', '/'); $fb = [IO.Path]::GetFullPath($b).TrimEnd('\', '/') } catch { return $false }
    return [string]::Equals($fa, $fb, [StringComparison]::OrdinalIgnoreCase)
}
# 'own' = this root's driver, 'other' = not this root's, 'unknown' = relative path whose current folder cannot be read.
function DriverOf($proc) {
    if ($proc.Name -ne 'powershell.exe' -or -not $proc.CommandLine -or $proc.CommandLine -cnotmatch $driverRe) { return 'none' }
    $path = if ($Matches[1]) { $Matches[1] } else { $Matches[2] }
    if ($path -match '^(?i:scratch[\\/]queue-driver-project\.ps1)$') {
        $cwd = [BsProcCwd]::Get([int]$proc.ProcessId)
        if ($null -eq $cwd) { return 'unknown' }
        if (SameFolder $cwd $root) { return 'own' }
        return 'other'
    }
    if ([IO.Path]::IsPathRooted($path) -and (SameFolder $path $ownScript)) { return 'own' }
    return 'other'
}
$procs = @(Get-CimInstance Win32_Process)
$targets = @()
foreach ($p in $procs) {
    $kind = DriverOf $p
    if ($kind -eq 'unknown') { Refuse "the current folder of driver pid $($p.ProcessId) cannot be read, so it cannot be shown to be this root's driver: $($p.CommandLine)" }
    if ($kind -eq 'other') { Write-Output "stop-driver: pid $($p.ProcessId) is a driver of another folder; left alone" }
    if ($kind -eq 'own') { $targets += $p }
}
$driverIds = @($targets | ForEach-Object { $_.ProcessId })
foreach ($c in @($procs | Where-Object { $_.Name -eq 'broadsword.exe' -and ($driverIds -contains $_.ParentProcessId) })) {
    Write-Output "stop-driver: child pid $($c.ProcessId) is left to finish its drain (never stopped): $($c.CommandLine)"
}
if ($targets.Count -eq 0) { Write-Output "stop-driver: no queue driver of '$project' is running; nothing stopped"; exit 0 }
foreach ($t in $targets) { Write-Output "stop-driver: stop pid $($t.ProcessId): $($t.CommandLine)" }
if ($dry) { Write-Output 'stop-driver: dry run, nothing stopped'; exit 0 }

# Only the outer driver loops are stopped; each one re-read and the queue re-checked right before its stop.
$failed = 0
foreach ($t in $targets) {
    $nowRunning = RunningEntries
    if ($nowRunning.Count -gt 0) { Refuse "queue entry $($nowRunning -join ', ') of '$project' started running; pid $($t.ProcessId) not stopped" }
    $id = [int]$t.ProcessId
    # The stop is bound to the process, not the pid: open a handle first (the pid cannot be reused while it is open),
    # verify through it, and terminate through it. There is no kill by pid.
    $h = [BsProcCwd]::OpenForStop($id)
    if ($h -eq [IntPtr]::Zero) {
        if ($null -eq (Get-CimInstance Win32_Process -Filter "ProcessId = $id")) { Write-Output "stop-driver: pid $id exited; nothing to stop"; continue }
        $failed++; [Console]::Error.WriteLine("stop-driver: could not open pid $id to stop it"); continue
    }
    try {
        if ([BsProcCwd]::HasExited($h)) { Write-Output "stop-driver: pid $id exited; nothing to stop"; continue }
        $now = Get-CimInstance Win32_Process -Filter "ProcessId = $id"
        if ($null -eq $now -or $now.CreationDate -ne $t.CreationDate -or $now.CommandLine -ne $t.CommandLine -or (DriverOf $now) -ne 'own' -or -not [BsProcCwd]::SameStart($h, $t.CreationDate)) {
            Write-Output "stop-driver: pid $id changed; not stopped"; continue
        }
        if (-not [BsProcCwd]::Terminate($h)) { $failed++; [Console]::Error.WriteLine("stop-driver: could not stop pid $id") }
    } finally { [BsProcCwd]::Close($h) }
}
if ($failed -gt 0) { exit 1 }
Write-Output "stop-driver: stopped $($targets.Count) process(es) of '$project'"
exit 0

# Tracked copy of scratch/start-driver.ps1 (task t-20261121-201016); scratch/ copy stays in use until the references move.
# Start scripts/queue-driver-project.ps1 unless the lane has a driver. usage: start-driver.ps1 <project> [-DryRun]
param([Parameter(Mandatory=$true)][string]$Project, [switch]$DryRun)
$root = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
Set-Location $root
$rootPrefix = $root.TrimEnd('\') + '\'
$escapedProject = [regex]::Escape($Project)
$driverPattern = "queue-driver-project\.ps1\s+$escapedProject(\s|$)"
$queuePattern = '(?:^|\s)queue\s+run(?=\s|$)[\s\S]*?\s--project\s+(?:"' + $escapedProject + '"|' + $escapedProject + ')(?=\s|$)'
$processes = @(Get-CimInstance Win32_Process -Filter "Name='powershell.exe' OR Name='broadsword.exe'")

# Missing process metadata is not evidence that a lane is being driven.
$readable = @($processes | Where-Object { $_.ExecutablePath -and $_.CommandLine })
$running = $readable | Where-Object {
    $_.Name -ieq 'powershell.exe' -and $_.CommandLine -imatch $driverPattern
} | Select-Object -First 1
if ($running) {
    "driver for $Project already running (pid $($running.ProcessId))"
    exit 0
}

$running = $readable | Where-Object {
    if ($_.Name -ine 'broadsword.exe' -or $_.CommandLine -inotmatch $queuePattern) {
        return $false
    }
    try {
        # Reject drive-relative and root-relative metadata before full-path resolution.
        if ($_.ExecutablePath -notmatch '^(?:[a-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+[\\/])') {
            return $false
        }
        $exe = [IO.Path]::GetFullPath($_.ExecutablePath)
        $exe.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)
    } catch {
        $false
    }
} | Select-Object -First 1
if ($running) {
    "driver for $Project already running (queue run pid $($running.ProcessId))"
} elseif ($DryRun) {
    "dry run: would start driver for $Project"
} else {
    Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-File','scripts/queue-driver-project.ps1',$Project
    "started driver for $Project"
}
exit 0

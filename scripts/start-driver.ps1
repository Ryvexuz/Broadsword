# Tracked copy of scratch/start-driver.ps1 (task t-20261121-201016); scratch/ copy stays in use until the references move.
# Start scripts/queue-driver-project.ps1 for a project unless one already runs. usage: start-driver.ps1 <project>
param([Parameter(Mandatory=$true)][string]$Project)
Set-Location (Split-Path $PSScriptRoot -Parent)
$running = Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | Where-Object { $_.CommandLine -match "queue-driver-project\.ps1\s+$([regex]::Escape($Project))(\s|$)" }
if ($running) { "driver for $Project already running (pid $($running.ProcessId))" } else { Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile','-File','scripts/queue-driver-project.ps1',$Project; "started driver for $Project" }

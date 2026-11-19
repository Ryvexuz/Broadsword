# A1 evidence: run the replay viewer in a normal on-screen window and capture ONLY that window's rectangle.
param([string]$Root = 'C:\Users\MrComputer\Desktop\Broadsword')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
"@
$out = Join-Path $Root 'scratch\a1'
$exe = Join-Path $Root 'build\release\examples\replay.exe'
$csv = Join-Path $Root 'projects\render\tests\fixtures\trajectory.csv'
$p = Start-Process -FilePath $exe -ArgumentList "`"$csv`"" -PassThru -WorkingDirectory (Join-Path $Root 'projects\render')
$t0 = Get-Date
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 100 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 50; $p.Refresh(); $h = $p.MainWindowHandle }
if ($h -eq [IntPtr]::Zero) { throw "no window appeared" }
[W]::SetForegroundWindow($h) | Out-Null
$log = @("pid=$($p.Id) handle=$h title='$($p.MainWindowTitle)' visible=$([W]::IsWindowVisible($h))")
foreach ($at in 0.3, 0.7, 1.2, 2.0, 3.5) {
  $wait = $at - ((Get-Date) - $t0).TotalSeconds
  if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]($wait * 1000)) }
  $r = New-Object W+RECT; [W]::GetWindowRect($h, [ref]$r) | Out-Null
  $w = $r.R - $r.L; $hh = $r.B - $r.T
  $bmp = New-Object System.Drawing.Bitmap $w, $hh
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size $w, $hh))
  $name = 'replay-t{0:0.0}s.png' -f $at
  $bmp.Save((Join-Path $out $name), [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
  $log += "{0:0.00}s rect=({1},{2})-({3},{4}) size={5}x{6} -> {7}" -f ((Get-Date) - $t0).TotalSeconds, $r.L, $r.T, $r.R, $r.B, $w, $hh, $name
}
$closed = $p.CloseMainWindow()
if (-not $p.WaitForExit(5000)) { $log += "did not exit within 5 s after close" } else { $log += "closed by CloseMainWindow=$closed, exit code $($p.ExitCode)" }
$log | Out-File -Encoding utf8 (Join-Path $out 'capture.log')
$log

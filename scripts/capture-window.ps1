# Capture a screenshot of an application's main window.
#
# Used as a manual/automated smoke check that the packaged UI actually renders
# (a launched process alone does not prove the WebView painted anything).
#
#   powershell -File scripts/capture-window.ps1 -ProcessId 1234 -OutFile shot.png

param(
    [Parameter(Mandatory = $true)][int]$ProcessId,
    [Parameter(Mandatory = $true)][string]$OutFile,
    [int]$WaitMs = 2500
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class CaaWin {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$proc = Get-Process -Id $ProcessId -ErrorAction Stop
$handle = $proc.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) {
    throw "process $ProcessId has no main window yet"
}

[void][CaaWin]::ShowWindow($handle, 5)     # SW_SHOW
[void][CaaWin]::SetForegroundWindow($handle)
Start-Sleep -Milliseconds $WaitMs

$rect = New-Object CaaWin+RECT
if (-not [CaaWin]::GetWindowRect($handle, [ref]$rect)) {
    throw "GetWindowRect failed"
}

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) {
    throw "window has non-positive size ${width}x${height}"
}

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
$directory = Split-Path -Parent $OutFile
if ($directory -and -not (Test-Path $directory)) {
    New-Item -ItemType Directory -Force -Path $directory | Out-Null
}
$bitmap.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()

Write-Output "captured ${width}x${height} -> $OutFile"

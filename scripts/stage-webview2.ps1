# Stage the WebView2 loader next to build outputs.
#
# `webview2-com-sys` links against `WebView2Loader.dll` dynamically for the
# GNU/Windows target and ships the DLL inside its crate source. The Windows
# loader only searches the executable's own directory (and PATH), so the DLL must
# sit beside every produced .exe — including test harnesses.
#
# Run after `cargo build` / before `cargo test`:
#     powershell -File scripts/stage-webview2.ps1

param(
    [string]$TargetDir = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "D:\caa-target" })
)

$ErrorActionPreference = "Stop"

$candidates = Get-ChildItem -Path $TargetDir -Recurse -Filter "WebView2Loader.dll" -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } |
    Sort-Object LastWriteTime -Descending

if (-not $candidates) {
    Write-Output "WebView2Loader.dll not found under $TargetDir (build the crate first)."
    exit 0
}

$loader = $candidates[0].FullName
Write-Output "source: $loader"

$allDestinations = @(
    (Join-Path $TargetDir "debug"),
    (Join-Path $TargetDir "debug\deps"),
    (Join-Path $TargetDir "release"),
    (Join-Path $TargetDir "release\deps")
)

foreach ($dir in $allDestinations) {
    if (-not (Test-Path $dir)) { continue }
    Copy-Item -Path $loader -Destination (Join-Path $dir "WebView2Loader.dll") -Force
    Write-Output "copied -> $dir\WebView2Loader.dll"
}

# Keep a copy in the repository so the installer can ship it as a resource.
$resourceDir = Join-Path (Split-Path $PSScriptRoot -Parent) "src-tauri\webview2"
New-Item -ItemType Directory -Force -Path $resourceDir | Out-Null
Copy-Item -Path $loader -Destination (Join-Path $resourceDir "WebView2Loader.dll") -Force
Write-Output "copied -> src-tauri\webview2\WebView2Loader.dll"

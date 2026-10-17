# Shared environment for building this project on Windows.
#
# Dot-source it before building:   . .\scripts\env.ps1
#
# The machine this project was developed on has no MSVC build tools (installing
# them requires elevation), so the Windows build uses the Rust GNU toolchain with
# a portable MinGW-w64. Tauri supports x86_64-pc-windows-gnu; see docs/decisions.

$ErrorActionPreference = "Continue"

$script:ToolRoot = if ($env:CAA_TOOLCHAIN_ROOT) { $env:CAA_TOOLCHAIN_ROOT } else { "D:\toolchain" }

$env:RUST_HOME = Join-Path $script:ToolRoot "rust"
$env:MINGW_HOME = Join-Path $script:ToolRoot "mingw64"
$env:ANDROID_HOME = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { "$env:LOCALAPPDATA\Android\Sdk" }
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME
$env:NDK_HOME = if ($env:NDK_HOME) { $env:NDK_HOME } else { Join-Path $env:ANDROID_HOME "ndk\27.2.12479018" }
$env:JAVA_HOME = if ($env:JAVA_HOME) { $env:JAVA_HOME } else { "C:\Program Files\Eclipse Adoptium\jdk-17.0.20.101-hotspot" }

# Rust must find a linker; the GNU toolchain uses MinGW's gcc.
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $env:MINGW_HOME "bin\gcc.exe"
$env:CC = Join-Path $env:MINGW_HOME "bin\gcc.exe"
$env:AR = Join-Path $env:MINGW_HOME "bin\ar.exe"
$env:WINDRES = Join-Path $env:MINGW_HOME "bin\windres.exe"

# The workspace path contains a space ("Canvas Learning Assistant"). MinGW's
# windres receives unquoted -I paths from tauri-winres and fails with
# "cc1.exe: fatal error: Learning\: No such file or directory". Building into a
# space-free target directory keeps every path windres sees space-free.
if (-not $env:CARGO_TARGET_DIR) {
    $env:CARGO_TARGET_DIR = "D:\caa-target"
}
New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null

# Android: point each Rust target at the correct NDK clang wrapper.
$ndkBin = Join-Path $env:NDK_HOME "toolchains\llvm\prebuilt\windows-x86_64\bin"
$api = "24"
$env:CC_aarch64_linux_android = Join-Path $ndkBin "aarch64-linux-android$api-clang.cmd"
$env:AR_aarch64_linux_android = Join-Path $ndkBin "llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = Join-Path $ndkBin "aarch64-linux-android$api-clang.cmd"
$env:CC_armv7_linux_androideabi = Join-Path $ndkBin "armv7a-linux-androideabi$api-clang.cmd"
$env:AR_armv7_linux_androideabi = Join-Path $ndkBin "llvm-ar.exe"
$env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER = Join-Path $ndkBin "armv7a-linux-androideabi$api-clang.cmd"
$env:CC_i686_linux_android = Join-Path $ndkBin "i686-linux-android$api-clang.cmd"
$env:AR_i686_linux_android = Join-Path $ndkBin "llvm-ar.exe"
$env:CARGO_TARGET_I686_LINUX_ANDROID_LINKER = Join-Path $ndkBin "i686-linux-android$api-clang.cmd"
$env:CC_x86_64_linux_android = Join-Path $ndkBin "x86_64-linux-android$api-clang.cmd"
$env:AR_x86_64_linux_android = Join-Path $ndkBin "llvm-ar.exe"
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = Join-Path $ndkBin "x86_64-linux-android$api-clang.cmd"

$paths = @(
    (Join-Path $env:RUST_HOME "bin"),
    (Join-Path $env:MINGW_HOME "bin"),
    (Join-Path $env:ANDROID_HOME "platform-tools"),
    (Join-Path $env:ANDROID_HOME "cmdline-tools\latest\bin"),
    (Join-Path $env:JAVA_HOME "bin"),
    "C:\Program Files\Git\cmd"
) | Where-Object { Test-Path $_ }

$env:PATH = (($paths -join ";") + ";" + $env:PATH)

Write-Output "Rust:    $((& (Join-Path $env:RUST_HOME 'bin\rustc.exe') --version))"
Write-Output "Cargo:   $((& (Join-Path $env:RUST_HOME 'bin\cargo.exe') --version))"
Write-Output "MinGW:   $((& (Join-Path $env:MINGW_HOME 'bin\gcc.exe') -dumpversion))"
Write-Output "Android: $env:ANDROID_HOME"
Write-Output "NDK:     $env:NDK_HOME"
Write-Output "Java:    $env:JAVA_HOME"

# Cargo does not honour the Windows proxy settings, and crate downloads from
# static.crates.io are ~180x slower without one on this network. If a local
# proxy is listening, use it for HTTP(S) clients that respect these variables.
$proxyPort = 7890
$proxyListening = (Get-NetTCPConnection -State Listen -LocalPort $proxyPort -ErrorAction SilentlyContinue | Measure-Object).Count -gt 0
if ($proxyListening -and -not $env:HTTPS_PROXY) {
    $env:HTTP_PROXY = "http://127.0.0.1:$proxyPort"
    $env:HTTPS_PROXY = "http://127.0.0.1:$proxyPort"
    Write-Output "Proxy:   http://127.0.0.1:$proxyPort (local)"
} else {
    Write-Output "Proxy:   $($env:HTTPS_PROXY)"
}

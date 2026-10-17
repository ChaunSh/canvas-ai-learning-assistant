# Full local verification gate.
#
# Runs everything that must pass before calling a build "good": type checking,
# frontend tests, the Rust test suite, a warnings check, and a production bundle.
# Exits non-zero on the first failure.
#
#   powershell -File scripts/verify.ps1
#   powershell -File scripts/verify.ps1 -SkipBundle

param(
    [switch]$SkipBundle
)

$ErrorActionPreference = "Continue"

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Step($name) {
    Write-Host ""
    Write-Host "=== $name ===" -ForegroundColor Cyan
}

$failures = @()

function Invoke-Gate($name, $command, $logName) {
    Step $name
    $log = Join-Path $env:TEMP $logName
    cmd /c "$command 1> `"$log`" 2>&1"
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Write-Host "FAILED ($code): $command" -ForegroundColor Red
        Get-Content $log -Tail 25 | Write-Host
        $script:failures += $name
        return $false
    }
    # Surface a short summary line so the run is readable without opening logs.
    Get-Content $log | Where-Object { $_ -match "passed|built in|Compiling|Finished" } |
        Select-Object -Last 2 | ForEach-Object { Write-Host "    $($_ -replace '\x1b\[[0-9;]*m','')" }
    Write-Host "    OK" -ForegroundColor Green
    return $true
}

# Load the toolchain environment once (Rust, MinGW, NDK, proxy).
. (Join-Path $repo "scripts\env.ps1") | Out-Null

Invoke-Gate "TypeScript" "npx tsc --noEmit -p tsconfig.json" "verify-tsc.log" | Out-Null
Invoke-Gate "Frontend tests" "npx vitest run" "verify-vitest.log" | Out-Null
Invoke-Gate "Rust tests" "cd src-tauri && cargo test" "verify-cargo-test.log" | Out-Null
Invoke-Gate "Rust warnings" "cd src-tauri && cargo check --all-targets" "verify-cargo-check.log" | Out-Null

if (-not $SkipBundle) {
    Invoke-Gate "Production bundle" "npm run build:web" "verify-build.log" | Out-Null
}

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "ALL GATES PASSED" -ForegroundColor Green
    exit 0
}

Write-Host "FAILED GATES: $($failures -join ', ')" -ForegroundColor Red
exit 1

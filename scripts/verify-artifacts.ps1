# Acceptance checks for the produced artifacts.
#
# Verifies that what we ship is what we claim: the Windows installer installs,
# launches and uninstalls cleanly, and the APK really contains both native ABIs
# plus the application classes. Read-only with respect to source; it does install
# and remove the app for the current user.
#
#   powershell -File scripts/verify-artifacts.ps1
#   powershell -File scripts/verify-artifacts.ps1 -SkipInstall

param(
    [string]$Installer = "artifacts\Canvas AI Learning Assistant_0.1.0_x64-setup.exe",
    [string]$Apk = "artifacts\Canvas-AI-Learning-Assistant-0.1.0-arm64-x86_64.apk",
    [switch]$SkipInstall
)

$ErrorActionPreference = "Continue"

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
Add-Type -AssemblyName System.IO.Compression.FileSystem

$failures = @()

function Check($name, $condition, $detail) {
    if ($condition) {
        Write-Host "  PASS  $name" -ForegroundColor Green
    } else {
        Write-Host "  FAIL  $name — $detail" -ForegroundColor Red
        $script:failures += $name
    }
}

Write-Host "=== Windows installer ===" -ForegroundColor Cyan
if (-not (Test-Path $Installer)) {
    Check "installer exists" $false "$Installer not found (run 'pnpm tauri:build')"
} else {
    $info = Get-Item $Installer
    Check "installer exists" $true ""
    Check "installer is a plausible size" ($info.Length -gt 3MB -and $info.Length -lt 80MB) "$([math]::Round($info.Length/1MB,2)) MiB"

    if (-not $SkipInstall) {
        $installDir = Join-Path $env:LOCALAPPDATA "Canvas AI Learning Assistant"
        $setup = Start-Process -FilePath $info.FullName -ArgumentList "/S" -PassThru -Wait
        Start-Sleep -Seconds 2
        Check "silent install exit code 0" ($setup.ExitCode -eq 0) "exit $($setup.ExitCode)"
        Check "app binary installed" (Test-Path (Join-Path $installDir "canvas-ai-assistant.exe")) "missing exe"
        Check "WebView2 loader shipped" (Test-Path (Join-Path $installDir "WebView2Loader.dll")) "missing WebView2Loader.dll"

        $exe = Join-Path $installDir "canvas-ai-assistant.exe"
        if (Test-Path $exe) {
            $app = Start-Process -FilePath $exe -PassThru
            Start-Sleep -Seconds 8
            $alive = $app -and -not $app.HasExited
            Check "installed app stays running" $alive "process exited"
            if ($alive) { Stop-Process -Id $app.Id -Force; Start-Sleep -Seconds 1 }

            $dataDir = Join-Path $env:APPDATA "com.hkustgz.canvasai"
            Check "database created on first run" (Test-Path (Join-Path $dataDir "canvas-ai.db")) "no db in $dataDir"
        }

        $uninstaller = Join-Path $installDir "uninstall.exe"
        if (Test-Path $uninstaller) {
            # NSIS relaunches itself from %TEMP% and the launcher can exit before
            # the deletion finishes, so poll instead of assuming a fixed delay.
            Get-Process canvas-ai-assistant -ErrorAction SilentlyContinue | Stop-Process -Force
            $un = Start-Process -FilePath $uninstaller -ArgumentList "/S" -PassThru -Wait
            Check "silent uninstall exit code 0" ($un.ExitCode -eq 0) "exit $($un.ExitCode)"

            $removed = $false
            for ($i = 0; $i -lt 30; $i++) {
                if (-not (Test-Path $installDir)) { $removed = $true; break }
                Start-Sleep -Milliseconds 500
            }
            Check "install directory removed" $removed "still present after 15s"
            $dataDir = Join-Path $env:APPDATA "com.hkustgz.canvasai"
            Check "user data preserved by uninstall" (Test-Path (Join-Path $dataDir "canvas-ai.db")) "data was deleted"
        }
    }
}

Write-Host ""
Write-Host "=== Android APK ===" -ForegroundColor Cyan
if (-not (Test-Path $Apk)) {
    Check "apk exists" $false "$Apk not found (run 'scripts/build-android.ps1 -Release')"
} else {
    $info = Get-Item $Apk
    Check "apk exists" $true ""
    Check "apk size is sane" ($info.Length -gt 10MB -and $info.Length -lt 120MB) "$([math]::Round($info.Length/1MB,2)) MiB"

    $zip = [IO.Compression.ZipFile]::OpenRead($info.FullName)
    try {
        $names = $zip.Entries | ForEach-Object { $_.FullName }
        Check "arm64-v8a native library" ($names -contains "lib/arm64-v8a/libcanvas_ai_assistant_lib.so") "missing arm64 lib"
        Check "x86_64 native library" ($names -contains "lib/x86_64/libcanvas_ai_assistant_lib.so") "missing x86_64 lib"
        Check "application dex present" (($names | Where-Object { $_ -match "^classes.*\.dex$" }).Count -gt 0) "no classes.dex"
        Check "manifest present" ($names -contains "AndroidManifest.xml") "no manifest"

        $lib = $zip.Entries | Where-Object { $_.FullName -eq "lib/arm64-v8a/libcanvas_ai_assistant_lib.so" }
        if ($lib) {
            Check "native library is not a stub" ($lib.Length -gt 1MB) "$([math]::Round($lib.Length/1MB,2)) MiB"
        }
    }
    finally {
        $zip.Dispose()
    }

    # Signature: apksigner lives in build-tools; skip silently if unavailable.
    $bt = Join-Path $env:LOCALAPPDATA "Android\Sdk\build-tools"
    $apksigner = Get-ChildItem $bt -Recurse -Filter "apksigner.bat" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1
    if ($apksigner) {
        $verifyLog = Join-Path $env:TEMP "apksigner-verify.log"
        cmd /c "`"$($apksigner.FullName)`" verify --print-certs `"$($info.FullName)`" 1> `"$verifyLog`" 2>&1"
        $signed = $LASTEXITCODE -eq 0
        Check "apk signature verifies" $signed "apksigner failed; see $verifyLog"
        if ($signed) {
            Get-Content $verifyLog | Where-Object { $_ -match "SHA-256 digest" } | Select-Object -First 1 |
                ForEach-Object { Write-Host "        $_" }
        }
    } else {
        Write-Host "  SKIP  apk signature (apksigner not found)" -ForegroundColor Yellow
    }
}

Write-Host ""
if ($failures.Count -eq 0) {
    Write-Host "ARTIFACTS OK" -ForegroundColor Green
    exit 0
}
Write-Host "FAILED CHECKS: $($failures -join ', ')" -ForegroundColor Red
exit 1

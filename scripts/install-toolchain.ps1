# Install the Rust toolchain directly from the official distribution tarballs.
#
# Why not rustup? On this machine rustup's parallel component downloader stalled
# repeatedly while a single HTTPS stream to static.rust-lang.org reached ~4.7 MB/s.
# The official tarballs are the same artifacts rustup installs, so assembling
# them by hand produces an equivalent toolchain sysroot.
#
# Usage:  pwsh -File scripts/install-toolchain.ps1 [-Root D:\toolchain]

param(
    [string]$Root = "D:\toolchain",
    [string]$Channel = "stable",
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$HostTriple = "x86_64-pc-windows-gnu"
$AndroidTargets = @(
    "aarch64-linux-android",
    "armv7-linux-androideabi",
    "i686-linux-android",
    "x86_64-linux-android"
)

$RustRoot = Join-Path $Root "rust"
$DownloadDir = Join-Path $Root "downloads"
New-Item -ItemType Directory -Force -Path $RustRoot, $DownloadDir | Out-Null

function Get-Manifest {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $resp = Invoke-WebRequest "https://static.rust-lang.org/dist/channel-rust-$Channel.toml" -UseBasicParsing -TimeoutSec 60
    $text = $resp.Content
    if ($text -is [byte[]]) { $text = [Text.Encoding]::UTF8.GetString($text) }
    return $text
}

function Get-Package {
    param($Manifest, [string]$Pkg, [string]$Target, [switch]$Optional)
    $pattern = "(?s)\[pkg\." + [regex]::Escape($Pkg) + "\.target\." + [regex]::Escape($Target) + "\]\s*\n(.*?)(?=\n\[|\z)"
    $m = [regex]::Match($Manifest, $pattern)
    if (-not $m.Success) {
        if ($Optional) { return $null }
        throw "package not found in manifest: $Pkg / $Target"
    }
    $body = $m.Groups[1].Value
    if ($body -match 'available\s*=\s*false') {
        if ($Optional) { return $null }
        throw "package not available: $Pkg / $Target"
    }
    $url = [regex]::Match($body, 'xz_url\s*=\s*"([^"]+)"').Groups[1].Value
    $hash = [regex]::Match($body, 'xz_hash\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $url) {
        if ($Optional) { return $null }
        throw "no xz_url for $Pkg / $Target"
    }
    return [pscustomobject]@{ Url = $url; Hash = $hash; Name = ($Pkg + "-" + $Target) }
}

function Get-Tarball {
    param($Package)
    $file = Join-Path $DownloadDir (Split-Path $Package.Url -Leaf)
    if ((-not (Test-Path $file)) -or (Get-Item $file).Length -eq 0) {
        # Write-Host: Write-Output inside this function would corrupt the return value.
        Write-Host "  downloading $($Package.Url)"
        $tmp = "$file.partial"
        Invoke-WebRequest $Package.Url -OutFile $tmp -UseBasicParsing -TimeoutSec 1800
        Move-Item $tmp $file -Force
    }
    $actual = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($Package.Hash -and $actual -ne $Package.Hash.ToLower()) {
        throw "SHA-256 mismatch for $file`n  expected $($Package.Hash)`n  actual   $actual"
    }
    return $file
}

function Expand-Package {
    param($Package)
    $file = Get-Tarball $Package
    $tmp = Join-Path $DownloadDir ("x-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    & tar -xf $file -C $tmp
    if ($LASTEXITCODE -ne 0) { throw "tar failed for $file" }
    $outer = Get-ChildItem $tmp -Directory | Select-Object -First 1
    $inner = Get-ChildItem $outer.FullName -Directory | Where-Object { $_.Name -ne "rust-installer-version" } | Select-Object -First 1
    if (-not $inner) { $inner = $outer }
    return $inner.FullName
}

function Merge-Tree {
    param([string]$Source, [string]$Destination)
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    # robocopy exit codes 0-7 are success.
    & robocopy $Source $Destination /E /NFL /NDL /NJH /NJS /NP /R:1 /W:1 | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE): $Source -> $Destination" }
}

Write-Output "==> Rust $Channel from static.rust-lang.org"
$manifest = Get-Manifest
$version = [regex]::Match($manifest, '(?s)\[pkg\.rust\]\s*\nversion = "([^"]+)"').Groups[1].Value
$distDate = [regex]::Match($manifest, '(?m)^date = "([^"]+)"').Groups[1].Value
Write-Output "    rust $version (dist $distDate)"

$hostPackages = @(
    (Get-Package $manifest "rustc" $HostTriple),
    (Get-Package $manifest "cargo" $HostTriple),
    (Get-Package $manifest "rust-std" $HostTriple),
    (Get-Package $manifest "rustfmt" $HostTriple -Optional),
    (Get-Package $manifest "clippy-preview" $HostTriple -Optional)
) | Where-Object { $_ -ne $null }

foreach ($pkg in $hostPackages) {
    Write-Output "==> $($pkg.Name)"
    $src = Expand-Package $pkg
    Merge-Tree $src $RustRoot
    Remove-Item (Split-Path (Split-Path $src -Parent) -Parent) -Recurse -Force -ErrorAction SilentlyContinue
}

foreach ($target in $AndroidTargets) {
    $pkg = Get-Package $manifest "rust-std" $target
    Write-Output "==> $($pkg.Name) (cross std)"
    $src = Expand-Package $pkg
    Merge-Tree $src $RustRoot
    Remove-Item (Split-Path (Split-Path $src -Parent) -Parent) -Recurse -Force -ErrorAction SilentlyContinue
}

# rustup normally writes these; some tooling (cargo --version fallbacks) reads them.
$channelManifest = Join-Path $RustRoot "lib\rustlib\multirust-channel-manifest.toml"
New-Item -ItemType Directory -Force -Path (Split-Path $channelManifest) | Out-Null
[IO.File]::WriteAllText($channelManifest, $manifest, (New-Object Text.UTF8Encoding($false)))

Write-Output ""
Write-Output "==> Installed toolchain at $RustRoot"
& (Join-Path $RustRoot "bin\rustc.exe") --version
& (Join-Path $RustRoot "bin\cargo.exe") --version
Write-Output ""
Write-Output "==> Installed targets:"
Get-ChildItem (Join-Path $RustRoot "lib\rustlib") -Directory | Select-Object -ExpandProperty Name

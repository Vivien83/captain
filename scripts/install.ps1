# Captain installer for Windows
# Usage: iwr -useb https://captain.sh/install.ps1 | iex
#   or:  powershell -c "irm https://captain.sh/install.ps1 | iex"
#
# Flags (via environment variables):
#   $env:CAPTAIN_INSTALL_DIR = custom install directory
#   $env:CAPTAIN_VERSION     = specific version tag (e.g. "v0.1.0")
#   $env:CAPTAIN_DIST_BASE_URL = optional controlled release mirror using dist/releases layout
#   $env:CAPTAIN_GITHUB_REPO = GitHub repo for release assets (default: Vivien83/captain)
#   $env:CAPTAIN_GITHUB_TOKEN = optional token for private GitHub release downloads
#   $env:CAPTAIN_BUNDLE_PATH = install from a local precompiled .zip bundle
#   $env:CAPTAIN_BUNDLE_SHA256 = expected sha256 for CAPTAIN_BUNDLE_PATH
#   $env:CAPTAIN_PROFILE = core | vps | desktop | full-media (default: core)
#   $env:CAPTAIN_SETUP = ask | 1 | 0 (default: ask)
#   $env:CAPTAIN_SETUP_QUICK = 1 to run non-interactive setup from env vars
#   $env:CAPTAIN_MEMPALACE_INSTALL = 0 to explicitly skip managed MemPalace

$ErrorActionPreference = 'Stop'

$DistBaseUrl = if ($env:CAPTAIN_DIST_BASE_URL) { $env:CAPTAIN_DIST_BASE_URL.TrimEnd('/') } else { "" }
$GithubRepo = if ($env:CAPTAIN_GITHUB_REPO) { $env:CAPTAIN_GITHUB_REPO } else { "Vivien83/captain" }
$GithubBaseUrl = if ($env:CAPTAIN_GITHUB_BASE_URL) { $env:CAPTAIN_GITHUB_BASE_URL.TrimEnd('/') } else { "https://github.com" }
$DefaultInstallDir = Join-Path $env:USERPROFILE ".captain\bin"
$InstallDir = if ($env:CAPTAIN_INSTALL_DIR) { $env:CAPTAIN_INSTALL_DIR } else { $DefaultInstallDir }
$Profile = if ($env:CAPTAIN_PROFILE) { $env:CAPTAIN_PROFILE } else { "core" }

function Write-Banner {
    Write-Host ""
    Write-Host "  Captain Installer" -ForegroundColor Cyan
    Write-Host "  =================" -ForegroundColor Cyan
    Write-Host ""
}

function Fail-CaptainInstall {
    param([string]$Message)
    Write-Host "  Error: $Message" -ForegroundColor Red
    exit 1
}

function Test-Yes {
    param([string]$Value)
    return $Value -match '^(1|true|yes|y)$'
}

function Test-No {
    param([string]$Value)
    return $Value -match '^(0|false|no|n)$'
}

function Run-InitialSetup {
    param([string]$CaptainExe)

    $setupMode = if ($env:CAPTAIN_SETUP) { $env:CAPTAIN_SETUP } else { "ask" }

    if (Test-Yes $env:CAPTAIN_SETUP_QUICK) {
        Write-Host ""
        Write-Host "  Running captain setup --quick..." -ForegroundColor Cyan
        & $CaptainExe setup --quick --profile $Profile --yes
        return
    }

    if ($setupMode -match '^(0|false|no|n)$') {
        return
    }

    if ($setupMode -match '^(1|true|yes|y)$') {
        Write-Host ""
        Write-Host "  Running guided setup..." -ForegroundColor Cyan
        & $CaptainExe setup --profile $Profile
        return
    }

    if ($setupMode -ne "ask" -and $setupMode -ne "") {
        Fail-CaptainInstall "Unsupported CAPTAIN_SETUP: $setupMode (expected ask, 1, or 0)"
    }

    Write-Host ""
    $answer = Read-Host "  Configure Captain now so it is ready at first launch? [Y/n]"
    if ($answer -match '^(n|no)$') {
        Write-Host "  Setup skipped. Run next: captain setup" -ForegroundColor Yellow
        return
    }
    & $CaptainExe setup --profile $Profile
}

function Get-Architecture {
    $arch = ""

    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch {}

    if (-not $arch -or $arch -eq "") {
        try { $arch = $env:PROCESSOR_ARCHITECTURE } catch {}
    }

    if (-not $arch -or $arch -eq "") {
        try {
            $wmiArch = (Get-CimInstance Win32_Processor).Architecture
            if ($wmiArch -eq 9) { $arch = "AMD64" }
            elseif ($wmiArch -eq 12) { $arch = "ARM64" }
        } catch {}
    }

    if (-not $arch -or $arch -eq "") {
        if ([IntPtr]::Size -eq 8) { $arch = "X64" }
    }

    $archUpper = "$arch".ToUpper().Trim()
    switch ($archUpper) {
        { $_ -in "X64", "AMD64", "X86_64" } { return "x86_64" }
        { $_ -in "ARM64", "AARCH64", "ARM" } { return "aarch64" }
        default {
            Fail-CaptainInstall "Unsupported architecture: $arch. Download a matching Captain bundle or contact support."
        }
    }
}

function Get-LatestVersion {
    if ($env:CAPTAIN_VERSION) {
        return $env:CAPTAIN_VERSION
    }
    if (-not $DistBaseUrl) {
        return "latest"
    }

    Write-Host "  Fetching latest release..."
    try {
        return (Invoke-WebRequest -Uri "$DistBaseUrl/latest.txt" -UseBasicParsing).Content.Trim()
    }
    catch {
        Fail-CaptainInstall "Could not determine latest version from $DistBaseUrl/latest.txt. Set CAPTAIN_VERSION or CAPTAIN_BUNDLE_PATH."
    }
}

function Invoke-CaptainDownload {
    param(
        [string]$Uri,
        [string]$OutFile
    )

    $headers = @{}
    if ($env:CAPTAIN_GITHUB_TOKEN) {
        $headers["Authorization"] = "Bearer $env:CAPTAIN_GITHUB_TOKEN"
        $headers["Accept"] = "application/octet-stream"
    }

    Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing -Headers $headers
}

function Install-Captain {
    Write-Banner

    $arch = Get-Architecture
    $version = if ($env:CAPTAIN_BUNDLE_PATH -and -not $env:CAPTAIN_VERSION) { "local-bundle" } else { Get-LatestVersion }
    $target = "${arch}-pc-windows-msvc"
    $archive = "captain-${target}.zip"
    if ($DistBaseUrl) {
        $url = "$DistBaseUrl/$version/$archive"
    }
    elseif ($version -eq "latest") {
        $url = "$GithubBaseUrl/$GithubRepo/releases/latest/download/$archive"
    }
    else {
        $url = "$GithubBaseUrl/$GithubRepo/releases/download/$version/$archive"
    }
    $checksumUrl = "$url.sha256"

    if ($env:CAPTAIN_BUNDLE_PATH) {
        $archivePath = $env:CAPTAIN_BUNDLE_PATH
        if (-not (Test-Path $archivePath)) {
            Fail-CaptainInstall "CAPTAIN_BUNDLE_PATH does not exist: $archivePath"
        }
        Write-Host "  Installing Captain $version for $target from local bundle..."
    }
    else {
        Write-Host "  Installing Captain $version for $target..."
    }

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "captain-install"
    if (Test-Path $tempDir) { Remove-Item -Recurse -Force $tempDir }
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    $checksumPath = Join-Path $tempDir "$archive.sha256"

    if (-not $env:CAPTAIN_BUNDLE_PATH) {
        $archivePath = Join-Path $tempDir $archive
        try {
            Invoke-CaptainDownload -Uri $url -OutFile $archivePath
        }
        catch {
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
            Fail-CaptainInstall "Download failed from $url. The controlled Captain distribution may not contain this platform yet."
        }
    }

    $checksumDownloaded = $false
    if ($env:CAPTAIN_BUNDLE_SHA256) {
        $expectedHash = $env:CAPTAIN_BUNDLE_SHA256.Trim().ToLower()
        $actualHash = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
        if ($expectedHash -ne $actualHash) {
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
            Fail-CaptainInstall "Checksum verification failed. Expected $expectedHash, got $actualHash"
        }
        Write-Host "  Checksum verified." -ForegroundColor Green
    }
    else {
        try {
            if ($env:CAPTAIN_BUNDLE_PATH -and (Test-Path "$archivePath.sha256")) {
                Copy-Item -Path "$archivePath.sha256" -Destination $checksumPath -Force
                $checksumDownloaded = $true
            }
            elseif (-not $env:CAPTAIN_BUNDLE_PATH) {
                Invoke-CaptainDownload -Uri $checksumUrl -OutFile $checksumPath
                $checksumDownloaded = $true
            }
        }
        catch {
            Write-Host "  Checksum file not available, skipping verification." -ForegroundColor Yellow
        }
        if ($checksumDownloaded) {
            $expectedHash = (Get-Content $checksumPath -Raw).Split(" ")[0].Trim().ToLower()
            $actualHash = (Get-FileHash $archivePath -Algorithm SHA256).Hash.ToLower()
            if ($expectedHash -ne $actualHash) {
                Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
                Fail-CaptainInstall "Checksum verification failed. Expected $expectedHash, got $actualHash"
            }
            Write-Host "  Checksum verified." -ForegroundColor Green
        }
    }

    Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force
    $exePath = Join-Path $tempDir "captain.exe"
    if (-not (Test-Path $exePath)) {
        $found = Get-ChildItem -Path $tempDir -Filter "captain.exe" -Recurse | Select-Object -First 1
        if ($found) {
            $exePath = $found.FullName
        }
        else {
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
            Fail-CaptainInstall "Release archive does not contain a captain CLI binary."
        }
    }

    $installedExe = Join-Path $InstallDir "captain.exe"
    Copy-Item -Path $exePath -Destination $installedExe -Force
    Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$currentPath", "User")
        Write-Host "  Added $InstallDir to user PATH." -ForegroundColor Green
        Write-Host "  Restart your terminal for persisted PATH changes to apply." -ForegroundColor Yellow
    }
    if ($env:Path -notlike "*$InstallDir*") {
        $env:Path = "$InstallDir;$env:Path"
    }

    if (-not (Test-Path $installedExe)) {
        Fail-CaptainInstall "Captain CLI was not installed at $installedExe"
    }
    try {
        $versionOutput = & $installedExe --version 2>&1
    }
    catch {
        Fail-CaptainInstall "Captain CLI is present but failed to run: $installedExe --version"
    }
    $resolved = Get-Command "captain.exe" -ErrorAction SilentlyContinue
    if (-not $resolved) {
        Fail-CaptainInstall "Captain CLI was installed but is not resolvable on PATH."
    }

    Write-Host ""
    Write-Host "  Captain CLI verified: $installedExe" -ForegroundColor Green
    Write-Host "  Captain installed successfully! ($versionOutput)" -ForegroundColor Green

    Run-InitialSetup $installedExe

    if (Test-No $env:CAPTAIN_MEMPALACE_INSTALL) {
        Write-Host ""
        Write-Host "  Warning: managed MemPalace installation explicitly skipped." -ForegroundColor Yellow
        Write-Host "  Durable local memory remains available, but semantic memory is degraded." -ForegroundColor Yellow
    }
    else {
        Write-Host ""
        Write-Host "  Installing managed MemPalace memory runtime..." -ForegroundColor Cyan
        & $installedExe memory install
        if ($LASTEXITCODE -ne 0) {
            Fail-CaptainInstall "Managed MemPalace installation failed. Retry: captain memory install --force"
        }
        & $installedExe memory doctor --json | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Fail-CaptainInstall "Managed MemPalace installed but failed its live semantic probe. Retry: captain memory install --force"
        }
        Write-Host "  Managed MemPalace runtime checked." -ForegroundColor Green
    }

    Write-Host ""
    Write-Host "  Get started:" -ForegroundColor Cyan
    Write-Host "    captain setup"
    Write-Host ""
    Write-Host "  The setup wizard will guide you through provider selection"
    Write-Host "  and configuration."
    Write-Host ""
}

Install-Captain

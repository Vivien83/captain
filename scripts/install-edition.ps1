# Install a lightweight Captain Console or optional Captain Node on Windows.
# The edition can be passed with -Edition or CAPTAIN_EDITION.

[CmdletBinding()]
param(
    [ValidateSet("console", "node")]
    [string]$Edition = $env:CAPTAIN_EDITION,
    [string]$Version = $env:CAPTAIN_VERSION,
    [string]$InstallDir = $env:CAPTAIN_INSTALL_DIR,
    [string]$BundlePath = $env:CAPTAIN_BUNDLE_PATH,
    [string]$BundleSha256 = $env:CAPTAIN_BUNDLE_SHA256
)

$ErrorActionPreference = "Stop"
$Repo = if ($env:CAPTAIN_GITHUB_REPO) { $env:CAPTAIN_GITHUB_REPO } else { "Vivien83/captain" }
$GithubBase = if ($env:CAPTAIN_GITHUB_BASE_URL) { $env:CAPTAIN_GITHUB_BASE_URL } else { "https://github.com" }
$DistBase = $env:CAPTAIN_DIST_BASE_URL
$Target = "x86_64-pc-windows-msvc"

function Fail-CaptainEdition([string]$Message) {
    throw "Captain lightweight install failed: $Message"
}

if (-not $Edition) {
    Fail-CaptainEdition "CAPTAIN_EDITION or -Edition must be console or node"
}
if (-not $Version) {
    $Version = "latest"
}
if (-not [Environment]::Is64BitOperatingSystem) {
    Fail-CaptainEdition "only 64-bit Windows is supported"
}

switch ($Edition) {
    "console" {
        $ArchivePrefix = "captain-console"
        $BinaryName = "captain-console.exe"
    }
    "node" {
        $ArchivePrefix = "captain-node"
        $BinaryName = "captain-node.exe"
    }
}

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "Captain\bin"
}
$ArchiveName = "$ArchivePrefix-$Target.zip"
$TempRoot = Join-Path ([IO.Path]::GetTempPath()) ("captain-edition-install-" + [Guid]::NewGuid().ToString("N"))
$Archive = Join-Path $TempRoot $ArchiveName
$Checksum = "$Archive.sha256"
$Extract = Join-Path $TempRoot "extract"

function Download-File([string]$Url, [string]$Output) {
    $headers = @{}
    if ($env:CAPTAIN_GITHUB_TOKEN) {
        $headers.Authorization = "Bearer $($env:CAPTAIN_GITHUB_TOKEN)"
    }
    Invoke-WebRequest -UseBasicParsing -Headers $headers -Uri $Url -OutFile $Output
}

function Download-GithubAsset([object]$Release, [string]$Name, [string]$Output) {
    $asset = $Release.assets | Where-Object { $_.name -eq $Name } | Select-Object -First 1
    if (-not $asset) {
        Fail-CaptainEdition "release $Version has no $Name asset"
    }
    Invoke-WebRequest -UseBasicParsing -Headers @{
        Authorization = "Bearer $($env:CAPTAIN_GITHUB_TOKEN)"
        Accept = "application/octet-stream"
    } -Uri "https://api.github.com/repos/$Repo/releases/assets/$($asset.id)" -OutFile $Output
}

function Verify-Archive([string]$Path, [string]$Sidecar) {
    $expected = $BundleSha256
    if (-not $expected) {
        if (-not (Test-Path -LiteralPath $Sidecar -PathType Leaf)) {
            Fail-CaptainEdition "a SHA-256 sidecar is required: $Sidecar"
        }
        $parts = ((Get-Content -LiteralPath $Sidecar -TotalCount 1).Trim() -split "\s+")
        if ($parts.Count -lt 2 -or $parts[1].TrimStart("*") -ne [IO.Path]::GetFileName($Path)) {
            Fail-CaptainEdition "checksum sidecar names an unexpected archive"
        }
        $expected = $parts[0]
    }
    if ($expected -notmatch "^[0-9a-fA-F]{64}$") {
        Fail-CaptainEdition "the expected SHA-256 is invalid"
    }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($actual -ne $expected) {
        Fail-CaptainEdition "bundle checksum verification failed"
    }
}

function Assert-SafeZip([string]$Path, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $root = [IO.Path]::GetFullPath($Destination + [IO.Path]::DirectorySeparatorChar)
    $zip = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        foreach ($entry in $zip.Entries) {
            if ($entry.FullName.StartsWith("/") -or $entry.FullName.StartsWith("\")) {
                Fail-CaptainEdition "bundle contains an absolute archive path"
            }
            $destinationPath = [IO.Path]::GetFullPath((Join-Path $Destination $entry.FullName))
            if (-not $destinationPath.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                Fail-CaptainEdition "bundle contains a path outside its extraction root"
            }
        }
    }
    finally {
        $zip.Dispose()
    }
}

try {
    New-Item -ItemType Directory -Force -Path $TempRoot, $Extract | Out-Null
    Write-Host ""
    Write-Host "  Captain $Edition installer"
    Write-Host "  ========================="
    Write-Host "  Version:  $Version"
    Write-Host "  Platform: $Target"
    Write-Host ""

    if ($BundlePath) {
        if (-not (Test-Path -LiteralPath $BundlePath -PathType Leaf)) {
            Fail-CaptainEdition "CAPTAIN_BUNDLE_PATH does not exist"
        }
        $Archive = (Resolve-Path -LiteralPath $BundlePath).Path
        $Checksum = "$Archive.sha256"
    }
    elseif ($DistBase) {
        if ($Version -eq "latest") {
            Fail-CaptainEdition "CAPTAIN_VERSION is required with CAPTAIN_DIST_BASE_URL"
        }
        Download-File "$DistBase/$Version/$ArchiveName" $Archive
        Download-File "$DistBase/$Version/$ArchiveName.sha256" $Checksum
    }
    elseif ($env:CAPTAIN_GITHUB_TOKEN) {
        $releaseUri = if ($Version -eq "latest") {
            "https://api.github.com/repos/$Repo/releases/latest"
        }
        else {
            "https://api.github.com/repos/$Repo/releases/tags/$Version"
        }
        $release = Invoke-RestMethod -Headers @{
            Authorization = "Bearer $($env:CAPTAIN_GITHUB_TOKEN)"
            Accept = "application/vnd.github+json"
        } -Uri $releaseUri
        if ($release.tag_name) {
            $Version = $release.tag_name
        }
        Download-GithubAsset $release $ArchiveName $Archive
        Download-GithubAsset $release "$ArchiveName.sha256" $Checksum
    }
    else {
        $releaseBase = if ($Version -eq "latest") {
            "$GithubBase/$Repo/releases/latest/download"
        }
        else {
            "$GithubBase/$Repo/releases/download/$Version"
        }
        Download-File "$releaseBase/$ArchiveName" $Archive
        Download-File "$releaseBase/$ArchiveName.sha256" $Checksum
    }

    Verify-Archive $Archive $Checksum
    Assert-SafeZip $Archive $Extract
    Expand-Archive -LiteralPath $Archive -DestinationPath $Extract -Force
    $Root = Join-Path $Extract "$ArchivePrefix-$Target"
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
        $Root = $Extract
    }
    $SourceBinary = Join-Path $Root $BinaryName
    $VersionMarker = Join-Path $Root "VERSION"
    if (-not (Test-Path -LiteralPath $SourceBinary -PathType Leaf)) {
        Fail-CaptainEdition "bundle does not contain $BinaryName"
    }
    if (-not (Test-Path -LiteralPath $VersionMarker -PathType Leaf)) {
        Fail-CaptainEdition "bundle VERSION marker is missing"
    }
    $BundleVersion = (Get-Content -LiteralPath $VersionMarker -Raw).Trim()
    if ($Version -eq "latest") {
        $Version = $BundleVersion
    }
    elseif ($BundleVersion -ne $Version) {
        Fail-CaptainEdition "bundle version does not match $Version"
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $Destination = Join-Path $InstallDir $BinaryName
    $Previous = "$Destination.previous"
    $Temporary = "$Destination.$([Guid]::NewGuid().ToString('N')).tmp"
    Copy-Item -LiteralPath $SourceBinary -Destination $Temporary
    Remove-Item -LiteralPath $Previous -Force -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $Destination) {
        Move-Item -LiteralPath $Destination -Destination $Previous
    }
    try {
        Move-Item -LiteralPath $Temporary -Destination $Destination
        $ProbeOutput = (& $Destination --version | Out-String).Trim()
        $CanonicalVersion = $Version -replace "^[vV](?=\d)", ""
        $ExpectedOutput = "$($BinaryName.Substring(0, $BinaryName.Length - 4)) $CanonicalVersion"
        if ($LASTEXITCODE -ne 0 -or $ProbeOutput -ne $ExpectedOutput) {
            throw "exact version probe failed"
        }
    }
    catch {
        Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $Previous) {
            Move-Item -LiteralPath $Previous -Destination $Destination
        }
        Fail-CaptainEdition "$BinaryName failed its post-install probe; previous binary restored"
    }
    Set-Content -LiteralPath (Join-Path $InstallDir "VERSION") -Value $Version -Encoding ASCII

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = @($userPath -split ";" | Where-Object { $_ })
    if ($env:CAPTAIN_UPDATE_PATH -notmatch "^(0|false|no)$" -and $pathParts -notcontains $InstallDir) {
        [Environment]::SetEnvironmentVariable("Path", (($pathParts + $InstallDir) -join ";"), "User")
        Write-Host "  Added $InstallDir to the user PATH."
    }

    Write-Host ""
    Write-Host "  Installed: $Destination" -ForegroundColor Green
    & $Destination --version
    if ($Edition -eq "console") {
        Write-Host "  Next: captain-console pair --hub https://your-captain.example"
    }
    else {
        Write-Host "  Next: captain-node pair --hub https://your-captain.example --workspace C:\path"
        Write-Host "        captain-node service install"
    }
    Write-Host ""
}
finally {
    Remove-Item -LiteralPath $TempRoot -Recurse -Force -ErrorAction SilentlyContinue
}

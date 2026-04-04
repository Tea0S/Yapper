# Writes updater static manifest latest.json after `tauri build`.
# Emits Tauri v2 platform keys so MSI and NSIS installs each get the matching URL + signature:
#   windows-x86_64-msi, windows-x86_64-nsis, and windows-x86_64 (fallback / dev).
# Reads version + GitHub owner/repo from src-tauri/tauri.conf.json (plugins.updater.endpoints).
# Overrides: $env:YAPPER_INSTALLER_DOWNLOAD_URL (NSIS .exe), $env:YAPPER_MSI_INSTALLER_DOWNLOAD_URL (.msi)
# Release tag: $env:YAPPER_RELEASE_TAG when it differs from tauri.conf version
param(
    [ValidateSet("release", "debug")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$TauriDir = Join-Path $RepoRoot "src-tauri"
$ConfPath = Join-Path $TauriDir "tauri.conf.json"
$NsisDir = Join-Path $TauriDir "target\$Profile\bundle\nsis"
$MsiDir = Join-Path $TauriDir "target\$Profile\bundle\msi"

if (-not (Test-Path -LiteralPath $ConfPath)) {
    Write-Error "Missing $ConfPath"
}

$conf = Get-Content -LiteralPath $ConfPath -Raw | ConvertFrom-Json
$version = $conf.version
if (-not $version) {
    Write-Error "tauri.conf.json missing version"
}

if (-not (Test-Path -LiteralPath $NsisDir)) {
    Write-Error "NSIS bundle folder not found: $NsisDir (run tauri build first)"
}

$exe = Get-ChildItem -LiteralPath $NsisDir -Filter "Yapper_${version}_x64-setup.exe" -File -ErrorAction SilentlyContinue
if (-not $exe) {
    $exe = Get-ChildItem -LiteralPath $NsisDir -Filter "*_x64-setup.exe" -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}
if (-not $exe) {
    Write-Error "No *_x64-setup.exe under $NsisDir"
}

$nsisSigPath = "$($exe.FullName).sig"
if (-not (Test-Path -LiteralPath $nsisSigPath)) {
    Write-Error "Missing signature file: $nsisSigPath (createUpdaterArtifacts + signing key required)"
}
$nsisSignature = [System.IO.File]::ReadAllText($nsisSigPath)

$msi = $null
$msiSignature = $null
if (Test-Path -LiteralPath $MsiDir) {
    $msi = Get-ChildItem -LiteralPath $MsiDir -Filter "Yapper_${version}_x64*.msi" -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $msi) {
        $msi = Get-ChildItem -LiteralPath $MsiDir -Filter "*.msi" -File |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
    }
    if ($msi) {
        $msiSigPath = "$($msi.FullName).sig"
        if (-not (Test-Path -LiteralPath $msiSigPath)) {
            Write-Error "Missing signature file: $msiSigPath (createUpdaterArtifacts + signing key required)"
        }
        $msiSignature = [System.IO.File]::ReadAllText($msiSigPath)
    }
}

$endpoint = $conf.plugins.updater.endpoints[0]
if (-not $endpoint) {
    Write-Error "No plugins.updater.endpoints[0] in tauri.conf.json"
}
if ($endpoint -notmatch "github\.com/([^/]+)/([^/]+)") {
    Write-Error "Could not parse owner/repo from endpoint: $endpoint. Set YAPPER_INSTALLER_DOWNLOAD_URL / YAPPER_MSI_INSTALLER_DOWNLOAD_URL."
}
$owner = $Matches[1]
$repo = $Matches[2]
$tag = if ($env:YAPPER_RELEASE_TAG) { $env:YAPPER_RELEASE_TAG } else { $version }

$nsisUrl = if ($env:YAPPER_INSTALLER_DOWNLOAD_URL) { $env:YAPPER_INSTALLER_DOWNLOAD_URL } else {
    "https://github.com/$owner/$repo/releases/download/$tag/$($exe.Name)"
}
$msiUrl = $null
if ($msi) {
    $msiUrl = if ($env:YAPPER_MSI_INSTALLER_DOWNLOAD_URL) { $env:YAPPER_MSI_INSTALLER_DOWNLOAD_URL } else {
        "https://github.com/$owner/$repo/releases/download/$tag/$($msi.Name)"
    }
}

$pubDate = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ss\Z")

$platforms = [ordered]@{}
if ($msi -and $msiSignature -and $msiUrl) {
    $platforms["windows-x86_64-msi"] = [ordered]@{
        signature = $msiSignature
        url       = $msiUrl
    }
}
$platforms["windows-x86_64-nsis"] = [ordered]@{
    signature = $nsisSignature
    url       = $nsisUrl
}
# Fallback when bundle type is unknown (e.g. some dev runs); match NSIS as default download.
$platforms["windows-x86_64"] = [ordered]@{
    signature = $nsisSignature
    url       = $nsisUrl
}

$manifest = [ordered]@{
    version   = $version
    notes     = ""
    pub_date  = $pubDate
    platforms = $platforms
}

$json = ($manifest | ConvertTo-Json -Depth 6 -Compress)
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$outNsis = Join-Path $NsisDir "latest.json"
[System.IO.File]::WriteAllText($outNsis, $json + "`n", $utf8NoBom)
Write-Host "Wrote $outNsis" -ForegroundColor Green

if ($msi) {
    $outMsi = Join-Path $MsiDir "latest.json"
    [System.IO.File]::WriteAllText($outMsi, $json + "`n", $utf8NoBom)
    Write-Host "Wrote $outMsi" -ForegroundColor Green
}

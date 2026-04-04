# Writes updater static manifest latest.json next to the NSIS bundle after `tauri build`.
# Reads version + GitHub owner/repo from src-tauri/tauri.conf.json (plugins.updater.endpoints).
# Override installer URL: $env:YAPPER_INSTALLER_DOWNLOAD_URL = 'https://.../file.exe'
param(
    [ValidateSet("release", "debug")]
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$TauriDir = Join-Path $RepoRoot "src-tauri"
$ConfPath = Join-Path $TauriDir "tauri.conf.json"
$NsisDir = Join-Path $TauriDir "target\$Profile\bundle\nsis"

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

$sigPath = "$($exe.FullName).sig"
if (-not (Test-Path -LiteralPath $sigPath)) {
    Write-Error "Missing signature file: $sigPath (createUpdaterArtifacts + signing key required)"
}

$signature = [System.IO.File]::ReadAllText($sigPath)

$installerUrl = $env:YAPPER_INSTALLER_DOWNLOAD_URL
if (-not $installerUrl) {
    $endpoint = $conf.plugins.updater.endpoints[0]
    if (-not $endpoint) {
        Write-Error "No plugins.updater.endpoints[0] in tauri.conf.json; set YAPPER_INSTALLER_DOWNLOAD_URL"
    }
    if ($endpoint -notmatch "github\.com/([^/]+)/([^/]+)") {
        Write-Error "Could not parse owner/repo from endpoint: $endpoint. Set YAPPER_INSTALLER_DOWNLOAD_URL."
    }
    $owner = $Matches[1]
    $repo = $Matches[2]
    $tag = if ($version.StartsWith("v")) { $version } else { "v$version" }
    $installerUrl = "https://github.com/$owner/$repo/releases/download/$tag/$($exe.Name)"
}

$pubDate = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ss\Z")

$manifest = [ordered]@{
    version   = $version
    notes     = ""
    pub_date  = $pubDate
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $signature
            url       = $installerUrl
        }
    }
}

$json = ($manifest | ConvertTo-Json -Depth 5 -Compress)
$outPath = Join-Path $NsisDir "latest.json"
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($outPath, $json + "`n", $utf8NoBom)
Write-Host "Wrote $outPath" -ForegroundColor Green

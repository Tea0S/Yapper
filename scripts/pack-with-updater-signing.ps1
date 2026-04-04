# Loads minisign private key into TAURI_SIGNING_PRIVATE_KEY (required when createUpdaterArtifacts is true).
# TAURI_SIGNING_PRIVATE_KEY_PATH alone is not always honored by the bundler; key material must be in env.
# Usage:
#   .\scripts\pack-with-updater-signing.ps1              -> npx tauri build
#   .\scripts\pack-with-updater-signing.ps1 -Release   -> bundle:python + tauri build
#   .\scripts\pack-with-updater-signing.ps1 -Debug     -> npx tauri build --debug
param(
    [switch]$Release,
    [switch]$Debug
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path $PSScriptRoot -Parent
$KeyPath = Join-Path $RepoRoot "src-tauri\.tauri\updater.key"

if (-not (Test-Path -LiteralPath $KeyPath)) {
    Write-Host "No updater private key at:" -ForegroundColor Yellow
    Write-Host "  $KeyPath" -ForegroundColor Yellow
    Write-Host "Generate one (prints pubkey for tauri.conf.json):" -ForegroundColor Yellow
    Write-Host '  $env:CI = "true"; npx tauri signer generate -w src-tauri/.tauri/updater.key' -ForegroundColor Cyan
    exit 1
}

$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -LiteralPath $KeyPath -Raw
# Some tooling also reads PATH; harmless to set both.
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = (Resolve-Path -LiteralPath $KeyPath).Path

Set-Location $RepoRoot

if ($Release) {
    npm run bundle:python
}

$npxCli = @(
    "tauri", "build",
    "-c", (Join-Path $RepoRoot "src-tauri/tauri.updater-release.conf.json")
)
if ($Debug) {
    $npxCli += "--debug"
}
npx @npxCli

$profile = if ($Debug) { "debug" } else { "release" }
& (Join-Path $PSScriptRoot "generate-latest-json.ps1") -Profile $profile

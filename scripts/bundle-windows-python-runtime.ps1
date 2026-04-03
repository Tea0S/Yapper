#Requires -Version 5.1
<#
  Downloads Windows embeddable CPython, bootstraps pip, and installs Yapper Python deps into
  src-tauri/resources/python-runtime/ so release builds work without a system Python.

  Run from repo root:  npm run bundle:python
  Or:                  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/bundle-windows-python-runtime.ps1
#>
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$ReqFile = Join-Path $Root "scripts\python-runtime-requirements.txt"
$Dest = Join-Path $Root "src-tauri\resources\python-runtime"

if (-not (Test-Path $ReqFile)) {
    throw "Missing requirements file: $ReqFile"
}

$PyTag = "3.12.8"
$ZipName = "python-$PyTag-embed-amd64.zip"
$ZipUrl = "https://www.python.org/ftp/python/$PyTag/$ZipName"

Write-Host "Bundling embeddable Python $PyTag -> $Dest"

if (Test-Path $Dest) {
    Remove-Item $Dest -Recurse -Force
}
New-Item -ItemType Directory -Path $Dest -Force | Out-Null

$ZipPath = Join-Path ([System.IO.Path]::GetTempPath()) $ZipName
Invoke-WebRequest -Uri $ZipUrl -OutFile $ZipPath -UseBasicParsing
try {
    Expand-Archive -Path $ZipPath -DestinationPath $Dest -Force
} finally {
    Remove-Item $ZipPath -Force -ErrorAction SilentlyContinue
}

$zipFile = Get-ChildItem -LiteralPath $Dest -Filter "python*.zip" | Select-Object -First 1
if (-not $zipFile) {
    throw "Expected python*.zip in embeddable layout"
}
$zipLine = $zipFile.Name

$pthFile = Get-ChildItem -LiteralPath $Dest -Filter "python*._pth" | Select-Object -First 1
if (-not $pthFile) {
    throw "Expected python*._pth in embeddable layout"
}

$sitePackages = Join-Path $Dest "Lib\site-packages"
New-Item -ItemType Directory -Path $sitePackages -Force | Out-Null

$pthContent = @(
    $zipLine
    "."
    "Lib\site-packages"
    ""
    "import site"
) -join "`r`n"
Set-Content -LiteralPath $pthFile.FullName -Value $pthContent -Encoding ascii

$py = Join-Path $Dest "python.exe"
$getPip = Join-Path ([System.IO.Path]::GetTempPath()) "get-pip-yapper-tmp.py"
Invoke-WebRequest -Uri "https://bootstrap.pypa.io/get-pip.py" -OutFile $getPip -UseBasicParsing
try {
    & $py $getPip --no-warn-script-location
    if ($LASTEXITCODE -ne 0) {
        throw "get-pip failed with exit $LASTEXITCODE"
    }
} finally {
    Remove-Item $getPip -Force -ErrorAction SilentlyContinue
}

& $py -m pip install --upgrade pip setuptools wheel --no-warn-script-location
if ($LASTEXITCODE -ne 0) {
    throw "pip self-upgrade failed with exit $LASTEXITCODE"
}

& $py -m pip install --no-warn-script-location -r $ReqFile
if ($LASTEXITCODE -ne 0) {
    throw "pip install -r failed with exit $LASTEXITCODE"
}

Write-Host "Done. Bundled runtime ready for tauri build."

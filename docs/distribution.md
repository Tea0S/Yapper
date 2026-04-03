# Distribution: easy download & install

Goal: a **single downloadable installer** so end users never install Python or run `pip` themselves.

## Release build (`npm run pack:release`)

1. **`npm run bundle:python`** — PowerShell script downloads Windows **embeddable CPython** (see `scripts/bundle-windows-python-runtime.ps1`), bootstraps **pip**, and installs merged deps from `scripts/python-runtime-requirements.txt` (sidecar + Yapper Node) into **`src-tauri/resources/python-runtime/`** (gitignored, not committed).
2. **`npm run pack`** — `tauri build` bundles that folder via `resources/**/*` and produces installers under:

`src-tauri/target/release/bundle/`

Typical outputs:

- **NSIS** — `Yapper_*_x64-setup.exe` (wizard installer; good default for “download from website”).
- **MSI** — `.msi` (IT / silent deploy / Intune-friendly).

Install scope defaults to **current user** (no admin) in `tauri.conf.json` unless you change `bundle.windows.nsis.installMode`.

At runtime the app prefers **`resource_dir()/python-runtime/python.exe`** (see `paths::bundled_python_exe`); override with **`YAPPER_PYTHON`** if needed.

**Quick dev build without embedding Python:** `npm run pack` only, and use system Python + `pip install -r sidecar/requirements.txt` on your machine.

## End-user story

1. User downloads `Yapper-setup.exe` from GitHub Releases or your site.
2. Installer places the app (Rust + UI + embedded Python + `sidecar/` sources) and registers Start Menu / optional auto-start.
3. First dictation: Whisper **model weights** may download to the app cache under `%LOCALAPPDATA%` (large, one-time per model).

## Size and maintenance

| Topic | Notes |
|--------|--------|
| Installer size | Embeddable Python + wheels is **tens to low hundreds of MB** (faster-whisper / CTranslate2). |
| Runtime updates | Bump **`$PyTag`** in `scripts/bundle-windows-python-runtime.ps1` when you want a newer CPython security release; rebuild `pack:release`. |
| Alternatives | **PyInstaller** one-file sidecar is possible later (smaller surface, more packaging edge cases). |

## Code signing (Windows)

Unsigned installers trigger **SmartScreen** warnings. For public distribution:

- Obtain a **standard code signing** certificate (not EV required for basic trust, EV helps SmartScreen reputation faster).
- Set `bundle.windows.certificateThumbprint` (or sign post-build with `signtool`) and use a timestamp server.

## Hosting

- **GitHub Releases** — attach `*-setup.exe` + `.msi` + `latest.json` if you add the updater.
- **winget** — publish a manifest pointing at your release URLs.
- **Microsoft Store** — possible later via MSIX path (separate packaging effort).

## CI (optional)

On **`windows-latest`**: install Rust + Node + MSVC, then **`npm ci && npm run pack:release`** (not plain `tauri build`, unless you intentionally skip the embedded runtime). Upload `bundle/` artifacts. Keep signing secrets in encrypted variables.

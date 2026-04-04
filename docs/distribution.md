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

If you see **`Unable to open file 'model.bin'`**, the Hugging Face download was interrupted or the cache is partial. The sidecar will try to delete that model’s cache folder and re-download once; if it still fails, remove `%LOCALAPPDATA%\com.yapper.app\models\models--Systran--faster-whisper-*` (or the matching repo folder) and start the engine again. Keep disk space free and stable network for large models (e.g. **large-v3**).

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

- **GitHub Releases** — attach `*-setup.exe` + `.msi`. With the built-in updater (`tauri-plugin-updater`), also attach per release:
  - **`latest.json`** at `releases/latest/download/latest.json` (static JSON listing `version`, optional `notes` / `pub_date`, and `platforms["windows-x86_64"].url` + `.signature` from the `.sig` file next to the NSIS installer).
  - Keep **`plugins.updater.endpoints`** in `src-tauri/tauri.conf.json` pointed at that URL (replace `yourusername/yapper` with your org/repo).
  - Sign builds with the minisign key: local file via `TAURI_SIGNING_PRIVATE_KEY_PATH`, or paste the key into `TAURI_SIGNING_PRIVATE_KEY` in CI. The **public** key in `tauri.conf.json` must match the private key used to sign. See [Tauri updater](https://v2.tauri.app/plugin/updater/).
- **winget** — publish a manifest pointing at your release URLs.
- **Microsoft Store** — possible later via MSIX path (separate packaging effort).

## CI (optional)

On **`windows-latest`**: install Rust + Node + MSVC, then **`npm ci && npm run pack:release`** (not plain `tauri build`, unless you intentionally skip the embedded runtime). Upload `bundle/` artifacts. Keep signing secrets in encrypted variables.

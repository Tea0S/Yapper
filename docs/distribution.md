# Distribution: easy download & install

Goal: a **single downloadable installer** so end users never install Python or run `pip` themselves.

## Release build (`npm run pack:release`)

1. **`npm run bundle:python`** — PowerShell script downloads Windows **embeddable CPython** (see `scripts/bundle-windows-python-runtime.ps1`), bootstraps **pip**, and installs merged deps from `scripts/python-runtime-requirements.txt` (sidecar + Yapper Node) into **`src-tauri/resources/python-runtime/`** (gitignored, not committed).
2. **`npm run pack`** (or **`npm run pack:release`** for Python + build) — `scripts/pack-with-updater-signing.ps1` loads `src-tauri/.tauri/updater.key` into **`TAURI_SIGNING_PRIVATE_KEY`**, then runs `tauri build`, which bundles via `resources/**/*` and produces installers under:

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
  - **`latest.json`** at `releases/latest/download/latest.json`. `scripts/generate-latest-json.ps1` fills **`platforms`** with **`windows-x86_64-msi`** (`.msi` + `.msi.sig`) and **`windows-x86_64-nsis`** / **`windows-x86_64`** (setup `.exe` + `.exe.sig`) so MSI and NSIS installs each download the matching signed artifact. The `signature` field is the full text of the corresponding `.sig` file. The `url` must use the **same Git tag** as the release. Set **`YAPPER_RELEASE_TAG`** if your Git tag differs from `tauri.conf.json` `version`. **Do not** re-save or Authenticode-sign the installer *after* the Tauri build if that changes bytes—the minisign signature would no longer match (sign via Tauri’s `signCommand` during bundle, or only distribute the exact files from `target/.../bundle/`).
  - Keep **`plugins.updater.endpoints`** in `src-tauri/tauri.conf.json` pointed at that URL (replace `yourusername/yapper` with your org/repo).
  - Sign builds with the minisign key: **`npm run pack`** / **`npm run pack:release`** load the gitignored file `src-tauri/.tauri/updater.key` into **`TAURI_SIGNING_PRIVATE_KEY`** (required by the bundler). In CI, set **`TAURI_SIGNING_PRIVATE_KEY`** to the full key text (or inject the file and read it into that variable). The **public** key in `tauri.conf.json` must match the private key. Escape hatch without the script: **`npm run pack:raw`** (still needs the env var if `createUpdaterArtifacts` is true). See [Tauri updater](https://v2.tauri.app/plugin/updater/).
- **winget** — publish a manifest pointing at your release URLs.
- **Microsoft Store** — possible later via MSIX path (separate packaging effort).

## CI (optional)

On **`windows-latest`**: install Rust + Node + MSVC, then **`npm ci && npm run pack:release`** after writing the minisign private key to `src-tauri/.tauri/updater.key`, or set **`TAURI_SIGNING_PRIVATE_KEY`** from a secret and use **`npm run pack:raw`** after `bundle:python`. Upload `bundle/` artifacts.

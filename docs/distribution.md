# Distribution: easy download & install

Goal: a **single downloadable installer** (or store listing) so users never touch Rust or Node.

## What you ship today (`npm run tauri build`)

Tauri produces Windows artifacts under:

`src-tauri/target/release/bundle/`

Typical outputs:

- **NSIS** — `Yapper_*_x64-setup.exe` (wizard installer; good default for “download from website”).
- **MSI** — `.msi` (IT / silent deploy / Intune-friendly).

Install scope defaults to **current user** (no admin) in `tauri.conf.json` unless you change `bundle.windows.nsis.installMode`.

The app binary and bundled **resources** are included: tone YAML plus the **`sidecar/`** Python sources (`server.py`, `requirements.txt`) so the installed app can find the script without cloning the repo. Users still need a **Python runtime** and `pip install -r …/requirements.txt` (or a venv you document) until you ship an embedded interpreter or frozen sidecar.

## End-user story (target)

1. User downloads `Yapper-setup.exe` from GitHub Releases or your site.
2. Installer places the app and registers Start Menu / optional auto-start.
3. First launch: optional bootstrap (e.g. download Whisper weights to `%LOCALAPPDATA%`, or run bundled inference).

## Roadmap to “no Python installed”

| Approach | Pros | Cons |
|----------|------|------|
| **Embeddable Python + venv** in `resources/` | Same `server.py`, reproducible | Larger download (~tens of MB+), you maintain runtime updates |
| **PyInstaller / Nuitka** one-file sidecar | Single extra `.exe`, no visible Python | Build complexity, AV false positives |
| **Remote-only default** | Tiny desktop installer; GPU on another PC | Requires your Yapper Node on LAN/VPN |

Recommended order: ship **NSIS + documented Python** for early adopters → add **embedded sidecar** → add **code signing** → add **auto-update** (`tauri-plugin-updater`).

## Code signing (Windows)

Unsigned installers trigger **SmartScreen** warnings. For public distribution:

- Obtain a **standard code signing** certificate (not EV required for basic trust, EV helps SmartScreen reputation faster).
- Set `bundle.windows.certificateThumbprint` (or sign post-build with `signtool`) and use a timestamp server.

## Hosting

- **GitHub Releases** — attach `*-setup.exe` + `.msi` + `latest.json` if you add the updater.
- **winget** — publish a manifest pointing at your release URLs.
- **Microsoft Store** — possible later via MSIX path (separate packaging effort).

## CI (optional)

Use a workflow that runs on `windows-latest`, installs Rust + Node + MSVC, runs `npm ci && npm run tauri build`, and uploads `bundle/` artifacts. Keep secrets (signing cert) in repository encrypted variables.

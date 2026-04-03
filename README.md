# Yapper

Windows-first desktop dictation: **Whisper** (via [faster-whisper](https://github.com/SYSTRAN/faster-whisper)) in a local Python sidecar, optional **self-hosted** [Yapper Node](yapper-node/main.py) on your LAN/VPN, plus dictionary/corrections and YAML **tone** presets applied in the Rust app.

## Prerequisites

- [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/) (for Tauri 2 + SvelteKit)
- **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (Desktop development with C++ / MSVC) — required to compile the Tauri/Rust backend.
- For **local dev without a bundled runtime:** Python 3.10+ on `PATH` / `py` launcher, with:

```bash
pip install -r sidecar/requirements.txt
```

Release installers that include **no separate Python install** are built with `npm run pack:release` (bundles an embeddable CPython + deps under `src-tauri/resources/python-runtime/`; folder is gitignored).

### `program not found` / `cargo metadata` failed

Tauri needs **`cargo`** on your `PATH`. Install Rust with [rustup](https://rustup.rs/) or, on Windows:

```powershell
winget install Rustlang.Rustup --accept-package-agreements --accept-source-agreements
```

Then **close and reopen** your terminal (or sign out/in) so `PATH` picks up `%USERPROFILE%\.cargo\bin`. Confirm:

```powershell
cargo --version
```

If it still fails, run `cargo` by full path once to verify the install: `& "$env:USERPROFILE\.cargo\bin\cargo.exe" --version`.

## Run (development)

```bash
npm install
npm run tauri dev
```

Set `YAPPER_SIDECAR` to an absolute path to `sidecar/server.py` if auto-detection fails.

### Port 1420 already in use

The dev server uses **localhost:1420** (see `vite.config.js` and `src-tauri/tauri.conf.json`). If a previous `npm run dev` / `tauri dev` left **Node** listening, the next run fails with `Port 1420 is already in use` even though an older **Yapper.exe** window may still be open.

1. Close extra Yapper dev windows, then free the port (Windows):

   ```powershell
   npm run kill:dev-port
   ```

2. Start again: `npm run tauri dev`.

If you change the port, set **`VITE_DEV_PORT`** and update **`build.devUrl`** in `src-tauri/tauri.conf.json` to the same value.

## Remote inference (optional)

On a GPU machine on your network:

```bash
pip install -r yapper-node/requirements.txt
pip install -r sidecar/requirements.txt
set YAPPER_TOKEN=your-secret
python yapper-node/main.py --host 0.0.0.0 --port 8765 --token %YAPPER_TOKEN%
```

In **Settings**, choose *Remote Yapper Node*, set the WebSocket URL and token. Prefer Tailscale/WireGuard; do not expose the node to the public internet without TLS and stronger auth.

Protocol: [docs/protocol.md](docs/protocol.md).

## Linux (later)

Tauri and `cpal` build on Linux; global shortcuts and paste-to-focused-window need X11/Wayland-specific follow-up. Start with `npm run tauri build` on a Linux host after installing distro Tauri prerequisites.

## Windows installer (packaged app)

```bash
npm run pack:release
```

Runs `scripts/bundle-windows-python-runtime.ps1` (one-time download: embeddable Python + `pip install` for sidecar and Yapper Node), then `tauri build`. Installers land in `src-tauri/target/release/bundle/` (typically **NSIS** `.exe` and **MSI**). NSIS defaults to **per-user** install (no admin).

For a quick build **without** embedding Python (dev machine only), use `npm run pack` and keep system Python + `pip install -r sidecar/requirements.txt`.

**End users of a `pack:release` build:** no Python install required for local dictation or in-app Yapper Node. The first Whisper model still downloads to the app cache on use (can be large). **Visual C++ Redistributable** is usually already present on Windows; install it if import errors mention missing `VCRUNTIME`.

**Still recommended for public downloads:** code signing (SmartScreen), then **GitHub Releases** / **winget**. See [docs/distribution.md](docs/distribution.md).

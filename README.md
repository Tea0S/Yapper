# Yapper

Windows-first desktop dictation: **Whisper** (via [faster-whisper](https://github.com/SYSTRAN/faster-whisper)) in a local Python sidecar, optional **self-hosted** [Yapper Node](yapper-node/main.py) on your LAN/VPN, plus dictionary/corrections and YAML **tone** presets applied in the Rust app.

## Prerequisites

- [Rust](https://rustup.rs/) and [Node.js](https://nodejs.org/) (for Tauri 2 + SvelteKit)
- **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (Desktop development with C++ / MSVC) — required to compile the Tauri/Rust backend.
- Python 3.10+ with sidecar dependencies:

```bash
pip install -r sidecar/requirements.txt
```

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
npm run pack
```

Same as `npm run tauri build`. Installers are written to `src-tauri/target/release/bundle/` (typically an **NSIS** `.exe` setup and an **MSI**). NSIS is configured for **per-user** install (no admin) by default.

**End users today:** the GUI is fully packaged; **local inference** still expects [Python](https://www.python.org/downloads/) and `pip install -r sidecar/requirements.txt` unless you ship a bundled sidecar (see below).

**Roadmap for “download and run”:** embed a Python runtime or a frozen `server.exe` next to Yapper, add **code signing** to avoid SmartScreen warnings, then host installers on **GitHub Releases** or **winget**. See [docs/distribution.md](docs/distribution.md) for a concrete checklist.

# Yapper
## What is it?
Windows-first desktop dictation: **Whisper** (via [faster-whisper](https://github.com/SYSTRAN/faster-whisper)) in a local Python sidecar, optional **self-hosted** [Yapper Node](yapper-node/main.py) on your LAN/VPN, plus dictionary/corrections and YAML **tone** presets applied in the Rust app.

[![Image](https://cdn.discordapp.com/attachments/742092148526153822/1489848243796971570/yapper_hRLig2QTeT.png?ex=69d1e8cd&is=69d0974d&hm=0a8bce6bac1a9661db4b2bf11748e16656a01845d640d64144ceff0c62af5918&animated=true)](https://cdn.discordapp.com/attachments/742092148526153822/1489848243796971570/yapper_hRLig2QTeT.png?ex=69d1e8cd&is=69d0974d&hm=0a8bce6bac1a9661db4b2bf11748e16656a01845d640d64144ceff0c62af5918&animated=true)

## Why?
Initial release of a new project called **Yapper**. This is local, ai driven dictation powered off Whisper, or Nvidia's Parakeet. While Whisper should run on CPU if you have an Nvidia GPU, your experience will be infinitely better with one!

It supports key bind dictation right into your text box or google doc, audio file transcription of recorded files, custom dictionaries and phrases all in a simple packaged interface. 

**Why?** Well simply, I rely on dictation every single day as someone with awful hands. I usually rely on my iPhone for this because for years, Apple's dictation was simply the best. But that always stopped me from being able to use it on my PC, where the only accurate options cost 100's of dollars and inaccessible. In the age of AI, you would think this would be more accessible with so many open source models, but the software that has cropped up around this space is either a paid subscription or lacks features that the older software has like dictionaries. 

I wanted to change that, at least for myself. And If I'm going to build it, I might as well release it. This is an inital version that I have been using for a few days — if you also need dictation at your PC please give it a whirl and let me know what you think!

Support the development of Yapper.

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/M4M21WSC82)

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

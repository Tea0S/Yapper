#!/usr/bin/env bash
# Loads minisign private key into TAURI_SIGNING_PRIVATE_KEY and enables createUpdaterArtifacts
# via config merge (see src-tauri/tauri.updater-release.conf.json).
#
# Usage (repo root):
#   bash scripts/pack-with-updater-signing.sh           -> npx tauri build (signed updater artifacts)
#   bash scripts/pack-with-updater-signing.sh --release -> npm run bundle:python:mac + tauri build
#   bash scripts/pack-with-updater-signing.sh --debug   -> tauri build --debug
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="$ROOT/src-tauri/.tauri/updater.key"
MERGE="$ROOT/src-tauri/tauri.updater-release.conf.json"

if [[ ! -f "$KEY" ]]; then
  echo "No updater private key at: $KEY" >&2
  echo "Generate one (prints pubkey for tauri.conf.json):" >&2
  echo '  CI=true npx tauri signer generate -w src-tauri/.tauri/updater.key' >&2
  exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
export TAURI_SIGNING_PRIVATE_KEY_PATH="$KEY"

cd "$ROOT"

RELEASE=0
DEBUG=0
for a in "$@"; do
  case "$a" in
  --release) RELEASE=1 ;;
  --debug) DEBUG=1 ;;
  esac
done

if [[ "$RELEASE" -eq 1 ]]; then
  npm run bundle:python:mac
fi

ARGS=(tauri build -c "$MERGE")
if [[ "$DEBUG" -eq 1 ]]; then
  ARGS+=(--debug)
fi
npx "${ARGS[@]}"

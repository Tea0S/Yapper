#!/usr/bin/env bash
# Apple Silicon (arm64) only. Downloads relocatable CPython (python-build-standalone),
# installs sidecar + Yapper Node deps into src-tauri/resources/python-runtime/, then you can
# `npm run tauri build` with no separate Python install.
#
# Run from repo root:  npm run bundle:python:mac
# Or:                  bash scripts/bundle-macos-python-runtime.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REQ="$ROOT/scripts/python-runtime-requirements.txt"
DEST="$ROOT/src-tauri/resources/python-runtime"

# Pin to Astral python-build-standalone release + CPython version (keep in sync with Windows script when practical).
PBS_RELEASE="${PBS_RELEASE:-20260325}"
CPYTHON_FULL="${CPYTHON_FULL:-3.12.13}"

if [[ ! -f "$REQ" ]]; then
  echo "Missing $REQ" >&2
  exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script is for macOS only (got $(uname -s))" >&2
  exit 1
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "Apple Silicon (arm64) only; this machine is $(uname -m)" >&2
  exit 1
fi
TRIPLE="aarch64-apple-darwin"

ARCHIVE_NAME="cpython-${CPYTHON_FULL}+${PBS_RELEASE}-${TRIPLE}-install_only.tar.gz"
# '+' must be encoded in the URL path
ARCHIVE_URL_PATH="cpython-${CPYTHON_FULL}%2B${PBS_RELEASE}-${TRIPLE}-install_only.tar.gz"
URL="https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_RELEASE}/${ARCHIVE_URL_PATH}"

echo "Bundling standalone Python ${CPYTHON_FULL} (${TRIPLE}) -> ${DEST}"

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

echo "Download: ${URL}"
curl -fL --connect-timeout 30 --retry 3 -o "${TMP}/${ARCHIVE_NAME}" "${URL}"

tar -xzf "${TMP}/${ARCHIVE_NAME}" -C "${TMP}"
if [[ ! -d "${TMP}/python/bin" ]]; then
  echo "Unexpected tarball layout (expected python/bin)" >&2
  exit 1
fi

rm -rf "${DEST}"
mkdir -p "${DEST}"
# Flatten `python/` into python-runtime/ so resources/python-runtime matches Windows layout root (bin/, lib/, …)
shopt -s dotglob nullglob
mv "${TMP}/python/"* "${DEST}/"

PY="${DEST}/bin/python3.12"
if [[ ! -x "$PY" ]]; then
  PY="${DEST}/bin/python3"
fi
if [[ ! -x "$PY" ]]; then
  echo "No python3.12 or python3 under ${DEST}/bin" >&2
  exit 1
fi

echo "Installing Python deps with ${PY} …"
"${PY}" -m pip install --upgrade pip setuptools wheel
"${PY}" -m pip install --no-warn-script-location -r "${REQ}"

echo "Done. Bundled runtime ready for: npm run tauri build"

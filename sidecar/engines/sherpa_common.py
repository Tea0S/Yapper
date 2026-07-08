"""Shared helpers for sherpa-onnx model download and ONNX Runtime provider selection."""
from __future__ import annotations

import shutil
import tarfile
import urllib.request
from pathlib import Path

SHERPA_RELEASE_BASE = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
)

# Legacy NeMo HF IDs stored in older Yapper settings.
LEGACY_PARAKEET_IDS: dict[str, str] = {
    "nvidia/parakeet-tdt-0.6b-v3": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
    "nvidia/parakeet-tdt-0.6b-v2": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
}


def normalize_parakeet_model_id(model_id: str) -> str:
    m = (model_id or "").strip()
    return LEGACY_PARAKEET_IDS.get(m, m)


def onnx_provider(device: str) -> str:
    if device == "cuda":
        try:
            import sherpa_onnx  # noqa: F401

            return "cuda"
        except Exception:
            pass
    return "cpu"


def ensure_sherpa_tarball(model_id: str, model_dir: str | None) -> Path:
    """Download and extract a sherpa-onnx release tarball if missing."""
    if not model_dir:
        raise RuntimeError("model_dir is required for sherpa-onnx models")
    root = Path(model_dir) / "sherpa"
    root.mkdir(parents=True, exist_ok=True)
    dest = root / model_id
    if (dest / "tokens.txt").is_file() and (
        (dest / "encoder.int8.onnx").is_file()
        or (dest / "encoder.onnx").is_file()
    ):
        return dest

    archive = root / f"{model_id}.tar.bz2"
    url = f"{SHERPA_RELEASE_BASE}/{model_id}.tar.bz2"
    if not archive.is_file():
        tmp = archive.with_suffix(".part")
        req = urllib.request.Request(url, headers={"User-Agent": "yapper-sidecar/1.0"})
        with urllib.request.urlopen(req, timeout=600) as resp, open(tmp, "wb") as out:
            shutil.copyfileobj(resp, out)
        tmp.replace(archive)

    extract_root = root / "_extract"
    if extract_root.is_dir():
        shutil.rmtree(extract_root, ignore_errors=True)
    extract_root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:bz2") as tf:
        tf.extractall(extract_root)
    candidates = [p for p in extract_root.iterdir() if p.is_dir()]
    if not candidates:
        raise RuntimeError(f"Empty sherpa archive: {model_id}")
    src = candidates[0]
    if dest.is_dir():
        shutil.rmtree(dest, ignore_errors=True)
    shutil.move(str(src), str(dest))
    shutil.rmtree(extract_root, ignore_errors=True)
    return dest


def transducer_paths(model_root: Path) -> tuple[str, str, str, str]:
    enc = model_root / "encoder.int8.onnx"
    if not enc.is_file():
        enc = model_root / "encoder.onnx"
    dec = model_root / "decoder.int8.onnx"
    if not dec.is_file():
        dec = model_root / "decoder.onnx"
    join = model_root / "joiner.int8.onnx"
    if not join.is_file():
        join = model_root / "joiner.onnx"
    tok = model_root / "tokens.txt"
    for p in (enc, dec, join, tok):
        if not p.is_file():
            raise RuntimeError(f"Missing sherpa model file: {p}")
    return str(enc), str(dec), str(join), str(tok)

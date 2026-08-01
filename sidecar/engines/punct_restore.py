"""Optional sherpa-onnx English punctuation restore (lazy-loaded)."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from .sherpa_common import ensure_punct_tarball, onnx_provider

PUNCT_MODEL_ID = "sherpa-onnx-online-punct-en-2024-08-06"

_PUNCT: Any = None
_MODEL_DIR: str | None = None


def available() -> bool:
    try:
        import sherpa_onnx  # noqa: F401

        return True
    except Exception:
        return False


def unload() -> None:
    global _PUNCT, _MODEL_DIR
    _PUNCT = None
    _MODEL_DIR = None


def ensure_loaded(model_dir: str | None, device: str = "cpu") -> None:
    """Download (if needed) and load the online EN punctuation model."""
    global _PUNCT, _MODEL_DIR
    if _PUNCT is not None and _MODEL_DIR == model_dir:
        return
    if not available():
        raise RuntimeError("sherpa-onnx is not installed")
    import sherpa_onnx

    root = ensure_punct_tarball(PUNCT_MODEL_ID, model_dir)
    cnn = root / "model.int8.onnx"
    if not cnn.is_file():
        cnn = root / "model.onnx"
    vocab = root / "bpe.vocab"
    if not cnn.is_file() or not vocab.is_file():
        raise RuntimeError(f"Missing punct model files under {root}")

    config = sherpa_onnx.OnlinePunctuationConfig(
        model=sherpa_onnx.OnlinePunctuationModelConfig(
            cnn_bilstm=str(cnn),
            bpe_vocab=str(vocab),
            num_threads=1,
            provider=onnx_provider(device),
        ),
    )
    _PUNCT = sherpa_onnx.OnlinePunctuation(config)
    _MODEL_DIR = model_dir


def restore(text: str, model_dir: str | None, device: str = "cpu") -> str:
    """Add punctuation/casing. Returns input unchanged on empty text or failure to load."""
    t = (text or "").strip()
    if not t:
        return text or ""
    ensure_loaded(model_dir, device)
    assert _PUNCT is not None
    out = _PUNCT.add_punctuation(t)
    if isinstance(out, str) and out.strip():
        return out
    return text

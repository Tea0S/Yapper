"""Offline Parakeet inference via sherpa-onnx (CPU or CUDA)."""
from __future__ import annotations

from typing import Any

import numpy as np

from .sherpa_common import (
    ensure_sherpa_tarball,
    normalize_parakeet_model_id,
    onnx_provider,
    transducer_paths,
)

_RECOGNIZER: Any = None
_MODEL_ID = ""


def available() -> bool:
    try:
        import sherpa_onnx  # noqa: F401

        return True
    except ImportError:
        return False


def load(model_id: str, device: str, model_dir: str | None) -> None:
    global _RECOGNIZER, _MODEL_ID
    import sherpa_onnx

    mid = normalize_parakeet_model_id(model_id)
    root = ensure_sherpa_tarball(mid, model_dir)
    enc, dec, join, tok = transducer_paths(root)
    provider = onnx_provider(device)
    _RECOGNIZER = sherpa_onnx.OfflineRecognizer.from_transducer(
        tokens=tok,
        encoder=enc,
        decoder=dec,
        joiner=join,
        num_threads=2,
        sample_rate=16000,
        feature_dim=80,
        decoding_method="greedy_search",
        provider=provider,
    )
    _MODEL_ID = mid


def unload() -> None:
    global _RECOGNIZER, _MODEL_ID
    _RECOGNIZER = None
    _MODEL_ID = ""


def is_loaded() -> bool:
    return _RECOGNIZER is not None


def model_id() -> str:
    return _MODEL_ID


def transcribe_pcm_i16(pcm: bytes, sample_rate: int) -> tuple[str, float]:
    if _RECOGNIZER is None:
        return "", 0.0
    samples = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
    stream = _RECOGNIZER.create_stream()
    stream.accept_waveform(sample_rate, samples)
    _RECOGNIZER.decode_stream(stream)
    result = _RECOGNIZER.get_result(stream)
    text = (result.text if hasattr(result, "text") else str(result)).strip()
    duration = len(samples) / max(sample_rate, 1)
    return text, 0.01 if duration <= 0 else 0.05


def _read_audio_f32(path: str) -> tuple[Any, int]:
    import av
    import numpy as np

    container = av.open(path)
    stream = container.streams.audio[0]
    chunks: list[np.ndarray] = []
    sr = stream.sample_rate or 16000
    for frame in container.decode(audio=0):
        arr = frame.to_ndarray()
        if arr.ndim > 1:
            arr = arr.mean(axis=0)
        chunks.append(arr.astype(np.float32) / 32768.0 if arr.dtype == np.int16 else arr.astype(np.float32))
    if not chunks:
        return np.zeros(0, dtype=np.float32), sr
    return np.concatenate(chunks), sr


def transcribe_file(path: str) -> str:
    if _RECOGNIZER is None:
        raise RuntimeError("Parakeet model not loaded")
    samples, sr = _read_audio_f32(path)
    stream = _RECOGNIZER.create_stream()
    stream.accept_waveform(sr, samples)
    _RECOGNIZER.decode_stream(stream)
    result = _RECOGNIZER.get_result(stream)
    return (result.text if hasattr(result, "text") else str(result)).strip()

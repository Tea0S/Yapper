"""Streaming sherpa-onnx recognizer (Parakeet Unified EN)."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

import numpy as np

from .sherpa_common import ensure_sherpa_tarball, onnx_provider, transducer_paths

DEFAULT_STREAM_MODEL = "sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-560ms"

_RECOGNIZER: Any = None
_MODEL_ID = ""


def available() -> bool:
    try:
        import sherpa_onnx  # noqa: F401

        return True
    except ImportError:
        return False


def ensure_recognizer(model_id: str, device: str, model_dir: str | None) -> None:
    global _RECOGNIZER, _MODEL_ID
    if _RECOGNIZER is not None and _MODEL_ID == model_id:
        return
    import sherpa_onnx

    root = ensure_sherpa_tarball(model_id, model_dir)
    enc, dec, join, tok = transducer_paths(root)
    provider = onnx_provider(device)
    _RECOGNIZER = sherpa_onnx.OnlineRecognizer.from_transducer(
        tokens=tok,
        encoder=enc,
        decoder=dec,
        joiner=join,
        num_threads=2,
        sample_rate=16000,
        feature_dim=80,
        decoding_method="greedy_search",
        enable_endpoint_detection=False,
        provider=provider,
    )
    _MODEL_ID = model_id


@dataclass
class StreamSession:
    session_id: int
    model_id: str
    device: str
    model_dir: str | None
    stream: Any = None
    last_text: str = ""
    finished: bool = False

    def _ensure(self) -> None:
        ensure_recognizer(self.model_id, self.device, self.model_dir)
        if self.stream is None:
            self.stream = _RECOGNIZER.create_stream()

    def feed(self, pcm: bytes, sample_rate: int) -> str:
        self._ensure()
        samples = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
        self.stream.accept_waveform(sample_rate, samples)
        while _RECOGNIZER.is_ready(self.stream):
            _RECOGNIZER.decode_stream(self.stream)
        text = _RECOGNIZER.get_result(self.stream).strip()
        if text:
            self.last_text = text
        return self.last_text

    def finish(self) -> str:
        if self.stream is None:
            return self.last_text
        tail = np.zeros(int(0.3 * 16000), dtype=np.float32)
        self.stream.accept_waveform(16000, tail)
        self.stream.input_finished()
        while _RECOGNIZER.is_ready(self.stream):
            _RECOGNIZER.decode_stream(self.stream)
        text = _RECOGNIZER.get_result(self.stream).strip()
        if text:
            self.last_text = text
        self.finished = True
        return self.last_text


_SESSIONS: dict[int, StreamSession] = {}


def start_session(
    session_id: int,
    model_id: str,
    device: str,
    model_dir: str | None,
) -> None:
    mid = model_id.strip() or DEFAULT_STREAM_MODEL
    _SESSIONS[session_id] = StreamSession(
        session_id=session_id,
        model_id=mid,
        device=device,
        model_dir=model_dir,
    )


def feed_session(session_id: int, pcm: bytes, sample_rate: int) -> str:
    sess = _SESSIONS.get(session_id)
    if sess is None:
        raise RuntimeError(f"Unknown stream session {session_id}")
    return sess.feed(pcm, sample_rate)


def end_session(session_id: int) -> str:
    sess = _SESSIONS.pop(session_id, None)
    if sess is None:
        return ""
    return sess.finish()

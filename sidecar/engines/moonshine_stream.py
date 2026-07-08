"""Streaming live dictation via moonshine-voice."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import numpy as np

ARCH_MAP = {
    "tiny_streaming": "TINY_STREAMING",
    "small_streaming": "SMALL_STREAMING",
    "medium_streaming": "MEDIUM_STREAMING",
}


def available() -> bool:
    try:
        import moonshine_voice  # noqa: F401

        return True
    except ImportError:
        return False


def _model_arch(name: str) -> Any:
    from moonshine_voice import ModelArch

    key = (name or "small_streaming").strip().lower()
    attr = ARCH_MAP.get(key, "SMALL_STREAMING")
    return getattr(ModelArch, attr)


def resolve_model(model_name: str, model_dir: str | None) -> tuple[str, Any]:
    """Download/cache Moonshine streaming weights and return (path, arch)."""
    from moonshine_voice import get_model_for_language

    arch = _model_arch(model_name)
    kwargs: dict[str, Any] = {"wanted_model_arch": arch}
    if model_dir:
        kwargs["cache_root"] = Path(model_dir) / "moonshine_cache"
    return get_model_for_language("en", **kwargs)


def _line_text(event: Any) -> str:
    line = getattr(event, "line", None)
    if line is None:
        return ""
    return str(getattr(line, "text", "") or "").strip()


def _transcript_text(transcriber: Any) -> str:
    try:
        tr = transcriber.update_transcription()
    except Exception:
        return ""
    full = str(getattr(tr, "text", "") or "").strip()
    if full:
        return full
    lines = getattr(tr, "lines", None)
    if not lines:
        return ""
    parts: list[str] = []
    for line in lines:
        t = str(getattr(line, "text", "") or "").strip()
        if t:
            parts.append(t)
    return " ".join(parts)


@dataclass
class StreamSession:
    session_id: int
    model_name: str
    model_dir: str | None
    transcriber: Any = None
    last_text: str = ""
    started: bool = False

    def _ensure(self) -> None:
        if self.transcriber is not None:
            return
        from moonshine_voice import Transcriber, TranscriptEventListener

        model_path, model_arch = resolve_model(self.model_name, self.model_dir)

        class _Listener(TranscriptEventListener):
            def __init__(self, outer: StreamSession) -> None:
                self._outer = outer

            def on_line_text_changed(self, event: Any) -> None:
                text = _line_text(event)
                if text:
                    self._outer.last_text = text

            def on_line_updated(self, event: Any) -> None:
                text = _line_text(event)
                if text:
                    self._outer.last_text = text

            def on_line_completed(self, event: Any) -> None:
                text = _line_text(event)
                if text:
                    self._outer.last_text = text

        self.transcriber = Transcriber(
            model_path=model_path,
            model_arch=model_arch,
            update_interval=0.25,
        )
        self.transcriber.add_listener(_Listener(self))

    def start(self) -> None:
        self._ensure()
        if not self.started:
            self.transcriber.start()
            self.started = True

    def feed(self, pcm: bytes, sample_rate: int) -> str:
        self.start()
        samples = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
        self.transcriber.add_audio(samples.tolist(), sample_rate)
        text = _transcript_text(self.transcriber)
        if text:
            self.last_text = text
        return self.last_text

    def finish(self) -> str:
        if self.transcriber is None:
            return self.last_text
        text = _transcript_text(self.transcriber)
        if text:
            self.last_text = text
        if self.started:
            self.transcriber.stop()
        return self.last_text


_SESSIONS: dict[int, StreamSession] = {}


def start_session(session_id: int, model_name: str, model_dir: str | None) -> None:
    """Register a session; model weights load on the first audio feed."""
    _SESSIONS[session_id] = StreamSession(
        session_id=session_id,
        model_name=model_name or "small_streaming",
        model_dir=model_dir,
    )


def feed_session(session_id: int, pcm: bytes, sample_rate: int) -> str:
    sess = _SESSIONS.get(session_id)
    if sess is None:
        raise RuntimeError(f"Unknown moonshine session {session_id}")
    return sess.feed(pcm, sample_rate)


def end_session(session_id: int) -> str:
    sess = _SESSIONS.pop(session_id, None)
    if sess is None:
        return ""
    return sess.finish()

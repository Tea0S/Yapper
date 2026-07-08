"""Streaming live dictation via moonshine-voice."""
from __future__ import annotations

import subprocess
import sys
from dataclasses import dataclass, field
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


def ensure_models(model_dir: str | None) -> str:
    """Return moonshine model directory, downloading English weights if needed."""
    from pathlib import Path

    root = Path(model_dir or ".") / "moonshine" / "en"
    if any(root.rglob("*.onnx")) or any(root.rglob("*.bin")):
        return str(root)
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [sys.executable, "-m", "moonshine_voice.download", "--language", "en"],
        check=False,
        cwd=str(root.parent.parent),
    )
    return str(root)


@dataclass
class StreamSession:
    session_id: int
    model_name: str
    model_dir: str | None
    transcriber: Any = None
    last_text: str = ""
    started: bool = False
    _updates: list[str] = field(default_factory=list)

    def _ensure(self) -> None:
        if self.transcriber is not None:
            return
        from moonshine_voice import Transcriber, TranscriptEventListener

        path = ensure_models(self.model_dir)

        class _Listener(TranscriptEventListener):
            def __init__(self, outer: StreamSession) -> None:
                self._outer = outer

            def on_transcript_update(self, event: Any) -> None:
                text = getattr(event, "text", None) or ""
                text = str(text).strip()
                if text:
                    self._outer.last_text = text
                    self._outer._updates.append(text)

        self.transcriber = Transcriber(
            model_path=path,
            model_arch=_model_arch(self.model_name),
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
        return self.last_text

    def finish(self) -> str:
        if self.transcriber is None:
            return self.last_text
        if self.started:
            self.transcriber.stop()
        return self.last_text


_SESSIONS: dict[int, StreamSession] = {}


def start_session(session_id: int, model_name: str, model_dir: str | None) -> None:
    _SESSIONS[session_id] = StreamSession(
        session_id=session_id,
        model_name=model_name or "small_streaming",
        model_dir=model_dir,
    )
    _SESSIONS[session_id].start()


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

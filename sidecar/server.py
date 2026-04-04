#!/usr/bin/env python3
"""Yapper inference sidecar: JSON lines on stdin -> JSON lines on stdout."""
from __future__ import annotations

import base64
import gc
import json
import os
import sys
import time
import warnings
from pathlib import Path
from typing import Any, Optional

# requests/urllib3 pin skew in global site-packages floods stderr; not fatal for dictation.
warnings.filterwarnings("ignore", message=r".*doesn't match a supported version.*")

MODEL: Any = None
MODEL_NAME = ""
DEVICE = "cpu"
MOCK = False
# When True, `MODEL` is a sentinel object; inference uses mlx_whisper (Apple Silicon).
USE_MLX = False
# Last init parameters for EnsureModel / unload / reload
CONFIG: dict[str, Any] = {}


def _verbose() -> bool:
    v = os.environ.get("YAPPER_VERBOSE", "").strip().lower()
    if v in ("0", "false", "off"):
        return False
    if v in ("1", "true", "on") or bool(v):
        return True
    return __debug__


def vlog(msg: str) -> None:
    if _verbose():
        sys.stderr.write(f"yapper-sidecar: {msg}\n")
        sys.stderr.flush()


def emit(obj: dict) -> None:
    t = obj.get("type", "?")
    extra = ""
    if t == "final":
        extra = f" seq={obj.get('seq')} text_chars={len(obj.get('text') or '')}"
    elif t == "error":
        extra = f" msg={str(obj.get('message', ''))[:100]}"
    vlog(f"emit → stdout type={t}{extra}")
    line = json.dumps(obj, ensure_ascii=False) + "\n"
    sys.stdout.write(line)
    sys.stdout.flush()
    vlog(f"emit flushed (line_len={len(line)})")


def unload_whisper() -> None:
    """Release model weights and try to return VRAM to the driver."""
    global MODEL, MODEL_NAME, USE_MLX
    if USE_MLX:
        try:
            from mlx_whisper.transcribe import ModelHolder

            ModelHolder.model = None
            ModelHolder.model_path = None
        except ImportError:
            pass
        USE_MLX = False
    MODEL = None
    MODEL_NAME = ""
    gc.collect()
    try:
        import torch

        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except ImportError:
        pass


def list_engines() -> list[str]:
    engines = ["whisper"]
    try:
        import torch

        if torch.cuda.is_available():
            try:
                import nemo.collections.asr  # noqa: F401

                engines.append("parakeet")
            except ImportError:
                pass
    except ImportError:
        pass
    return engines


def _looks_like_incomplete_hf_whisper_cache(err: BaseException) -> bool:
    """True when weights look truncated (HF snapshot) — not permission/AV locks or unrelated I/O."""
    s = str(err).lower()
    if any(
        x in s
        for x in (
            "permission",
            "access is denied",
            "errno 13",
            "operation not permitted",
        )
    ):
        return False
    # Require model.bin + typical CT2/HF missing-weight phrasing (avoid wiping cache on any "unable to open file").
    if "model.bin" not in s:
        return False
    return any(
        x in s
        for x in (
            "unable to open file",
            "cannot open",
            "failed to open",
            "no such file",
            "errno 2",
            "does not exist",
        )
    )


def _ct2_flat_path(download_root: str, model: str) -> Path:
    """Real on-disk files for CT2 (no HF snapshot symlinks). Reliable on Windows + embeddable Python."""
    safe = model.replace("/", "__").replace("\\", "_").strip() or "model"
    return Path(download_root) / "_ct2_flat" / safe


def _purge_ct2_flat(download_root: str, model: str) -> None:
    import shutil

    p = _ct2_flat_path(download_root, model)
    if p.is_dir():
        sys.stderr.write(f"yapper-sidecar: removing flat CT2 model dir: {p}\n")
        sys.stderr.flush()
        shutil.rmtree(p, ignore_errors=True)


def _use_flat_ct2_download(model: str, download_root: Optional[str]) -> bool:
    """Bundled installs use embeddable Python; HF hub cache symlinks often break there. Dev full-Python is fine."""
    if not download_root or not model.strip():
        return False
    m = model.strip()
    if os.path.isdir(m):
        return False
    if m.startswith(("/", "\\")) or (len(m) >= 2 and m[1] == ":"):
        return False
    return True


def _purge_incomplete_snapshot_only(download_root: str, err: BaseException) -> bool:
    """Remove only the snapshot dir named in the error, if it lives under download_root."""
    import re
    import shutil

    root = Path(download_root)
    try:
        root_resolved = root.resolve()
    except OSError:
        root_resolved = root

    s = str(err)
    for pat in (
        r"model\s+['\"]([^'\"]+)['\"]",
        r"in model\s+['\"]([^'\"]+)['\"]",
    ):
        m = re.search(pat, s, re.I)
        if not m:
            continue
        candidate = Path(m.group(1).strip())
        try:
            if not candidate.is_dir():
                continue
            cand_res = candidate.resolve()
        except OSError:
            continue
        try:
            cand_res.relative_to(root_resolved)
        except ValueError:
            continue
        rel = str(cand_res).replace("\\", "/").lower()
        if "snapshots" not in rel and "_ct2_flat" not in rel:
            continue
        sys.stderr.write(
            f"yapper-sidecar: removing incomplete model dir (snapshot or flat copy): {cand_res}\n"
        )
        sys.stderr.flush()
        shutil.rmtree(cand_res, ignore_errors=True)
        return True
    return False


def _purge_hf_repo_cache(download_root: str, model_id: str) -> None:
    """Remove Hugging Face hub folder for this repo so snapshot_download can run clean."""
    import re
    import shutil
    from pathlib import Path

    try:
        import faster_whisper.utils as fwu
    except ImportError:
        _purge_ct2_flat(download_root, model_id)
        return
    if re.match(r".*/.*", model_id):
        repo_id = model_id
    else:
        repo_id = fwu._MODELS.get(model_id)
    if not repo_id:
        _purge_ct2_flat(download_root, model_id)
        return
    folder = Path(download_root) / f"models--{repo_id.replace('/', '--')}"
    if folder.is_dir():
        sys.stderr.write(
            f"yapper-sidecar: removing incomplete model cache (will re-download): {folder}\n"
        )
        sys.stderr.flush()
        shutil.rmtree(folder, ignore_errors=True)
    _purge_ct2_flat(download_root, model_id)


def _is_mlx_model_id(model: str) -> bool:
    m = (model or "").strip()
    return "mlx-community/" in m or m.endswith("-mlx")


def load_whisper(model: str, device: str, compute_type: str, model_dir: Optional[str]) -> None:
    global MODEL, MODEL_NAME
    from faster_whisper import WhisperModel
    from faster_whisper.utils import download_model

    download_root = model_dir or None
    dev = device if device in ("cuda", "cpu") else "cpu"
    flat_dir = _ct2_flat_path(download_root, model) if _use_flat_ct2_download(model, download_root) else None

    sys.stderr.write(
        f"yapper-sidecar: loading WhisperModel model_id={model!r} device={dev!r} "
        f"compute_type={compute_type!r} download_root={download_root!r} "
        f"flat_ct2={str(flat_dir) if flat_dir else 'off'}\n"
    )
    sys.stderr.flush()

    for attempt in range(2):
        try:
            if flat_dir is not None:
                assert download_root is not None
                flat_dir.mkdir(parents=True, exist_ok=True)
                if not (flat_dir / "model.bin").is_file():
                    download_model(
                        model,
                        output_dir=str(flat_dir),
                        cache_dir=download_root,
                        local_files_only=False,
                    )
                MODEL = WhisperModel(
                    str(flat_dir),
                    device=dev,
                    compute_type=compute_type,
                )
            else:
                MODEL = WhisperModel(
                    model,
                    device=dev,
                    compute_type=compute_type,
                    download_root=download_root,
                )
            break
        except Exception as e:
            if (
                attempt == 0
                and download_root
                and _looks_like_incomplete_hf_whisper_cache(e)
            ):
                sys.stderr.write(
                    f"yapper-sidecar: load failed ({e}); clearing broken cache and retrying once.\n"
                )
                sys.stderr.flush()
                if not _purge_incomplete_snapshot_only(download_root, e):
                    _purge_hf_repo_cache(download_root, model)
                continue
            raise

    MODEL_NAME = model
    resolved = getattr(MODEL, "model_path", None)
    if resolved is None:
        # Some faster-whisper builds expose the path only on the underlying CT2 model.
        inner = getattr(MODEL, "model", None)
        if inner is not None:
            resolved = getattr(inner, "model_path", None)
    cache_names: list[str] = []
    if model_dir:
        root = Path(model_dir)
        if root.is_dir():
            cache_names = sorted(p.name for p in root.iterdir())
    sys.stderr.write(
        f"yapper-sidecar: WhisperModel loaded OK model_name={MODEL_NAME!r} "
        f"resolved_path={resolved!r} download_root_entry_count={len(cache_names)} "
        f"sample={cache_names[:10]!r}\n"
    )
    sys.stderr.flush()
    vlog(
        f"load_whisper done model_id={model!r} resolved_path={resolved!r} download_root_entries={cache_names!r}"
    )


def load_mlx_whisper(model: str, model_dir: Optional[str]) -> None:
    """Apple Silicon: Hugging Face MLX checkpoints via mlx-whisper (Metal)."""
    global MODEL, MODEL_NAME, USE_MLX
    try:
        import mlx_whisper  # noqa: F401
    except ImportError as e:
        raise RuntimeError(
            "mlx-whisper is not installed. On Apple Silicon Macs, run: "
            "pip install -r sidecar/requirements-macos.txt"
        ) from e
    if model_dir:
        try:
            hub = str(Path(model_dir) / "hub")
            Path(hub).mkdir(parents=True, exist_ok=True)
            os.environ.setdefault("HF_HUB_CACHE", hub)
            os.environ.setdefault("HUGGINGFACE_HUB_CACHE", hub)
        except OSError:
            pass
    import mlx.core as mx
    from mlx_whisper.transcribe import ModelHolder

    dtype = mx.float16
    sys.stderr.write(
        f"yapper-sidecar: MLX loading weights (HF download if needed) repo={model!r} …\n"
    )
    sys.stderr.flush()
    ModelHolder.get_model(model, dtype=dtype)
    USE_MLX = True
    MODEL = object()
    MODEL_NAME = model
    sys.stderr.write(f"yapper-sidecar: MLX Whisper loaded repo={model!r}\n")
    sys.stderr.flush()


def load_whisper_for_config(
    model: str, device: str, compute_type: str, model_dir: Optional[str]
) -> None:
    if _is_mlx_model_id(model):
        load_mlx_whisper(model, model_dir)
    else:
        load_whisper(model, device, compute_type, model_dir)


def store_config(
    model: str,
    dev: str,
    compute: str,
    model_dir: Optional[str],
    engine: str,
    mock: bool,
    whisper: Optional[dict[str, Any]] = None,
) -> None:
    CONFIG.clear()
    CONFIG["model"] = model
    CONFIG["device"] = dev
    CONFIG["compute_type"] = compute
    CONFIG["model_dir"] = model_dir
    CONFIG["engine"] = engine
    CONFIG["mock"] = mock
    CONFIG["whisper"] = dict(whisper) if isinstance(whisper, dict) else {}


def load_from_config() -> None:
    if CONFIG.get("mock"):
        return
    if CONFIG.get("engine") != "whisper":
        raise RuntimeError("Only whisper supports reload in this sidecar")
    load_whisper_for_config(
        CONFIG["model"],
        CONFIG["device"],
        CONFIG["compute_type"],
        CONFIG.get("model_dir"),
    )


def _trim_float_audio_edges(audio: Any, sample_rate: int) -> Any:
    """Strip low-RMS frames from start/end only (reduces tail hallucinations, not internal pauses)."""
    import numpy as np

    if audio.size < max(sample_rate // 5, 64):
        return audio
    frame = max(1, int(sample_rate * 0.025))
    hop = max(1, frame // 2)
    rms_list: list[float] = []
    pos: list[int] = []
    for i in range(0, len(audio) - frame + 1, hop):
        seg = audio[i : i + frame]
        rms_list.append(float(np.sqrt(np.mean(seg**2))))
        pos.append(i)
    if not rms_list:
        return audio
    peak = max(rms_list) or 1e-12
    thresh = max(0.007, peak * 0.07)
    first_i = 0
    for k, r in enumerate(rms_list):
        if r >= thresh:
            first_i = pos[k]
            break
    last_i = len(audio)
    for k in range(len(rms_list) - 1, -1, -1):
        if rms_list[k] >= thresh:
            last_i = min(len(audio), pos[k] + frame + hop)
            break
    if last_i <= first_i:
        return audio
    pad = int(0.1 * sample_rate)
    i0 = max(0, first_i - pad)
    i1 = min(len(audio), last_i + pad)
    return audio[i0:i1]


# Phrases Whisper often invents on noise / training-data bleed; drop whole segments that are mostly this.
_HALLUCINATION_SUBSTRINGS = (
    "thanks for watching",
    "thank you for watching",
    "please subscribe",
    "see you next time",
    "hit the like button",
    "leave a comment below",
    "smash that like",
)


def _segment_is_likely_hallucination(text: str) -> bool:
    t = text.lower().strip()
    if len(t) < 6:
        return False
    return any(s in t for s in _HALLUCINATION_SUBSTRINGS)


def _segment_probs(seg: Any) -> tuple[str, float, Any]:
    """Text, no_speech_prob, avg_logprob from a faster-whisper Segment or an mlx-whisper segment dict."""
    if isinstance(seg, dict):
        t = (seg.get("text") or "").strip()
        nsp = float(seg.get("no_speech_prob", 0.0) or 0.0)
        alp = seg.get("avg_logprob")
        return t, nsp, alp
    t = (getattr(seg, "text", None) or "").strip()
    nsp = float(getattr(seg, "no_speech_prob", 0.0) or 0.0)
    alp = getattr(seg, "avg_logprob", None)
    return t, nsp, alp


def _filtered_text_from_segments(
    segments: Any,
    *,
    log_tag: str,
    fallback_text: Optional[str] = None,
) -> str:
    """Same post-filter as live faster-whisper PCM: drop no-speech / bad logprob / common hallucinations."""
    parts: list[str] = []
    n_seg = 0
    n_kept = 0
    for seg in segments or []:
        n_seg += 1
        t, nsp, alp = _segment_probs(seg)
        if not t:
            continue
        if nsp >= 0.72:
            vlog(f"{log_tag}: skip seg no_speech_prob={nsp:.2f} {t[:50]!r}")
            continue
        if alp is not None and float(alp) < -1.05:
            vlog(f"{log_tag}: skip seg avg_logprob={alp:.2f} {t[:50]!r}")
            continue
        if _segment_is_likely_hallucination(t):
            vlog(f"{log_tag}: skip seg hallucination pattern {t[:60]!r}")
            continue
        parts.append(t)
        n_kept += 1
    out = " ".join(parts).strip()
    vlog(f"{log_tag}: segments total={n_seg} kept={n_kept} text_chars={len(out)}")
    if out:
        return out
    if isinstance(fallback_text, str) and fallback_text.strip():
        return fallback_text.strip()
    return ""


def _w_float(w: dict[str, Any], key: str, default: float) -> float:
    v = w.get(key, default)
    try:
        return float(v)
    except (TypeError, ValueError):
        return float(default)


def _w_int(w: dict[str, Any], key: str, default: int) -> int:
    v = w.get(key, default)
    try:
        return int(v)
    except (TypeError, ValueError):
        return int(default)


def _w_bool(w: dict[str, Any], key: str, default: bool) -> bool:
    v = w.get(key, default)
    if isinstance(v, bool):
        return v
    if isinstance(v, str):
        return v.strip().lower() in ("1", "true", "yes", "on")
    return default


def build_transcribe_kwargs(*, for_file: bool) -> dict[str, Any]:
    """Merge init `whisper` dict with sane defaults (PCM vs file VAD differ)."""
    w: dict[str, Any] = CONFIG.get("whisper") or {}
    lang = w.get("language")
    if not isinstance(lang, str) or not lang.strip():
        language: Any = None
    else:
        language = lang.strip()

    initial_prompt = w.get("initial_prompt")
    if not isinstance(initial_prompt, str):
        initial_prompt = ""
    initial_prompt = initial_prompt.strip()

    vad_filter = (
        _w_bool(w, "vad_filter_file", True)
        if for_file
        else _w_bool(w, "vad_filter_pcm", False)
    )

    kw: dict[str, Any] = dict(
        language=language,
        vad_filter=vad_filter,
        beam_size=max(1, _w_int(w, "beam_size", 5)),
        best_of=max(1, _w_int(w, "best_of", 1)),
        patience=_w_float(w, "patience", 1.0),
        temperature=_w_float(w, "temperature", 0.0),
        no_speech_threshold=_w_float(w, "no_speech_threshold", 0.78),
        log_prob_threshold=_w_float(w, "log_prob_threshold", -0.55),
        compression_ratio_threshold=_w_float(w, "compression_ratio_threshold", 1.9),
        condition_on_previous_text=_w_bool(w, "condition_on_previous_text", False),
    )
    if initial_prompt:
        kw["initial_prompt"] = initial_prompt
    return kw


def _call_mlx_transcribe(audio: Any, *, for_file: bool) -> dict[str, Any]:
    import mlx_whisper

    kw = build_transcribe_kwargs(for_file=for_file)
    wcfg = CONFIG.get("whisper") or {}
    hst = _w_float(wcfg, "hallucination_silence_threshold", 1.6)
    if kw.get("vad_filter") and not for_file:
        vlog(
            "mlx pcm: vad_filter_pcm is on but mlx-whisper has no Silero VAD; "
            "using Rust energy VAD + edge trim only (same as faster-whisper note in code)."
        )
    # Match faster-whisper: temperature 0 → greedy only. MLX default multi-temp retries change behavior.
    temperature = float(kw.get("temperature", 0.0))
    temp_arg: Any = (temperature,) if temperature > 0 else (0.0,)
    initial_prompt = kw.get("initial_prompt")
    if not isinstance(initial_prompt, str) or not initial_prompt.strip():
        ip = None
    else:
        ip = initial_prompt.strip()
    # mlx-whisper DecodingOptions: beam search is not implemented — never pass beam_size /
    # patience / best_of (faster-whisper defaults would set beam_size and break).
    decode_extras: dict[str, Any] = {}
    lang = kw.get("language")
    if isinstance(lang, str) and lang.strip():
        decode_extras["language"] = lang.strip()
    # Live dictation: text-only decoding (faster-whisper default for mic is also non-timestamp focused).
    if not for_file:
        decode_extras["without_timestamps"] = True
    return mlx_whisper.transcribe(
        audio,
        path_or_hf_repo=CONFIG["model"],
        verbose=None,
        temperature=temp_arg,
        compression_ratio_threshold=float(kw["compression_ratio_threshold"]),
        logprob_threshold=float(kw["log_prob_threshold"]),
        no_speech_threshold=float(kw["no_speech_threshold"]),
        condition_on_previous_text=bool(kw.get("condition_on_previous_text", False)),
        initial_prompt=ip,
        word_timestamps=False,
        hallucination_silence_threshold=hst,
        **decode_extras,
    )


def _mlx_result_to_text(result: dict[str, Any], *, log_tag: str) -> str:
    return _filtered_text_from_segments(
        result.get("segments"),
        log_tag=log_tag,
        fallback_text=result.get("text"),
    )


def transcribe_pcm_i16(pcm: bytes, sample_rate: int) -> tuple[str, float]:
    import numpy as np

    if MOCK:
        return "[mock transcription]", 0.01

    if MODEL is None:
        return "", 0.0

    if USE_MLX:
        audio = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
        before = len(audio)
        audio = _trim_float_audio_edges(audio, sample_rate)
        trimmed_ms = int(1000 * (before - len(audio)) / max(sample_rate, 1))
        duration_s = len(audio) / max(sample_rate, 1)
        vlog(
            f"transcribe_pcm_mlx: pcm_bytes={len(pcm)} sr={sample_rate} duration_s={duration_s:.3f} "
            f"trimmed_ms≈{trimmed_ms}"
        )
        t0 = time.perf_counter()
        result = _call_mlx_transcribe(audio, for_file=False)
        dt = time.perf_counter() - t0
        text = _mlx_result_to_text(result, log_tag="transcribe_mlx")
        rtf = (dt / duration_s) if duration_s > 1e-6 else 0.0
        vlog(
            f"transcribe_pcm_mlx: done in {dt:.2f}s text_chars={len(text)} rtf≈{rtf:.3f}"
        )
        return text, float(rtf)

    audio = np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0
    before = len(audio)
    audio = _trim_float_audio_edges(audio, sample_rate)
    trimmed_ms = int(1000 * (before - len(audio)) / max(sample_rate, 1))
    duration_s = len(audio) / max(sample_rate, 1)
    # Never use faster-whisper's Silero VAD on the full clip here: it often zeroed live mic audio.
    # Rust already gates with energy VAD; we merge segments client-side for one coherent decode.
    vf = build_transcribe_kwargs(for_file=False).get("vad_filter", False)
    vlog(
        f"transcribe_pcm: pcm_bytes={len(pcm)} sr={sample_rate} duration_s={duration_s:.3f} "
        f"trimmed_ms≈{trimmed_ms} vad_filter={vf}"
    )
    t0 = time.perf_counter()
    transcribe_kw = build_transcribe_kwargs(for_file=False)
    hst = _w_float(CONFIG.get("whisper") or {}, "hallucination_silence_threshold", 1.6)
    # Newer faster-whisper: suppress long-silence hallucinations inside a segment.
    try:
        segments, info = MODEL.transcribe(
            audio,
            **transcribe_kw,
            hallucination_silence_threshold=hst,
        )
    except TypeError:
        segments, info = MODEL.transcribe(audio, **transcribe_kw)
    dt = time.perf_counter() - t0
    text = _filtered_text_from_segments(segments, log_tag="transcribe_pcm", fallback_text=None)
    rtf = getattr(info, "duration", 0) and (getattr(info, "duration", 1) * 0.01)
    vlog(
        f"transcribe_pcm: done in {dt:.2f}s text_chars={len(text)} "
        f"info.duration={getattr(info, 'duration', None)!r}"
    )
    return text, float(rtf or 0.0)


def handle_init(msg: dict) -> None:
    global MOCK, MODEL, MODEL_NAME, DEVICE
    vlog(f"handle_init begin keys={list(msg.keys())}")
    MOCK = bool(msg.get("mock", False))
    model = msg.get("model", "base")
    device = msg.get("device", "cpu")
    compute = msg.get("compute_type", "int8")
    model_dir = msg.get("model_dir")
    engine = (msg.get("engine") or "whisper").lower()
    lazy_load = bool(msg.get("lazy_load", False))

    dev = device if device in ("cuda", "cpu") else "cpu"

    whisper_raw = msg.get("whisper")
    whisper_dict = whisper_raw if isinstance(whisper_raw, dict) else None
    store_config(model, dev, compute, model_dir, engine, MOCK, whisper_dict)
    sys.stderr.write(
        f"yapper-sidecar: handle_init model={model!r} device={dev!r} compute={compute!r} engine={engine!r} "
        f"mock={MOCK} lazy_load={lazy_load} model_dir={model_dir!r}\n"
    )
    sys.stderr.flush()

    if engine == "parakeet":
        emit(
            {
                "type": "error",
                "message": "Parakeet requires NeMo + CUDA. Install NVIDIA NeMo toolkit or switch engine to whisper in Settings.",
            }
        )
        emit(
            {
                "type": "ready",
                "engines": list_engines(),
                "inference_device": "cpu",
                "compute_type": compute,
            }
        )
        vlog("handle_init done (parakeet path)")
        return

    if MOCK:
        MODEL = None
        MODEL_NAME = model
        sys.stderr.write(
            "yapper-sidecar: MOCK mode — no Whisper weights loaded; chunks return placeholder text.\n"
        )
        sys.stderr.flush()
        emit(
            {
                "type": "ready",
                "engines": list_engines(),
                "inference_device": "mock",
                "compute_type": compute,
            }
        )
        vlog("handle_init done (mock)")
        return

    if lazy_load:
        sys.stderr.write(
            "yapper-sidecar: lazy_load — Whisper loads on first dictation; GPU/CPU stays idle until then.\n"
        )
        sys.stderr.flush()
        unload_whisper()
        emit({"type": "model_state", "loaded": False})
        emit(
            {
                "type": "ready",
                "engines": list_engines(),
                "inference_device": "pending_first_use",
                "compute_type": compute,
            }
        )
        vlog("handle_init done (lazy_load)")
        return

    try:
        load_whisper_for_config(model, dev, compute, model_dir)
    except Exception as e:
        vlog(f"handle_init load_whisper failed: {e!r}")
        emit({"type": "error", "message": f"Whisper load failed: {e}"})
        return
    DEVICE = "mlx" if USE_MLX else dev
    emit({"type": "model_state", "loaded": True})
    emit(
        {
            "type": "ready",
            "engines": list_engines(),
            "inference_device": DEVICE,
            "compute_type": compute,
        }
    )
    vlog("handle_init finished (whisper path)")


def handle_ensure_model() -> None:
    global DEVICE, MODEL_NAME
    vlog(f"handle_ensure_model MODEL_is_none={MODEL is None}")
    if CONFIG.get("engine") == "parakeet":
        emit({"type": "error", "message": "Parakeet is not loaded in this sidecar"})
        return
    if MOCK:
        MODEL_NAME = CONFIG.get("model", "base")
        emit({"type": "model_state", "loaded": True})
        return
    if MODEL is not None:
        emit({"type": "model_state", "loaded": True})
        return
    try:
        load_from_config()
    except Exception as e:
        emit({"type": "error", "message": f"Whisper load failed: {e}"})
        return
    DEVICE = "mlx" if USE_MLX else CONFIG["device"]
    MODEL_NAME = CONFIG["model"]
    emit({"type": "model_state", "loaded": True})


def handle_unload_model() -> None:
    unload_whisper()
    emit({"type": "model_state", "loaded": False})


def handle_chunk(msg: dict) -> None:
    seq = int(msg.get("seq", 0))
    sr = int(msg.get("sample_rate", 16000))
    b64 = msg.get("audio_b64", "")
    vlog(
        f"handle_chunk seq={seq} sr={sr} b64_len={len(b64) if isinstance(b64, str) else 'n/a'} MODEL_loaded={MODEL is not None}"
    )
    try:
        pcm = base64.b64decode(b64)
    except Exception as e:
        emit({"type": "error", "message": f"bad audio: {e}"})
        return

    if MOCK:
        emit({"type": "partial", "text": "[mock] ", "seq": seq})
        emit({"type": "final", "text": "[mock transcription]", "seq": seq, "rtf": 0.01})
        return

    if MODEL is None:
        emit({"type": "error", "message": "Model not loaded — wait for load or restart engine."})
        return

    vlog(f"handle_chunk pcm decoded bytes={len(pcm)} duration_s={len(pcm)/(2*max(sr,1)):.3f}")

    try:
        text, rtf = transcribe_pcm_i16(pcm, sr)
        if msg.get("is_final", True):
            emit(
                {
                    "type": "final",
                    "text": text,
                    "seq": int(seq),
                    "rtf": float(rtf),
                }
            )
        else:
            emit({"type": "partial", "text": text, "seq": int(seq)})
        vlog(f"handle_chunk emit done seq={seq}")
    except Exception as e:
        vlog(f"handle_chunk exception: {e!r}")
        emit({"type": "error", "message": f"transcribe failed: {e}"})


def handle_file(msg: dict) -> None:
    path = msg.get("path", "")
    p = Path(path)
    if not p.is_file():
        emit({"type": "error", "message": f"File not found: {path}"})
        return

    if MOCK:
        emit({"type": "file_done", "path": path, "text": "[mock file transcription]"})
        return

    if MODEL is None:
        emit({"type": "error", "message": "Model not loaded"})
        return

    emit({"type": "file_progress", "path": path, "percent": 10.0})
    try:
        if USE_MLX:
            result = _call_mlx_transcribe(str(p), for_file=True)
            text = _mlx_result_to_text(result, log_tag="transcribe_file_mlx")
        else:
            t_kw = build_transcribe_kwargs(for_file=True)
            hst = _w_float(CONFIG.get("whisper") or {}, "hallucination_silence_threshold", 1.6)
            try:
                segments, _ = MODEL.transcribe(
                    str(p),
                    **t_kw,
                    hallucination_silence_threshold=hst,
                )
            except TypeError:
                segments, _ = MODEL.transcribe(str(p), **t_kw)
            text = _filtered_text_from_segments(
                segments,
                log_tag="transcribe_file",
                fallback_text=None,
            )
        emit({"type": "file_progress", "path": path, "percent": 100.0})
        emit({"type": "file_done", "path": path, "text": text})
    except Exception as e:
        emit({"type": "error", "message": str(e)})


def main() -> None:
    if sys.version_info >= (3, 13):
        sys.stderr.write(
            "yapper-sidecar: Python 3.13+ often has no ctranslate2 / PyTorch wheels yet. "
            "If dictation fails, install Python 3.10–3.12, run pip install -r sidecar/requirements.txt, "
            "and set YAPPER_PYTHON to that python.exe (Windows: py -0p lists installs).\n"
        )
        sys.stderr.flush()
    vlog("sidecar main loop ready (reading stdin JSON lines)")
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        vlog(f"stdin line_len={len(line)}")
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            vlog(f"JSONDecodeError: {e} (first 120 chars: {line[:120]!r})")
            continue
        t = msg.get("type")
        vlog(f"dispatch type={t!r}")
        if t == "init":
            handle_init(msg)
        elif t == "chunk":
            handle_chunk(msg)
        elif t == "transcribe_file":
            handle_file(msg)
        elif t == "unload_model":
            handle_unload_model()
        elif t == "ensure_model":
            handle_ensure_model()
        elif t == "shutdown":
            break


if __name__ == "__main__":
    main()

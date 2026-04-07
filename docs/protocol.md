# Yapper inference protocol (`yapper.infer.v1`)

One JSON object per message. **Local sidecar:** newline-delimited JSON on stdin/stdout. **Yapper Node:** one WebSocket text frame per message (same JSON).

## Auth (remote only)

Client → server first frame after connect:

```json
{ "type": "auth", "token": "<pre-shared secret>" }
```

## Client → engine

### `init`

```json
{
  "type": "init",
  "model": "base",
  "device": "cpu",
  "compute_type": "int8",
  "model_dir": "/optional/cache/path",
  "mock": false,
  "engine": "whisper",
  "lazy_load": false,
  "whisper": {
    "beam_size": 5,
    "best_of": 1,
    "patience": 1.0,
    "temperature": 0.0,
    "no_speech_threshold": 0.78,
    "log_prob_threshold": -0.55,
    "compression_ratio_threshold": 1.9,
    "hallucination_silence_threshold": 1.6,
    "condition_on_previous_text": false,
    "initial_prompt": "",
    "language": "",
    "vad_filter_pcm": false,
    "vad_filter_file": true
  }
}
```

- `engine`: `whisper` (default) or `parakeet` (requires NeMo + CUDA on the node).
- `whisper`: optional faster-whisper decode options (desktop app sends defaults; omit for sidecar built-in defaults). Empty `language` means auto-detect. `vad_filter_pcm` is off by default for live mic; `vad_filter_file` defaults on for file jobs.

### `chunk`

PCM **s16le** mono, **base64**-encoded body.

```json
{
  "type": "chunk",
  "seq": 1,
  "sample_rate": 16000,
  "audio_b64": "...",
  "is_final": true
}
```

- `is_final`: when **`true`** (default dictation stop), the engine responds with a single **`final`** line for that `seq` (after full decode + post-filters in Python). When **`false`** (experimental live dictation), the engine responds with **`partial`** only for that `seq` — no `final` for the same message. Clients must **drain `partial` events** from their queue so they do not accumulate. The desktop app may paste each partial at the OS focus (clipboard + paste) with undo between updates; on stop it can **commit the last partial** through dictionary/tone in the app **without** sending another merged-audio `chunk` with `is_final: true` (unless live produced no text, then it falls back to a full `final` decode).

### `transcribe_file`

```json
{ "type": "transcribe_file", "path": "C:\\media\\clip.wav" }
```

Path must exist on the **inference host** (local sidecar) or files you can read on the node (remote use is limited; prefer streaming chunks from the client).

### `shutdown`

```json
{ "type": "shutdown" }
```

## Engine → client

### `ready`

```json
{ "type": "ready", "engines": ["whisper", "parakeet_stub"] }
```

### `partial` / `final`

```json
{ "type": "partial", "text": "...", "seq": 1 }
{ "type": "final", "text": "...", "seq": 1, "rtf": 0.35 }
```

- **`partial`**: best-effort text for live preview; not run through the desktop app’s dictionary/tone pipeline.
- **`final`**: committed decode for that chunk; the desktop app applies dictionary, corrections, and tone after receive.

## Future improvements (not in the wire protocol yet)

Overlapping audio windows, conditioning on prior preview text, emitting multiple partials per chunk as Whisper segments arrive, and stricter backpressure are planned to improve live preview quality and latency. True streaming ASR (separate from chunked Whisper) may use the same `partial` / `final` shapes with different engine backends.

### `file_progress` / `file_done`

```json
{ "type": "file_progress", "path": "...", "percent": 50.0 }
{ "type": "file_done", "path": "...", "text": "..." }
```

### `error`

```json
{ "type": "error", "message": "..." }
```

## Post-processing

Dictionary, corrections, and tone presets are applied in the **desktop app** after `final` / `file_done` text is received (default privacy posture).

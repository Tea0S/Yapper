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

/** Approximate on-disk cache size after download (CTranslate2 / faster-whisper). Same files for all compute types. */
export const WHISPER_MODEL_DISK_MB: Record<string, number> = {
  tiny: 75,
  base: 145,
  small: 470,
  medium: 1500,
  "large-v3": 3000,
};

/** Hugging Face MLX checkpoints — sizes mirror CT2 builds (approximate). */
export const WHISPER_MLX_MODEL_DISK_MB: Record<string, number> = {
  "mlx-community/whisper-tiny-mlx": 75,
  "mlx-community/whisper-base-mlx": 145,
  "mlx-community/whisper-small-mlx": 470,
  "mlx-community/whisper-medium-mlx": 1500,
  "mlx-community/whisper-large-v3-mlx": 3000,
  "mlx-community/whisper-large-v3-turbo": 1600,
};

/**
 * Rough peak model memory vs int8, for UI hints only.
 * float16/float32 need wider tensors at runtime even when weights on disk are quantized.
 */
export const COMPUTE_RUNTIME_MULT: Record<string, number> = {
  int8: 1,
  float16: 1.35,
  float32: 2,
};

export function whisperDiskMb(modelId: string): number {
  return (
    WHISPER_MODEL_DISK_MB[modelId] ??
    WHISPER_MLX_MODEL_DISK_MB[modelId] ??
    WHISPER_MODEL_DISK_MB.base
  );
}

/** True when the stored `whisper_model` setting is an MLX Hub repo (Apple Silicon path). */
export function isMlxWhisperModelId(modelId: string): boolean {
  return modelId.includes("mlx-community/") || modelId.endsWith("-mlx");
}

export function whisperRuntimeMbHint(modelId: string, computeType: string): number {
  const disk = whisperDiskMb(modelId);
  if (isMlxWhisperModelId(modelId)) {
    return Math.round(disk * 1.15);
  }
  const mult = COMPUTE_RUNTIME_MULT[computeType] ?? 1;
  return Math.round(disk * mult);
}

/** e.g. 145 → "~145 MB", 1500 → "~1.5 GB" */
export function formatStorageMb(mb: number): string {
  if (mb >= 1024) {
    const gb = mb / 1024;
    const rounded = gb >= 10 ? Math.round(gb) : Math.round(gb * 10) / 10;
    return `~${rounded} GB`;
  }
  return `~${Math.round(mb)} MB`;
}

export const WHISPER_MODEL_OPTIONS: {
  id: string;
  line: string;
}[] = [
  { id: "tiny", line: "Tiny — fastest, least accurate" },
  { id: "base", line: "Base — good default" },
  { id: "small", line: "Small — better accuracy" },
  { id: "medium", line: "Medium — high accuracy" },
  { id: "large-v3", line: "Large v3 — best quality" },
];

/** Apple Silicon — MLX Whisper on Metal (Hugging Face repos). */
export const WHISPER_MODEL_OPTIONS_MLX: {
  id: string;
  line: string;
}[] = [
  { id: "mlx-community/whisper-tiny-mlx", line: "Tiny — fastest, least accurate (MLX)" },
  { id: "mlx-community/whisper-base-mlx", line: "Base — good default (MLX)" },
  { id: "mlx-community/whisper-small-mlx", line: "Small — better accuracy (MLX)" },
  { id: "mlx-community/whisper-medium-mlx", line: "Medium — high accuracy (MLX)" },
  { id: "mlx-community/whisper-large-v3-turbo", line: "Large v3 Turbo — fast high quality (MLX)" },
  { id: "mlx-community/whisper-large-v3-mlx", line: "Large v3 — best quality (MLX)" },
];

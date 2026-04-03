/** Approximate on-disk cache size after download (CTranslate2 / faster-whisper). Same files for all compute types. */
export const WHISPER_MODEL_DISK_MB: Record<string, number> = {
  tiny: 75,
  base: 145,
  small: 470,
  medium: 1500,
  "large-v3": 3000,
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
  return WHISPER_MODEL_DISK_MB[modelId] ?? WHISPER_MODEL_DISK_MB.base;
}

export function whisperRuntimeMbHint(modelId: string, computeType: string): number {
  const disk = whisperDiskMb(modelId);
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

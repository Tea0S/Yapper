/** sherpa-onnx Parakeet models (ONNX, CPU or GPU). */
export const PARAKEET_MODEL_DISK_MB: Record<string, number> = {
  "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8": 670,
  "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8": 670,
  // Legacy NeMo HF IDs (migrated on load).
  "nvidia/parakeet-tdt-0.6b-v3": 670,
  "nvidia/parakeet-tdt-0.6b-v2": 670,
};

export function parakeetDiskMb(modelId: string): number {
  return PARAKEET_MODEL_DISK_MB[modelId] ?? 670;
}

export const PARAKEET_MODEL_OPTIONS: { id: string; line: string }[] = [
  {
    id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
    line: "TDT 0.6B v3 — 25 languages, punctuation built-in",
  },
  {
    id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
    line: "TDT 0.6B v2 — English only",
  },
];

export const DEFAULT_PARAKEET_MODEL = PARAKEET_MODEL_OPTIONS[0]!.id;

/** Map legacy NeMo HF model IDs to sherpa-onnx tarball names. */
export function migrateParakeetModelId(modelId: string): string {
  const legacy: Record<string, string> = {
    "nvidia/parakeet-tdt-0.6b-v3": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
    "nvidia/parakeet-tdt-0.6b-v2": "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
  };
  return legacy[modelId] ?? modelId;
}

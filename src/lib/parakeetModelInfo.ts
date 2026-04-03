/**
 * Hugging Face checkpoint IDs for NeMo Parakeet (English).
 * Disk sizes are rough one-time download ballparks (weights + typical NeMo cache); real use varies.
 */
export const PARAKEET_MODEL_DISK_MB: Record<string, number> = {
  "nvidia/parakeet-tdt-0.6b-v3": 2400,
  "nvidia/parakeet-tdt-0.6b-v2": 2400,
  "nvidia/parakeet-tdt-1.1b": 4500,
  "nvidia/parakeet-ctc-0.6b": 2300,
  "nvidia/parakeet-ctc-1.1b": 4400,
  "nvidia/parakeet-rnnt-0.6b": 2300,
  "nvidia/parakeet-rnnt-1.1b": 4400,
};

export function parakeetDiskMb(modelId: string): number {
  return PARAKEET_MODEL_DISK_MB[modelId] ?? PARAKEET_MODEL_DISK_MB["nvidia/parakeet-tdt-0.6b-v3"]!;
}

export const PARAKEET_MODEL_OPTIONS: { id: string; line: string }[] = [
  {
    id: "nvidia/parakeet-tdt-0.6b-v3",
    line: "TDT 0.6B (newest) — recommended; best balance for most setups",
  },
  {
    id: "nvidia/parakeet-tdt-0.6b-v2",
    line: "TDT 0.6B (earlier) — stable; try if the newest mishears your mic",
  },
  {
    id: "nvidia/parakeet-tdt-1.1b",
    line: "TDT 1.1B — highest-quality TDT; slower and needs more VRAM",
  },
  {
    id: "nvidia/parakeet-ctc-0.6b",
    line: "CTC 0.6B — fastest runs; great when you care about low delay",
  },
  {
    id: "nvidia/parakeet-ctc-1.1b",
    line: "CTC 1.1B — still quick; stronger than 0.6B CTC",
  },
  {
    id: "nvidia/parakeet-rnnt-0.6b",
    line: "RNN-T 0.6B — classic streaming-style; lighter on the GPU",
  },
  {
    id: "nvidia/parakeet-rnnt-1.1b",
    line: "RNN-T 1.1B — strongest RNN-T; more VRAM than 0.6B",
  },
];

export const DEFAULT_PARAKEET_MODEL = PARAKEET_MODEL_OPTIONS[0]!.id;

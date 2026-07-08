export const LIVE_STREAMING_ENGINES = [
  { id: "moonshine", line: "Moonshine — low-latency English streaming" },
  { id: "sherpa", line: "Sherpa — Parakeet Unified streaming (English)" },
] as const;

export const MOONSHINE_STREAM_MODELS = [
  { id: "tiny_streaming", line: "Tiny streaming — fastest" },
  { id: "small_streaming", line: "Small streaming — balanced (recommended)" },
  { id: "medium_streaming", line: "Medium streaming — best accuracy" },
] as const;

export const SHERPA_STREAM_MODELS = [
  {
    id: "sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-240ms",
    line: "Parakeet Unified 240 ms — lowest latency",
  },
  {
    id: "sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-560ms",
    line: "Parakeet Unified 560 ms — balanced",
  },
  {
    id: "sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-streaming-1120ms",
    line: "Parakeet Unified 1120 ms — highest accuracy",
  },
] as const;

export const DEFAULT_LIVE_STREAMING_ENGINE = "moonshine";
export const DEFAULT_LIVE_STREAMING_MODEL = "small_streaming";

export function liveStreamingModelsForEngine(engine: string) {
  return engine === "sherpa" ? SHERPA_STREAM_MODELS : MOONSHINE_STREAM_MODELS;
}

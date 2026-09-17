# Dictation improvements in 1.3

1. **Background processing:** batch recognition processes completed phrases after at least eight seconds and a sustained pause. Release waits for in-flight work and transcribes only the remaining audio. Text is inserted once. Disable this setting to use the existing experimental streaming preview instead.
2. **Speed presets:** Fast selects Whisper base/beam 1, Balanced small/beam 3, and Accurate medium/beam 5. Apple Silicon uses the corresponding MLX model (MLX does not use beam search). Presets are explicit choices; Save & restart applies them and may download a model.
3. **Readiness presets:** Ready instantly loads and warms the decoder before reporting ready and keeps it resident. Save memory uses lazy loading and five-minute idle unloading. On-demand loading also begins when recording starts if inference is available.
4. **Device benchmark:** Home benchmarks a recent spoken recording with the current engine, excluding local model loading and queue wait. It reports elapsed time and real-time factor, with conservative suggestions. Repeat after changing engines/models to compare. It does not measure recognition accuracy or claim to test alternative configurations automatically.
5. **Adaptive microphone gate:** estimates the recording's noise floor, rejects silence, and pads detected speech by 120 ms. Padding merges overlapping spans. The manual gate remains available.
6. **Recognition vocabulary:** Whisper receives up to 32 dictionary terms, ordered by priority and bounded to approximately 400 characters. Restart after editing the dictionary to refresh hints. Existing postprocessing corrections remain immediate; Parakeet continues to use those corrections.
7. **Teach a correction:** choose a recent transcript on Home, enter a phrase and replacement, and save. Existing matching correction entries are updated instead of duplicated.
8. **Recovery:** recent audio remains in RAM, with a maximum of five retained jobs. Completed jobs expire after ten minutes, checked every fifteen seconds. Active jobs remain until completion. Retry uses the current running engine and returns text to Home without automatic insertion. Discard deletes a completed job immediately.
9. **Queued recordings:** microphone capture is released before final decoding. Global push-to-talk and toggle can start another recording while older jobs finish in order. Processing is serialized with file transcription and engine changes; attempts to restart a busy engine return an actionable error.
10. **Destination-aware insertion:** on Windows, capture the application, window, native control and UI Automation element identity. Verify them before pasting. If identity cannot be verified, keep text on Home. Each destination application can opt into one multiline clipboard paste; chat-safe Shift+Enter remains the default. macOS/Linux retain their previous insertion behavior.

## Validation on a desktop

- Record at least three seconds, benchmark it, switch preset and restart, and benchmark the same retained recording. Compare both speed and transcription quality.
- Dictate two phrases separated by a pause after eight seconds. Check word boundaries and spoken punctuation against background processing disabled.
- Start a second global dictation immediately after releasing the first. Check order and that both recordings survive.
- Change text fields/windows during processing. On Windows, the result should remain on Home instead of appearing in the new destination. Test native editors and browser textareas.
- Cause an inference failure, restart, and retry from Home. Verify successful retry never auto-pastes and Discard removes the retained recording.
- Try silence, room noise and quiet speech; compare adaptive and manual gating.
- Test multiline text in a document editor and chat field with their respective insertion settings.

Automated checks cover audio gating/boundaries, vocabulary limits, recovery reservation/expiry, FIFO ordering, and eager/lazy decoder warm-up. Real microphone accuracy, GPU latency, remote inference and native insertion still need hardware validation.

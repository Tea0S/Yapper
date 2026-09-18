# Yapper 1.3.5

## Changes

- Saved speed, readiness and destination line-break choices have persistent selection indicators. Custom configurations are identified instead of showing a misleading preset.
- The Home microphone test supports click and keyboard start/stop, setup guidance, editable results and copy feedback.
- Microphone streams reopen for each recording, including after a headset reconnect or system-default change. Optional fallback preserves the preferred microphone and reports when the default is used.
- Stream errors and stalled callbacks are surfaced. Interrupted recordings retain the audio already captured and keep any resulting text on Home for review rather than automatically inserting incomplete text.
- Microphone settings scan once on entry or when Refresh microphones is selected, and preserve disconnected choices. Device enumeration runs off the UI thread. In-page settings links do not trigger rescans or reload unsaved choices. Natural, longer and extra-time pause presets control speech segmentation and background phrase boundaries. They never stop recording automatically.
- The widget includes Speak/Stop, recording level, Ready/Listening/Processing/Inserted status, live preview and an Open Yapper control. Windows sound cues are optional and off by default.
- Widget windows now fit their visible surface: 180 x 40 logical pixels idle, 240 x 52 recording, and 320 x 104 with preview or notices. Resizing preserves the bottom edge. On Windows the native window region also clips rounded corners from hit testing.
- Live preview takes precedence over background phrase processing when enabled. The first live session no longer receives the inactive zero ID. Contention no longer advances the audio cursor before audio is sent; queued preview results are drained rather than displayed one stale result at a time.
- Local model startup reports checking cached files, downloading, loading, warming and ready. Some MLX/Moonshine loaders combine download and loading into one preparation stage; no invented percentages are shown.

- Advanced engine, microphone, output, GPU setup, installation and manual shortcut controls are collapsed by default. Everyday choices and save actions stay visible.

## Validation

Automated checks cover frontend types/build, Rust unit tests including quiet interrupted speech, first streaming session IDs, startup progress isolation and stalled microphones, and Python warm-up/Unicode IPC tests using the bundled interpreter.

Desktop acceptance checks (require real hardware and applications):

1. Start and stop the Home microphone test using only the keyboard.
2. Disconnect the preferred headset before recording. Confirm the fallback notice, then reconnect and confirm the preferred device is used on the next recording. With fallback off, confirm an actionable error.
3. Unplug while recording. Stop and confirm captured text remains on Home without being inserted. Repeat with a device that stalls without an explicit driver error.
4. Dictate quiet speech with a one-second pause using Natural and Longer pauses. Verify neither loses words; pause presets do not promise a particular punctuation result.
5. Enable live preview while background processing is selected. Test the first recording after launch, a subsequent recording, and a recording while an older job finishes. A deferred/unavailable preview must not discard audio or prevent final transcription.
6. Check the widget at 100%, 150% and 200% display scaling, after dragging between monitors, in idle/recording/processing/preview/outcome states. Click just outside its surface and in its rounded corners; the app behind must receive the click.
7. Place the caret in a document, click Speak, then Stop. Verify the widget does not steal the destination, and changing destination before completion safely retains the text on Home.
8. Exercise cached-model startup, first model download, lazy loading and idle unload. Progress must reflect actual stages, and errors remain actionable.

The changes do not include general voice correction commands or automatic restart of an interrupted recording.

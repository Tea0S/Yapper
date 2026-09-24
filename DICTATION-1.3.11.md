# Yapper 1.3.11

- Resolve the dictation destination when transcription finishes instead of locking it to the app, window and field focused when recording began.
- Automatically insert into the currently focused Windows Edit or Document control, including text fields in native, browser, Office and Electron applications. Keep clipboard fallback when focus is not in a recognized text field.
- Apply multiline paste preferences to the app that actually receives the transcript and record that app on the completed dictation.

Validation: Rust unit tests and source checks, plus frontend type and accessibility checks. Native cross-application insertion still requires in-app verification.

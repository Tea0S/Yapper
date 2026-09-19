# Yapper 1.3.10

- Accept recreated accessibility objects when their field type, automation ID, class and nonempty screen bounds match. Keep app, window and native focus control checks. Missing accessibility information retains the native-control fallback. A different field in exactly the same location with identical metadata cannot be distinguished by this heuristic.
- Shorten Home, Settings, Dictionary and file-transcription copy. Keep setup requirements and useful error details; remove a competitor-name example.
- Preserve clipboard fallback when automatic insertion is unavailable.

Validation: source error checks, including field-matching test compilation. No release build or native Discord verification.

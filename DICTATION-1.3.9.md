# Yapper 1.3.9

Completed dictation falls back to the clipboard when its destination changed, could not be identified, or automatic insertion failed. Captured text from an interrupted dictation also copies to the clipboard. Home microphone tests and empty results do not overwrite it. Clipboard failures explicitly report the problem and retain the transcript on Home. Clipboard text translates spoken newline/tab markers and omits non-text editing markers.

Validation: source error checks only, no release build. Native clipboard/paste behavior requires in-app verification.

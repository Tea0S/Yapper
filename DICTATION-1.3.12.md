# Yapper 1.3.12

- Recognize writable value controls and editable legacy accessibility text roles in addition to Windows Edit, Document and TextEdit controls.
- Use the focused Windows typing caret when accessibility information is missing or reports only a generic container. Reject known non-text, disabled and explicitly read-only controls.
- Continue resolving the destination when transcription finishes, so dictation follows the currently focused text field across applications.

Validation: focused-field regression tests and a local Windows installer build passed. Real application compatibility still depends on the focus information exposed by each editor.

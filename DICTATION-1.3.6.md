# Yapper 1.3.6

- Restore the classic compact pill as the default. Choose Classic pill or Speak/Stop controls under Desktop widget. Both keep tight native window bounds.
- Initialize settings on component mount, independently of navigation callbacks, GPU detection and microphone enumeration. Display loading and load errors, with retry; block editing until saved values load.
- Preserve saved shortcuts and only write bindings the user changes. Report save and registration failures. Saving output style no longer writes unrelated engine options.
- Distinguish an unfinished or failed NVIDIA check from a negative result. Run the driver check on a blocking worker and enumerate microphones once on entry or manual refresh.
- Allow automatic paste when the same window, control and application remain selected but accessibility IDs are unavailable. When both IDs exist, changed fields still block insertion.

Validation: frontend and Rust error checks only; no installer or release build. Native microphone, widget and paste behavior still needs in-app verification.

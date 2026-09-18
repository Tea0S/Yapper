# Yapper 1.3.7

Correct the native Windows HUD frame before applying its rounded region. The window library retains caption styles on undecorated windows; explicit removal prevents the caption, icon and close button from appearing inside the compact pill. Preserve both widget styles, tight bounds, rounded hit testing and non-activating behavior. Repair caption flags if a visibility or topmost update restores them.

Validation: Rust and frontend source error checks; no release build. Native rendering still requires verification in the updated app.

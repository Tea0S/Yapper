# Yapper 1.3.8

Remove custom Windows region clipping and caption-style rewrites from the HUD. Return to Tauri-managed transparent, undecorated rendering. Leave a one-pixel transparent gutter for smooth CSS capsule edges, restore meter spacing and compact preview width, and remove size/padding transitions during native resizing. Classic hover hints include the saved shortcuts without reserving a large invisible tooltip area.

The native bounds remain tight; the one-pixel gutter and tiny transparent corners belong to the widget window. This patch does not claim pixel-perfect corner click-through.

Validation: source error checks only. No local release build; native appearance and transitions still require verification in the updated app.

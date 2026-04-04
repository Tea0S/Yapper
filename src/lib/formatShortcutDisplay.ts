/**
 * Pretty-print Tauri global-shortcut strings (from `keybindCapture` / DB) for UI.
 * On macOS, uses standard symbol labels (⌘⌃⌥⇧); elsewhere Ctrl/Win/Alt/Shift.
 */
export function formatShortcutDisplay(
  raw: string,
  opts?: { mac?: boolean },
): string {
  const mac = opts?.mac ?? false;
  const sep = mac ? "" : " + ";
  return raw
    .split("+")
    .map((t) => {
      const u = t.trim();
      const low = u.toLowerCase();
      if (low.startsWith("digit") && u.length > 5) return u.slice(5);
      if (low.startsWith("key") && u.length > 3) return u.slice(3).toUpperCase();
      if (mac) {
        if (low === "control") return "⌃";
        if (low === "super") return "⌘";
        if (low === "alt") return "⌥";
        if (low === "shift") return "⇧";
      } else {
        if (low === "control") return "Ctrl";
        if (low === "super") return "Win";
        if (low === "alt") return "Alt";
        if (low === "shift") return "Shift";
      }
      return t.trim();
    })
    .join(sep);
}

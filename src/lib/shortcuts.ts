import { invoke } from "@tauri-apps/api/core";

export type StatusFn = (s: string) => void;

/**
 * Registers global shortcuts from the SQLite keybind table using a **native Rust** handler.
 * (The JS `Channel` path often misses events while the main WebView is unfocused — typical for PTT.)
 */
export async function bindYapperShortcuts(onStatus?: StatusFn) {
  const st = onStatus ?? (() => {});
  try {
    st(await invoke<string>("refresh_global_shortcuts"));
  } catch (e) {
    const msg = `Global shortcuts failed: ${e}`;
    console.warn(msg);
    st(msg);
  }
}

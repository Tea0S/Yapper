import { invoke } from "@tauri-apps/api/core";
import { isRegistered, register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";

export type StatusFn = (s: string) => void;

/** IPC may send "Pressed" / "Released" or other casings depending on serde / bridge. */
function shortcutState(eventState: string): "pressed" | "released" | "other" {
  const s = eventState.toLowerCase();
  if (s === "pressed") return "pressed";
  if (s === "released") return "released";
  return "other";
}

export async function bindYapperShortcuts(onStatus?: StatusFn) {
  const st = onStatus ?? (() => {});

  /** Ensures `ptt_stop` IPC runs after `ptt_start` completes (tap-release can reorder otherwise). */
  let pttStartPromise: Promise<void> | null = null;

  async function handleAction(action: string, stateRaw: string) {
    const phase = shortcutState(stateRaw);
    if (action === "push_to_talk") {
      if (phase === "pressed") {
        pttStartPromise = invoke("ptt_start")
          .then(() => {
            st("Recording…");
          })
          .catch((e) => {
            pttStartPromise = null;
            st(String(e));
          });
      } else if (phase === "released") {
        const startP = pttStartPromise;
        pttStartPromise = null;
        try {
          if (startP) await startP.catch(() => {});
          const text = await invoke<string>("ptt_stop");
          st(text ? "Done." : "No speech captured.");
          if (text) await invoke("paste_text", { text });
        } catch (e) {
          st(String(e));
        }
      }
      return;
    }
    if (phase !== "pressed") return;
    if (action === "toggle_open_mic") {
      st("Open-mic mode is planned; use push-to-talk for now.");
      return;
    }
    if (action === "stop_dictation") {
      st("Stopped.");
    }
  }

  await unregisterAll();
  const rows = await invoke<{ action: string; shortcut: string }[]>(
    "list_keybinds_cmd",
  );

  const trimmed = rows
    .map((r) => ({
      action: r.action,
      shortcut: r.shortcut.trim(),
    }))
    .filter((r) => r.shortcut.length > 0);

  if (trimmed.length === 0) {
    st("No keybinds found — open Settings and save keybinds.");
    return;
  }

  /** Canonical shortcut string from the plugin (e.g. control+shift+Space) vs DB casing. */
  const actionByNormalized = new Map<string, string>();
  for (const r of trimmed) {
    actionByNormalized.set(normalizeShortcutKey(r.shortcut), r.action);
  }

  const shortcutStrings = trimmed.map((r) => r.shortcut);

  try {
    await register(shortcutStrings, (event) => {
      const action = actionByNormalized.get(normalizeShortcutKey(event.shortcut));
      if (!action) return;
      void handleAction(action, event.state);
    });
  } catch (e) {
    const msg = `Global shortcuts failed: ${e}`;
    console.warn(msg);
    st(msg);
    return;
  }

  const ptt = trimmed.find((r) => r.action === "push_to_talk");
  if (ptt) {
    const ok = await isRegistered(ptt.shortcut).catch(() => false);
    if (!ok) {
      st(
        `Shortcut "${ptt.shortcut}" may be owned by another app — try a different combo in Settings.`,
      );
    } else {
      st("Shortcuts ready — hold push-to-talk when the engine is running.");
    }
  }
}

/** Same order as global-hotkey `HotKey::into_string` (shift → control → alt → super), then main key. */
const MODIFIER_ORDER = ["shift", "control", "alt", "super", "hyper"] as const;

function normalizeShortcutKey(s: string): string {
  const raw = s
    .split("+")
    .map((t) => t.trim().replace(/\s+/g, ""))
    .filter(Boolean);
  const mods = new Set<string>();
  let mainKey = "";
  for (const token of raw) {
    const n = normalizeShortcutToken(token);
    if ((MODIFIER_ORDER as readonly string[]).includes(n)) {
      mods.add(n);
    } else {
      mainKey = n;
    }
  }
  if (!mainKey) return "";
  const orderedMods = [...mods].sort(
    (a, b) =>
      (MODIFIER_ORDER as readonly string[]).indexOf(a) -
      (MODIFIER_ORDER as readonly string[]).indexOf(b),
  );
  return [...orderedMods, mainKey].join("+");
}

/**
 * Align user-entered shortcuts with plugin event strings (see global-hotkey `into_string`:
 * always `control+` / `shift+` / `alt+`, never `ctrl+`).
 */
function normalizeShortcutToken(t: string): string {
  const u = t.toLowerCase();
  if (u === "ctrl" || u === "control") return "control";
  if (u === "cmd" || u === "command" || u === "super" || u === "win" || u === "meta")
    return "super";
  if (u === "option" || u === "alt") return "alt";
  if (u === "shift") return "shift";
  if (
    u === "commandorcontrol" ||
    u === "commandorctrl" ||
    u === "cmdorctrl" ||
    u === "cmdorcontrol"
  )
    return "control";
  if (/^key[a-z]$/i.test(u)) return u.toLowerCase();
  if (/^digit\d$/i.test(u)) return u.toLowerCase();
  const fn = /^f(\d{1,2})$/i.exec(u);
  if (fn) return `F${fn[1]}`;
  if (/^\d$/.test(u)) return `digit${u}`;
  if (/^[a-z]$/.test(u)) return `key${u}`;
  // DOM `event.code` is already PascalCase when captured from the browser.
  if (/^[A-Z][a-zA-Z0-9]*$/.test(t.trim())) return t.trim();
  const tailPascal = (rest: string) =>
    rest ? rest.charAt(0).toUpperCase() + rest.slice(1).toLowerCase() : "";
  if (u.startsWith("arrow") && u.length > 5) return "Arrow" + tailPascal(u.slice(5));
  if (u.startsWith("numpad") && u.length > 6) return "Numpad" + tailPascal(u.slice(6));
  if (/^[a-z][a-z0-9]*$/i.test(u)) {
    return u.charAt(0).toUpperCase() + u.slice(1).toLowerCase();
  }
  return u;
}

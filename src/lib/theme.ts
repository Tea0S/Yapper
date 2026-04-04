import { invoke } from "@tauri-apps/api/core";
import { setTheme as setNativeTheme } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type UiTheme = "system" | "light" | "dark";

function isHudRoute(): boolean {
  return typeof window !== "undefined" && window.location.pathname === "/hud";
}

/**
 * Match app shell to theme immediately (avoids white flash on load / theme toggle).
 * See PR #1 commit dcd7c53 — inline `app.html` defaults + native window color + body fill.
 */
export function applyUiTheme(mode: UiTheme) {
  const root = document.documentElement;
  if (mode === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", mode);
  }
  const resolved = resolveUiTheme(mode);
  root.style.colorScheme = resolved;
  if (!isHudRoute()) {
    document.body.style.backgroundColor = resolved === "light" ? "#f4f2ee" : "#0e1114";
    void syncWindowBackground(resolved);
  }
  void syncNativeTheme(mode);
}

function resolveUiTheme(mode: UiTheme): "light" | "dark" {
  if (mode !== "system") return mode;
  if (typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: light)").matches) {
    return "light";
  }
  return "dark";
}

async function syncNativeTheme(mode: UiTheme) {
  try {
    if (mode === "system") {
      await setNativeTheme(null);
    } else {
      await setNativeTheme(mode);
    }
  } catch {
    /* Web dev or restricted context */
  }
}

async function syncWindowBackground(mode: "light" | "dark") {
  try {
    await getCurrentWindow().setBackgroundColor(mode === "light" ? "#f4f2ee" : "#0e1114");
  } catch {
    /* Browser without Tauri or unsupported platform */
  }
}

export async function loadUiTheme(): Promise<UiTheme> {
  try {
    const v = await invoke<string | null>("get_setting_cmd", { key: "ui_theme" });
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* Browser without Tauri */
  }
  return "system";
}

export async function persistUiTheme(mode: UiTheme) {
  try {
    await invoke("set_setting_cmd", { key: "ui_theme", value: mode });
  } catch {
    /* Browser without Tauri */
  }
}

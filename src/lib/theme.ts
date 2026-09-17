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
export function applyUiTheme(mode: UiTheme, highContrast = false) {
  const root = document.documentElement;
  if (mode === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", mode);
  }
  if (highContrast) {
    root.setAttribute("data-contrast", "high");
  } else {
    root.removeAttribute("data-contrast");
  }
  const resolved = resolveUiTheme(mode);
  root.style.colorScheme = resolved;
  if (!isHudRoute()) {
    const bg = highContrast
      ? resolved === "light"
        ? "#ffffff"
        : "#000000"
      : resolved === "light"
        ? "#f4f2ee"
        : "#0e1114";
    document.body.style.backgroundColor = bg;
    void syncWindowBackground(resolved, highContrast);
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

async function syncWindowBackground(mode: "light" | "dark", highContrast: boolean) {
  try {
    const color = highContrast
      ? mode === "light"
        ? "#ffffff"
        : "#000000"
      : mode === "light"
        ? "#f4f2ee"
        : "#0e1114";
    await getCurrentWindow().setBackgroundColor(color);
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

export async function loadHighContrast(): Promise<boolean> {
  try {
    return (await invoke<string | null>("get_setting_cmd", { key: "ui_high_contrast" })) === "true";
  } catch {
    return false;
  }
}

export async function persistUiTheme(mode: UiTheme) {
  try {
    await invoke("set_setting_cmd", { key: "ui_theme", value: mode });
  } catch {
    /* Browser without Tauri */
  }
}

export async function persistHighContrast(on: boolean) {
  try {
    await invoke("set_setting_cmd", {
      key: "ui_high_contrast",
      value: on ? "true" : "false",
    });
  } catch {
    /* Browser without Tauri */
  }
}

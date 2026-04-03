import { invoke } from "@tauri-apps/api/core";
import { setTheme as setNativeTheme } from "@tauri-apps/api/app";

export type UiTheme = "system" | "light" | "dark";

export function applyUiTheme(mode: UiTheme) {
  const root = document.documentElement;
  if (mode === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", mode);
  }
  void syncNativeTheme(mode);
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

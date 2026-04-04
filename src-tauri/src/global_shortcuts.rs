//! Global shortcuts are registered here (Rust) so hotkey events are handled on the native
//! path. The JS `Channel` used by `@tauri-apps/plugin-global-shortcut` is often not delivered
//! when the main WebView2 window is unfocused — which is exactly when users expect PTT to work.

use crate::db::list_keybinds;
use crate::state::AppState;
use crate::trace_log::shortcut_log;
use crate::{open_db, ptt_start_inner, ptt_stop_inner};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Await any in-flight `ptt_start_inner` from a hotkey, then stop and paste (same as PTT release).
fn spawn_ptt_stop_after_pending(app: AppHandle, label: &'static str) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let pending = state.ptt_hotkey_start_pending.clone();
        let start_h = pending.lock().ok().and_then(|mut g| g.take());
        if let Some(jh) = start_h {
            let _ = jh.await;
        }
        match ptt_stop_inner(&app, &*state).await {
            Ok(text) => {
                shortcut_log(format!(
                    "{label}: stop ok pasted_non_empty={} chars={}",
                    !text.is_empty(),
                    text.len()
                ));
                if !text.is_empty() {
                    // Main thread + correct modifier (⌘ on macOS, Ctrl elsewhere) — see paste.rs.
                    let _ = crate::paste::paste_text_at_focus_on_main_thread(&app, text);
                }
            }
            Err(e) => shortcut_log(format!("{label}: stop failed: {e}")),
        }
    });
}

pub(crate) fn refresh(app: &AppHandle) -> Result<String, String> {
    let conn = open_db(app)?;
    let rows = list_keybinds(&conn).map_err(|e| e.to_string())?;

    let mut id_to_action: HashMap<u32, String> = HashMap::new();
    let mut hotkeys: Vec<Shortcut> = Vec::new();

    for r in rows {
        let s = r.shortcut.trim();
        if s.is_empty() {
            continue;
        }
        let hk: Shortcut = s
            .parse()
            .map_err(|e| format!("Invalid shortcut {s:?}: {e}"))?;
        if let Some(prev) = id_to_action.insert(hk.id(), r.action.clone()) {
            if prev != r.action {
                eprintln!(
                    "[yapper] keybind conflict: shortcuts for {prev:?} and {:?} registered the same global id — only {:?} will receive events. Reassign one in Settings.",
                    r.action, r.action
                );
            }
        }
        hotkeys.push(hk);
    }

    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;

    if hotkeys.is_empty() {
        return Ok("No keybinds found — open Settings and save keybinds.".into());
    }

    let has_ptt = id_to_action.values().any(|a| a == "push_to_talk");
    let has_toggle = id_to_action.values().any(|a| a == "toggle_open_mic");
    let id_to_action = std::sync::Arc::new(id_to_action);

    gs.on_shortcuts(hotkeys, move |app_handle, _shortcut, event| {
        let Some(action) = id_to_action.get(&event.id) else {
            return;
        };
        match (action.as_str(), event.state) {
            ("push_to_talk", ShortcutState::Pressed) => {
                shortcut_log("push_to_talk: Pressed");
                let app = app_handle.clone();
                let pending = app.state::<AppState>().ptt_hotkey_start_pending.clone();
                let h = tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let pending_slot = state.ptt_hotkey_start_pending.clone();
                    let r = ptt_start_inner(&app, &*state).await;
                    if r.is_err() {
                        let mut g = pending_slot.lock().unwrap_or_else(|e| e.into_inner());
                        *g = None;
                    }
                    r
                });
                {
                    let mut g = pending.lock().unwrap_or_else(|e| e.into_inner());
                    *g = Some(h);
                }
            }
            ("push_to_talk", ShortcutState::Released) => {
                shortcut_log("push_to_talk: Released → stop + paste");
                spawn_ptt_stop_after_pending(app_handle.clone(), "push_to_talk");
            }
            ("toggle_open_mic", ShortcutState::Pressed) => {
                let app = app_handle.clone();
                let state = app_handle.state::<AppState>();
                let active = state.ptt_session_active.load(Ordering::SeqCst);
                let has_pending = state
                    .ptt_hotkey_start_pending
                    .lock()
                    .map(|g| g.is_some())
                    .unwrap_or(false);
                shortcut_log(format!(
                    "toggle_open_mic: Pressed active={active} pending_start={has_pending}"
                ));
                if active || has_pending {
                    shortcut_log("toggle_open_mic: → stop + paste");
                    spawn_ptt_stop_after_pending(app, "toggle_open_mic");
                } else {
                    shortcut_log("toggle_open_mic: → start capture");
                    let pending = state.ptt_hotkey_start_pending.clone();
                    let app_spawn = app.clone();
                    let h = tauri::async_runtime::spawn(async move {
                        let state = app_spawn.state::<AppState>();
                        let pending_slot = state.ptt_hotkey_start_pending.clone();
                        let r = ptt_start_inner(&app_spawn, &*state).await;
                        if let Err(e) = &r {
                            shortcut_log(format!("toggle_open_mic: start failed: {e}"));
                        }
                        if r.is_err() {
                            let mut g = pending_slot.lock().unwrap_or_else(|e| e.into_inner());
                            *g = None;
                        }
                        r
                    });
                    {
                        let mut g = pending.lock().unwrap_or_else(|e| e.into_inner());
                        *g = Some(h);
                    }
                }
            }
            ("stop_dictation", ShortcutState::Pressed) => {
                let app = app_handle.clone();
                let state = app_handle.state::<AppState>();
                let active = state.ptt_session_active.load(Ordering::SeqCst);
                let has_pending = state
                    .ptt_hotkey_start_pending
                    .lock()
                    .map(|g| g.is_some())
                    .unwrap_or(false);
                shortcut_log(format!(
                    "stop_dictation: Pressed active={active} pending_start={has_pending}"
                ));
                if active || has_pending {
                    spawn_ptt_stop_after_pending(app, "stop_dictation");
                }
            }
            // Toggle / stop use Pressed only; Released still fires for the same binding — ignore.
            _ => {}
        }
    })
    .map_err(|e| e.to_string())?;

    let msg = match (has_ptt, has_toggle) {
        (true, true) => "Shortcuts ready — hold PTT or use toggle open mic (engine must be running).",
        (true, false) => "Shortcuts ready — hold push-to-talk when the engine is running.",
        (false, true) => "Shortcuts ready — toggle open mic when the engine is running.",
        (false, false) => "Shortcuts registered.",
    };
    Ok(msg.into())
}

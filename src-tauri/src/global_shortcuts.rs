//! Global shortcuts are registered here (Rust) so hotkey events are handled on the native
//! path. The JS `Channel` used by `@tauri-apps/plugin-global-shortcut` is often not delivered
//! when the main WebView2 window is unfocused — which is exactly when users expect PTT to work.

use crate::db::list_keybinds;
use crate::state::AppState;
use crate::{open_db, ptt_start_inner, ptt_stop_inner};
use std::collections::HashMap;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

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
        id_to_action.insert(hk.id(), r.action);
        hotkeys.push(hk);
    }

    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;

    if hotkeys.is_empty() {
        return Ok("No keybinds found — open Settings and save keybinds.".into());
    }

    let has_ptt = id_to_action.values().any(|a| a == "push_to_talk");
    let id_to_action = std::sync::Arc::new(id_to_action);

    gs.on_shortcuts(hotkeys, move |app_handle, _shortcut, event| {
        let Some(action) = id_to_action.get(&event.id) else {
            return;
        };
        match (action.as_str(), event.state) {
            ("push_to_talk", ShortcutState::Pressed) => {
                let app = app_handle.clone();
                let pending = app.state::<AppState>().ptt_hotkey_start_pending.clone();
                let h = tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    ptt_start_inner(&app, &*state).await
                });
                {
                    let mut g = pending.lock().unwrap_or_else(|e| e.into_inner());
                    *g = Some(h);
                }
            }
            ("push_to_talk", ShortcutState::Released) => {
                let app = app_handle.clone();
                let pending = app.state::<AppState>().ptt_hotkey_start_pending.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    let start_h = pending.lock().ok().and_then(|mut g| g.take());
                    if let Some(jh) = start_h {
                        let _ = jh.await;
                    }
                    match ptt_stop_inner(&app, &*state).await {
                        Ok(text) => {
                            if !text.is_empty() {
                                let _ = crate::paste::paste_text_at_focus(&text);
                            }
                        }
                        Err(_) => {}
                    }
                });
            }
            _ => {}
        }
    })
    .map_err(|e| e.to_string())?;

    if has_ptt {
        Ok("Shortcuts ready — hold push-to-talk when the engine is running.".into())
    } else {
        Ok("Shortcuts registered.".into())
    }
}

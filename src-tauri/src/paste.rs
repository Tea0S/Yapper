use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::mpsc;

struct PasteTarget {
    pub app_key: String,
    pub is_text_field: bool,
}

fn is_text_control_type(control_type: i32) -> bool {
    // UI Automation Edit and Document controls cover native inputs, browser
    // textareas/contenteditable regions, office editors and Electron fields.
    matches!(control_type, 50004 | 50030)
}

#[cfg(windows)]
fn focused_element_is_text_field() -> Option<bool> {
    use windows::Win32::{System::Com::*, UI::Accessibility::*};
    use windows::Win32::UI::WindowsAndMessaging::STATE_SYSTEM_READONLY;
    use windows::core::Interface;
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| {
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(timed) = automation.cast::<IUIAutomation2>() {
                let _ = timed.SetConnectionTimeout(300);
                let _ = timed.SetTransactionTimeout(300);
            }
            let element = automation.GetFocusedElement().ok()?;
            if element.CurrentIsEnabled().ok().is_some_and(|enabled| !enabled.as_bool()) {
                return Some(false);
            }
            let value = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok().and_then(|pattern| pattern.CurrentIsReadOnly().ok())
                .map(|readonly| readonly.as_bool());
            if value == Some(true) { return Some(false); }
            // Chromium/Electron and older native editors may expose editable
            // fields through the legacy role instead of Edit/TextEdit.
            let legacy = element.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId).ok();
            let legacy_text = legacy.as_ref()
                .and_then(|pattern| pattern.CurrentRole().ok()) == Some(ROLE_SYSTEM_TEXT);
            let legacy_state = legacy.as_ref().and_then(|pattern| pattern.CurrentState().ok());
            if legacy_text && legacy_state.is_some_and(|state| state & (STATE_SYSTEM_READONLY | 1) != 0) {
                return Some(false);
            }
            let control_type = element.CurrentControlType().ok().map(|kind| kind.0);
            let text = control_type.is_some_and(is_text_control_type)
                || value == Some(false)
                || element.GetCurrentPattern(UIA_TextEditPatternId).is_ok()
                || (legacy_text && legacy_state.is_some());
            // An unrecognized type is inconclusive: some editors expose only
            // their containing pane. Let a native caret establish text focus.
            if text { return Some(true); }
            // A button, menu, or other known non-text control is a definite
            // rejection. Only generic containers/text need the caret fallback.
            match control_type {
                None | Some(50020 | 50025 | 50026 | 50032 | 50033) => None,
                _ => Some(false),
            }
        })();
        if initialized { CoUninitialize(); }
        result
    }
}

fn accepts_text_focus(accessibility: Option<bool>, native_caret: bool) -> bool {
    accessibility.unwrap_or(native_caret)
}

fn capture_target() -> Option<PasteTarget> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, GetGUIThreadInfo, GUITHREADINFO};
        use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
        use windows::Win32::Foundation::CloseHandle;
        let window = GetForegroundWindow();
        if window.0.is_null() { return None; }
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(window, Some(&mut pid));
        let accessibility = focused_element_is_text_field();
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        let native_caret = GetGUIThreadInfo(thread, &mut info).is_ok()
            && !info.hwndFocus.0.is_null()
            && !info.hwndCaret.0.is_null()
            && info.rcCaret.bottom > info.rcCaret.top;
        // Avoid combining app settings and field information across a switch
        // that happens while the accessibility provider answers.
        if GetForegroundWindow() != window { return None; }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0u16; 32768];
        let mut len = path.len() as u32;
        let found = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(path.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        found.ok()?;
        let app_key = String::from_utf16_lossy(&path[..len as usize]).to_lowercase();
        return Some(PasteTarget {
            app_key,
            is_text_field: accepts_text_focus(accessibility, native_caret),
        });
    }
    #[cfg(not(windows))]
    { None }
}

/// Insert into the text field focused when transcription finishes. Never steal focus.
/// Resolve the destination here rather than when recording starts so switching apps
/// between dictations does not leave a valid text field looking stale.
pub fn paste_to_focused_text_field(app: &tauri::AppHandle, text: String) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let app_copy = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| {
            let current = capture_target().ok_or("Could not find the text field.")?;
            if !current.is_text_field {
                return Err("The focused control is not a text field.".to_string());
            }
            let conn = crate::open_db(&app_copy)?;
            let key = format!("paste_multiline_app_{}", current.app_key);
            let multiline = crate::db::get_setting(&conn, &key).map_err(|e| e.to_string())?.as_deref() == Some("true");
            if multiline && !text_has_spoken_key_sentinels(&text) {
                let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
                let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
                paste_clipboard_chunk(&mut cb, &mut enigo, &text)?;
            } else { paste_text_at_focus(&text)?; }
            Ok(current.app_key)
        })();
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// Copy completed dictation without typing into an application.
pub fn copy_dictation(text: &str) -> Result<(), String> {
    // Spoken editing commands are internal key markers, not clipboard characters.
    let plain: String = text.chars().filter_map(|c| match c {
        '\u{E090}' => Some('\n'),
        '\u{E092}' => Some('\t'),
        '\u{E091}' | '\u{E093}' | '\u{E094}' => None,
        _ => Some(c),
    }).collect();
    Clipboard::new().map_err(|e| e.to_string())?
        .set_text(plain).map_err(|e| e.to_string())
}

fn text_has_spoken_key_sentinels(s: &str) -> bool {
    s.chars().any(|c| sentinel_to_key(c).is_some())
}

fn sentinel_to_key(ch: char) -> Option<Key> {
    match ch {
        '\u{E090}' => Some(Key::Return),
        '\u{E091}' => Some(Key::CapsLock),
        '\u{E092}' => Some(Key::Tab),
        '\u{E093}' => Some(Key::Escape),
        '\u{E094}' => Some(Key::Backspace),
        _ => None,
    }
}

fn key_click(enigo: &mut Enigo, key: Key) -> Result<(), String> {
    enigo
        .key(key, Direction::Click)
        .map_err(|e| e.to_string())
}

/// Paste a block that may contain `\n` (Shift+Return between lines). No key sentinels inside.
fn paste_plain_block(cb: &mut Clipboard, enigo: &mut Enigo, block: &str) -> Result<u32, String> {
    if block.is_empty() {
        return Ok(0);
    }
    if !block.contains('\n') {
        paste_clipboard_chunk(cb, enigo, block)?;
        return Ok(1);
    }

    let lines: Vec<&str> = block.split('\n').collect();
    let n = lines.len();
    let mut undo_ops: u32 = 0;

    for (i, part) in lines.iter().enumerate() {
        if !part.is_empty() {
            paste_clipboard_chunk(cb, enigo, part)?;
            undo_ops = undo_ops.saturating_add(1);
        }
        if i + 1 < n {
            shift_return(enigo)?;
            undo_ops = undo_ops.saturating_add(1);
        }
    }

    Ok(undo_ops)
}

#[cfg(target_os = "macos")]
fn paste_modifier() -> Key {
    Key::Meta
}

#[cfg(not(target_os = "macos"))]
fn paste_modifier() -> Key {
    Key::Control
}

fn paste_clipboard_chunk(cb: &mut Clipboard, enigo: &mut Enigo, chunk: &str) -> Result<(), String> {
    cb.set_text(chunk).map_err(|e| e.to_string())?;
    let modifier = paste_modifier();
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Shift+Return / Shift+Enter — line break in chat-style fields that strip pasted `\n`.
fn shift_return(enigo: &mut Enigo) -> Result<(), String> {
    enigo
        .key(Key::Shift, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Return, Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Shift, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Pastes into the focused control. Returns how many discrete undo steps this used (for live dictation).
pub fn paste_text_at_focus(text: &str) -> Result<u32, String> {
    let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;

    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.is_empty() {
        return Ok(0);
    }

    if !text_has_spoken_key_sentinels(&normalized) {
        return paste_plain_block(&mut cb, &mut enigo, &normalized);
    }

    let mut undo_ops: u32 = 0;
    let mut buf = String::new();
    for ch in normalized.chars() {
        if let Some(key) = sentinel_to_key(ch) {
            undo_ops = undo_ops.saturating_add(paste_plain_block(&mut cb, &mut enigo, &buf)?);
            buf.clear();
            key_click(&mut enigo, key)?;
            undo_ops = undo_ops.saturating_add(1);
        } else {
            buf.push(ch);
        }
    }
    undo_ops = undo_ops.saturating_add(paste_plain_block(&mut cb, &mut enigo, &buf)?);
    Ok(undo_ops)
}

pub fn paste_text_at_focus_on_main_thread(
    app: &tauri::AppHandle,
    text: String,
) -> Result<u32, String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(paste_text_at_focus(&text));
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// One undo (⌘Z / Ctrl+Z) at the focused control — used to replace live dictation text.
fn undo_once_at_focus() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let modifier = paste_modifier();
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode('z'), Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn undo_n_times_at_focus(n: u32) -> Result<(), String> {
    let n = n.min(64);
    for _ in 0..n {
        undo_once_at_focus()?;
    }
    Ok(())
}

pub fn undo_n_times_at_focus_on_main_thread(app: &tauri::AppHandle, n: u32) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(undo_n_times_at_focus(n));
    })
    .map_err(|e| e.to_string())?;
    let inner = rx.recv().map_err(|e| e.to_string())?;
    inner
}

/// Same as [`paste_text_at_focus_on_main_thread`] but safe to `.await` from async tasks (uses the blocking pool).
pub async fn paste_text_at_focus_spawn(app: &tauri::AppHandle, text: String) -> Result<u32, String> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || paste_text_at_focus_on_main_thread(&app, text))
        .await
        .map_err(|e| format!("paste spawn_blocking: {e}"))?
}

pub async fn undo_n_times_at_focus_spawn(app: &tauri::AppHandle, n: u32) -> Result<(), String> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || undo_n_times_at_focus_on_main_thread(&app, n))
        .await
        .map_err(|e| format!("undo spawn_blocking: {e}"))?
}

#[cfg(test)]
mod field_tests {
    use super::*;
    #[test]
    fn only_edit_and_document_controls_accept_dictation() {
        assert!(is_text_control_type(50004));
        assert!(is_text_control_type(50030));
        assert!(!is_text_control_type(50000));
        assert!(!is_text_control_type(50005));
    }
    #[test]
    fn native_caret_recovers_missing_accessibility_information() {
        assert!(accepts_text_focus(None, true));
        assert!(!accepts_text_focus(None, false));
    }
    #[test]
    fn disabled_or_readonly_field_cannot_be_overridden_by_a_caret() {
        assert!(!accepts_text_focus(Some(false), true));
        assert!(accepts_text_focus(Some(true), false));
    }
}

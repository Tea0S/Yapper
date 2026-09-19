use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::mpsc;

#[derive(Clone)]
pub struct PasteTarget {
    pub window: isize,
    pub control: isize,
    pub app_key: String,
    pub element: Option<FocusedField>,
}

#[derive(Clone)]
pub struct FocusedField {
    runtime_id: Vec<i32>,
    automation_id: String,
    class_name: String,
    control_type: i32,
    bounds: [i32; 4],
}

impl FocusedField {
    fn same_field(&self, other: &Self) -> bool {
        if !self.runtime_id.is_empty() && self.runtime_id == other.runtime_id { return true; }
        // Electron can recreate the accessibility object while the input stays put.
        // Do not compare its text/name: those change as the user types.
        self.control_type == other.control_type
            && self.automation_id == other.automation_id
            && self.class_name == other.class_name
            && (!self.class_name.is_empty() || !self.automation_id.is_empty())
            && self.bounds[2] > self.bounds[0] && self.bounds[3] > self.bounds[1]
            && self.bounds == other.bounds
    }
}

#[cfg(windows)]
fn focused_element_id() -> Option<FocusedField> {
    use windows::Win32::{System::{Com::*, Ole::*}, UI::Accessibility::*};
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
            let rect = element.CurrentBoundingRectangle().ok()?;
            let identity = FocusedField {
                runtime_id: Vec::new(),
                automation_id: element.CurrentAutomationId().ok()?.to_string(),
                class_name: element.CurrentClassName().ok()?.to_string(),
                control_type: element.CurrentControlType().ok()?.0,
                bounds: [rect.left, rect.top, rect.right, rect.bottom],
            };
            let array = element.GetRuntimeId().ok()?;
            if array.is_null() { return None; }
            let result = (|| {
                let lo = SafeArrayGetLBound(array, 1).ok()?;
                let hi = SafeArrayGetUBound(array, 1).ok()?;
                if hi < lo || hi as i64 - lo as i64 > 128 { return None; }
                let mut ids = Vec::new();
                for i in lo..=hi {
                    let mut id: i32 = 0;
                    SafeArrayGetElement(array, &i, (&mut id as *mut i32).cast()).ok()?;
                    ids.push(id);
                }
                Some(ids)
            })();
            let _ = SafeArrayDestroy(array);
            result.map(|runtime_id| FocusedField { runtime_id, ..identity })
        })();
        if initialized { CoUninitialize(); }
        result
    }
}

pub fn capture_target() -> Option<PasteTarget> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO};
        use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
        use windows::Win32::Foundation::CloseHandle;
        let window = GetForegroundWindow();
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if window.0.is_null() || GetGUIThreadInfo(0, &mut info).is_err() { return None; }
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0u16; 32768];
        let mut len = path.len() as u32;
        let found = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(path.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        found.ok()?;
        let app_key = String::from_utf16_lossy(&path[..len as usize]).to_lowercase();
        return Some(PasteTarget { window: window.0 as isize, control: info.hwndFocus.0 as isize, app_key, element: focused_element_id() });
    }
    #[cfg(not(windows))]
    { None }
}

/// Validate immediately before insertion on the main thread. Never steal focus.
pub fn paste_to_target(app: &tauri::AppHandle, target: PasteTarget, text: String) -> Result<(), String> {
    let conn = crate::open_db(app)?;
    let key = format!("paste_multiline_app_{}", target.app_key);
    let multiline = crate::db::get_setting(&conn, &key).map_err(|e| e.to_string())?.as_deref() == Some("true");
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            let current = capture_target().ok_or("Could not find the text field.")?;
            if current.window != target.window || current.control != target.control || current.app_key != target.app_key
                || matches!((&target.element, &current.element), (Some(expected), Some(actual)) if !expected.same_field(actual)) {
                return Err("The active text field changed.".to_string());
            }
            if multiline && !text_has_spoken_key_sentinels(&text) {
                let mut cb = Clipboard::new().map_err(|e| e.to_string())?;
                let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
                paste_clipboard_chunk(&mut cb, &mut enigo, &text)?;
            } else { paste_text_at_focus(&text)?; }
            Ok(())
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
    fn field() -> FocusedField {
        FocusedField { runtime_id: vec![1], automation_id: "message".into(),
            class_name: "editable".into(), control_type: 50004, bounds: [10, 20, 300, 80] }
    }
    #[test]
    fn recreated_field_is_still_the_destination() {
        let a = field(); let mut b = field(); b.runtime_id = vec![2];
        assert!(a.same_field(&b));
    }
    #[test]
    fn different_field_is_not_the_destination() {
        let a = field(); let mut b = field(); b.runtime_id = vec![2];
        b.automation_id = "search".into(); assert!(!a.same_field(&b));
        b.automation_id = a.automation_id.clone(); b.bounds = [10, 100, 300, 160];
        assert!(!a.same_field(&b));
    }
    #[test]
    fn anonymous_objects_need_a_stable_runtime_id() {
        let mut a = field(); a.automation_id.clear(); a.class_name.clear();
        let mut b = a.clone(); b.runtime_id = vec![2]; assert!(!a.same_field(&b));
        b.runtime_id = a.runtime_id.clone(); assert!(a.same_field(&b));
    }
}

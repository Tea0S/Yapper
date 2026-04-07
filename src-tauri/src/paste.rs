use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::mpsc;

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

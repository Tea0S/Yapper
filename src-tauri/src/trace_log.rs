//! Opt-in verbose traces for dictation / sidecar / hotkey debugging (terminal only).
//! - **Dev builds:** verbose unless `YAPPER_VERBOSE=0|false|off`.
//! - **Release:** quiet unless `YAPPER_VERBOSE=1|true|on`.
//! - `[yapper:shortcut]` lines come from `shortcut_log` (global PTT / toggle / stop hotkeys).

pub fn verbose_dictation_trace() -> bool {
    match std::env::var("YAPPER_VERBOSE") {
        Ok(v) => {
            let s = v.trim();
            if s == "0" || s.eq_ignore_ascii_case("false") || s.eq_ignore_ascii_case("off") {
                return false;
            }
            if s == "1" || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("on") {
                return true;
            }
            !s.is_empty()
        }
        Err(_) => cfg!(debug_assertions),
    }
}

#[inline]
pub fn ptt_log(msg: impl std::fmt::Display) {
    if verbose_dictation_trace() {
        eprintln!("[yapper:ptt] {msg}");
    }
}

/// Global shortcut / hotkey path (same `YAPPER_VERBOSE` gate as `ptt_log`).
#[inline]
pub fn shortcut_log(msg: impl std::fmt::Display) {
    if verbose_dictation_trace() {
        eprintln!("[yapper:shortcut] {msg}");
    }
}

#[inline]
pub fn ipc_log(msg: impl std::fmt::Display) {
    if verbose_dictation_trace() {
        eprintln!("[yapper:ipc] {msg}");
    }
}

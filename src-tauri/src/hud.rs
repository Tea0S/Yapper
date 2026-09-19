use crate::state::{AppState, HudPhase};
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "hud";

/// Desktop HUD pill is shown when `hud_widget_enabled` is not `"false"` (default on).
pub(crate) fn widget_enabled(app: &AppHandle) -> Result<bool, String> {
    let conn = crate::open_db(app)?;
    let v = crate::db::get_setting(&conn, "hud_widget_enabled").map_err(|e| e.to_string())?;
    Ok(v.as_deref() != Some("false"))
}

pub(crate) fn widget_style(app: &AppHandle) -> Result<String, String> {
    let conn = crate::open_db(app)?;
    let value = crate::db::get_setting(&conn, "hud_widget_style").map_err(|e| e.to_string())?;
    Ok(if value.as_deref() == Some("controls") { "controls" } else { "classic" }.into())
}

// Tight bounds with a one-pixel antialiasing gutter for the classic pill.
// Leave native transparency and decorations to Tauri; do not clip the HWND.
const SIZE_COLLAPSED: (f64, f64) = (180.0, 40.0);
const SIZE_LISTENING: (f64, f64) = (240.0, 52.0);
const SIZE_PREVIEW: (f64, f64) = (320.0, 104.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HudLayout {
    Collapsed,
    Listening,
    Preview,
}

fn hud_url(app: &AppHandle) -> Result<Url, String> {
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    let mut u = main.url().map_err(|e| e.to_string())?;
    u.set_path("/hud");
    u.set_query(None);
    u.set_fragment(None);
    Ok(u)
}

/// Resize HUD and shift **X** so the window’s horizontal center stays fixed (logical size → may change outer px).
fn set_logical_size_keep_hcenter(win: &WebviewWindow, lw: f64, lh: f64) -> Result<(), String> {
    let pos = win.outer_position().map_err(|e| e.to_string())?;
    let old_sz = win.outer_size().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    if old_sz.width == (lw * scale).round() as u32 && old_sz.height == (lh * scale).round() as u32 { return Ok(()); }
    let center_x = pos.x + old_sz.width as i32 / 2;
    let bottom = pos.y + old_sz.height as i32;

    win.set_size(LogicalSize::new(lw, lh))
        .map_err(|e| e.to_string())?;

    let new_sz = win.outer_size().map_err(|e| e.to_string())?;
    let new_x = center_x - new_sz.width as i32 / 2;
    win.set_position(PhysicalPosition::new(new_x, bottom - new_sz.height as i32))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn position_bottom_center(win: &WebviewWindow) -> Result<(), String> {
    let monitor = win
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| win.primary_monitor().ok().flatten())
        .ok_or_else(|| "no monitor".to_string())?;
    let wa = monitor.work_area();
    let sz = win.outer_size().map_err(|e| e.to_string())?;
    let x = wa.position.x + (wa.size.width as i32 - sz.width as i32) / 2;
    let y = wa.position.y + wa.size.height as i32 - sz.height as i32 - 36;
    win.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn build_hud_window(app: &AppHandle, url: Url) -> Result<WebviewWindow, String> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("Yapper")
        .inner_size(SIZE_COLLAPSED.0, SIZE_COLLAPSED.1)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .focusable(false)
        .shadow(false)
        .transparent(true)
        .on_navigation(|url| crate::allow_navigation_in_webview(url))
        .on_new_window(|url, _| crate::handle_new_window_request(url))
        .build()
        .map_err(|e| e.to_string())
}

/// Create or refresh the HUD as a small bottom pill (engine running).
pub fn ensure_collapsed_visible(app: &AppHandle) -> Result<(), String> {
    if !widget_enabled(app)? {
        hide(app);
        return Ok(());
    }
    let url = hud_url(app)?;
    if let Some(w) = app.get_webview_window(LABEL) {
        w.navigate(url).map_err(|e| e.to_string())?;
        set_layout(app, HudLayout::Collapsed)?;
        w.show().map_err(|e| e.to_string())?;
        let _ = w.set_always_on_top(true);
        return Ok(());
    }

    let win = build_hud_window(app, url)?;
    set_layout(app, HudLayout::Collapsed)?;
    position_bottom_center(&win)?;
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_always_on_top(true);
    Ok(())
}

pub fn set_layout(app: &AppHandle, layout: HudLayout) -> Result<(), String> {
    if !widget_enabled(app)? {
        return Ok(());
    }
    let w = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "hud window missing".to_string())?;
    let (lw, lh) = if widget_style(app)? == "classic" {
        match layout {
            HudLayout::Collapsed => (74.0, 24.0),
            HudLayout::Listening => (104.0, 38.0),
            HudLayout::Preview => (230.0, 66.0),
        }
    } else { match layout {
        HudLayout::Collapsed => SIZE_COLLAPSED,
        HudLayout::Listening => SIZE_LISTENING,
        HudLayout::Preview => SIZE_PREVIEW,
    }};
    set_logical_size_keep_hcenter(&w, lw, lh)?;
    Ok(())
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
}

/// After `ptt_stop` returns, collapse the pill and return to idle (engine still on).
pub struct HudCollapseAfterPtt {
    app: AppHandle,
}

impl HudCollapseAfterPtt {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            app: app.clone(),
        }
    }
}

impl Drop for HudCollapseAfterPtt {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        if state.ptt_session_active.load(std::sync::atomic::Ordering::SeqCst) { return; }
        if let Ok(mut g) = state.hud_phase.lock() {
            *g = HudPhase::Idle;
        }
        let has_outcome = state
            .last_dictation_outcome
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|_| true))
            .unwrap_or(false);
        // Wider layout so the brief post-dictation tip fits above the pill.
        let layout = if has_outcome {
            HudLayout::Preview
        } else {
            HudLayout::Collapsed
        };
        let _ = set_layout(&self.app, layout);
    }
}

/// Optional brief Windows cues, kept off by default.
pub fn sound_cue(app: &AppHandle, frequency: u32) {
    #[cfg(windows)]
    if crate::dictation::enabled(app, "dictation_sound_cues", false) {
        std::thread::spawn(move || unsafe {
            let _ = windows::Win32::System::Diagnostics::Debug::Beep(frequency, 45);
        });
    }
}

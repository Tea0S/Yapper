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

/// Collapsed “always there” capsule — height includes space above the pill for the hover tooltip.
const SIZE_COLLAPSED: (f64, f64) = (112.0, 168.0);
/// Extra width when the meter is active (logical px).
const EXPAND_DELTA_W: f64 = 40.0;
/// Slightly wider while dictating / transcribing.
const SIZE_EXPANDED: (f64, f64) = (SIZE_COLLAPSED.0 + EXPAND_DELTA_W, SIZE_COLLAPSED.1);

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
    let center_x = pos.x + old_sz.width as i32 / 2;

    win.set_size(LogicalSize::new(lw, lh))
        .map_err(|e| e.to_string())?;

    let new_sz = win.outer_size().map_err(|e| e.to_string())?;
    let new_x = center_x - new_sz.width as i32 / 2;
    win.set_position(PhysicalPosition::new(new_x, pos.y))
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
        set_logical_size_keep_hcenter(&w, SIZE_COLLAPSED.0, SIZE_COLLAPSED.1)?;
        w.show().map_err(|e| e.to_string())?;
        let _ = w.set_always_on_top(true);
        return Ok(());
    }

    let win = build_hud_window(app, url)?;
    position_bottom_center(&win)?;
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_always_on_top(true);
    Ok(())
}

pub fn set_expanded(app: &AppHandle, expanded: bool) -> Result<(), String> {
    if !widget_enabled(app)? {
        return Ok(());
    }
    let w = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "hud window missing".to_string())?;
    let (lw, lh) = if expanded {
        SIZE_EXPANDED
    } else {
        SIZE_COLLAPSED
    };
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
        if let Ok(mut g) = state.hud_phase.lock() {
            *g = HudPhase::Idle;
        }
        let _ = set_expanded(&self.app, false);
    }
}

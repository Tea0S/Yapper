use crate::state::{AppState, HudPhase};
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "hud";

/// Collapsed “always there” capsule — height fits hover tooltip inside the webview.
#[cfg(not(target_os = "macos"))]
const SIZE_COLLAPSED: (f64, f64) = (112.0, 88.0);
#[cfg(target_os = "macos")]
const SIZE_COLLAPSED: (f64, f64) = (112.0, 36.0);
/// macOS keeps a fixed-width pill and only grows slightly taller while active.
#[cfg(not(target_os = "macos"))]
const SIZE_EXPANDED: (f64, f64) = (268.0, 88.0);
#[cfg(target_os = "macos")]
const SIZE_EXPANDED: (f64, f64) = (112.0, 40.0);

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

#[cfg(target_os = "macos")]
fn resize_preserving_center(win: &WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    let old_size = win.outer_size().map_err(|e| e.to_string())?;
    let old_pos = win.outer_position().map_err(|e| e.to_string())?;

    let center_x = old_pos.x + old_size.width as i32 / 2;
    let center_y = old_pos.y + old_size.height as i32 / 2;

    win.set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;

    let new_size = win.outer_size().map_err(|e| e.to_string())?;
    let mut x = center_x - new_size.width as i32 / 2;
    let mut y = center_y - new_size.height as i32 / 2;

    let monitor = win
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| win.primary_monitor().ok().flatten())
        .ok_or_else(|| "no monitor".to_string())?;
    let wa = monitor.work_area();
    let min_x = wa.position.x;
    let min_y = wa.position.y;
    let max_x = wa.position.x + wa.size.width as i32 - new_size.width as i32;
    let max_y = wa.position.y + wa.size.height as i32 - new_size.height as i32;

    x = x.clamp(min_x, max_x.max(min_x));
    y = y.clamp(min_y, max_y.max(min_y));

    win.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn build_hud_window(app: &AppHandle, url: Url) -> Result<WebviewWindow, String> {
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
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
        .shadow(false);

    builder = builder.transparent(true);

    builder.build().map_err(|e| e.to_string())
}

/// Create or refresh the HUD as a small bottom pill (engine running).
pub fn ensure_collapsed_visible(app: &AppHandle) -> Result<(), String> {
    let url = hud_url(app)?;
    if let Some(w) = app.get_webview_window(LABEL) {
        w.navigate(url).map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        resize_preserving_center(&w, SIZE_COLLAPSED.0, SIZE_COLLAPSED.1)?;
        #[cfg(not(target_os = "macos"))]
        w.set_size(LogicalSize::new(SIZE_COLLAPSED.0, SIZE_COLLAPSED.1))
            .map_err(|e| e.to_string())?;
        #[cfg(not(target_os = "macos"))]
        position_bottom_center(&w)?;
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
    let w = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "hud window missing".to_string())?;
    let (lw, lh) = if expanded {
        SIZE_EXPANDED
    } else {
        SIZE_COLLAPSED
    };
    #[cfg(target_os = "macos")]
    resize_preserving_center(&w, lw, lh)?;
    #[cfg(not(target_os = "macos"))]
    w.set_size(LogicalSize::new(lw, lh))
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "macos"))]
    position_bottom_center(&w)?;
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

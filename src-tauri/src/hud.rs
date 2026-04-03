use crate::state::{AppState, HudPhase};
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "hud";

/// Collapsed “always there” capsule (logical px).
const SIZE_COLLAPSED: (f64, f64) = (92.0, 34.0);
/// Expanded meter while dictating / transcribing.
const SIZE_EXPANDED: (f64, f64) = (268.0, 58.0);

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

    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.transparent(true);
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder.transparent(false);
    }

    builder.build().map_err(|e| e.to_string())
}

/// Create or refresh the HUD as a small bottom pill (engine running).
pub fn ensure_collapsed_visible(app: &AppHandle) -> Result<(), String> {
    let url = hud_url(app)?;
    if let Some(w) = app.get_webview_window(LABEL) {
        w.navigate(url).map_err(|e| e.to_string())?;
        w.set_size(LogicalSize::new(SIZE_COLLAPSED.0, SIZE_COLLAPSED.1))
            .map_err(|e| e.to_string())?;
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
    w.set_size(LogicalSize::new(lw, lh))
        .map_err(|e| e.to_string())?;
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

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

// Window bounds match the visible surface; no invisible tooltip reservation.
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
    ensure_borderless_frame(win)?;
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
    shape_window(win)?;
    Ok(())
}

/// Tao hides decorations through WM_NCCALCSIZE but retains WS_CAPTION.
/// A custom region can expose that native frame. Remove its styles before shaping,
/// and repair them if a later Tao visibility/topmost update restores them.
fn ensure_borderless_frame(win: &WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_STYLE,
            WS_CAPTION, WS_THICKFRAME, WS_SYSMENU, WS_MINIMIZEBOX, WS_MAXIMIZEBOX,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        };
        let hwnd = windows::Win32::Foundation::HWND(win.hwnd().map_err(|e| e.to_string())?.0);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let borderless = style & !(WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0
            | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0);
        if style != borderless {
            SetWindowLongW(hwnd, GWL_STYLE, borderless as i32);
            SetWindowPos(hwnd, None, 0, 0, 0, 0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Clip native hit testing too, so rounded transparent corners belong to the app behind us.
fn shape_window(win: &WebviewWindow) -> Result<(), String> {
    ensure_borderless_frame(win)?;
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
        let size = win.outer_size().map_err(|e| e.to_string())?;
        let radius = if widget_style(win.app_handle())? == "classic" {
            size.height as i32
        } else { (32.0 * win.scale_factor().map_err(|e| e.to_string())?).round() as i32 };
        let hwnd = win.hwnd().map_err(|e| e.to_string())?;
        unsafe {
            let region = CreateRoundRectRgn(0, 0, size.width as i32 + 1, size.height as i32 + 1, radius, radius);
            if region.0.is_null() { return Err("Could not shape dictation widget".into()); }
            if SetWindowRgn(windows::Win32::Foundation::HWND(hwnd.0), Some(region), true) == 0 {
                let _ = DeleteObject(region.into());
                return Err("Could not apply dictation widget shape".into());
            }
        }
    }
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
        shape_window(&w)?;
        return Ok(());
    }

    let win = build_hud_window(app, url)?;
    set_layout(app, HudLayout::Collapsed)?;
    shape_window(&win)?;
    position_bottom_center(&win)?;
    win.show().map_err(|e| e.to_string())?;
    let _ = win.set_always_on_top(true);
    shape_window(&win)?;
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
            HudLayout::Collapsed => (72.0, 22.0),
            HudLayout::Listening => (88.0, 36.0),
            HudLayout::Preview => (300.0, 64.0),
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

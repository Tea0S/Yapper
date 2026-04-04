//! On Windows, Tao applies the window icon only as `ICON_SMALL` (title bar). The taskbar button
//! uses `ICON_BIG`; if it is never set, WebView2 leaves the default blue placeholder.
//!
//! In **release** bundles, prefer loading the icon from the same PE resource `tauri-winres` embeds
//! (`32512`) so we do not depend on in-memory RGBA decoding matching the installer `exe`.

use std::mem;

use tauri::image::Image;
use tauri::WebviewWindow;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, DestroyIcon, LoadImageW, SendMessageW, HICON, ICON_BIG, IMAGE_ICON, LR_DEFAULTSIZE,
    WM_SETICON,
};

/// Resource ID used by `tauri-build` / `tauri-winres` (`set_icon_with_id(..., "32512")`).
const TAURI_WINRES_ICON_ID: usize = 32512;

#[repr(C)]
struct Pixel {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

const PIXEL_SIZE: usize = mem::size_of::<Pixel>();

impl Pixel {
    fn to_bgra(&mut self) {
        mem::swap(&mut self.r, &mut self.b);
    }
}

pub fn apply_taskbar_big_icon<R: tauri::Runtime>(window: &WebviewWindow<R>, icon: &Image<'_>) {
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    if apply_big_icon_from_exe_module(hwnd) {
        return;
    }
    if let Some(hicon) = hicon_from_rgba(icon) {
        unsafe {
            replace_wm_seticon(hwnd, ICON_BIG, hicon);
        }
    }
}

/// Uses the embedded application icon from the running `exe` (matches Start Menu / file icon).
fn apply_big_icon_from_exe_module(hwnd: HWND) -> bool {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return false;
        };
        let Ok(img) = LoadImageW(
            Some(windows::Win32::Foundation::HINSTANCE(module.0)),
            PCWSTR(TAURI_WINRES_ICON_ID as *mut u16),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        ) else {
            return false;
        };
        let hicon = HICON(img.0);
        replace_wm_seticon(hwnd, ICON_BIG, hicon);
        true
    }
}

unsafe fn replace_wm_seticon(hwnd: HWND, kind: u32, hicon: HICON) {
    let prev = SendMessageW(
        hwnd,
        WM_SETICON,
        Some(WPARAM(kind as usize)),
        Some(LPARAM(hicon.0 as isize)),
    );
    if prev.0 != 0 {
        let _ = DestroyIcon(HICON(prev.0 as _));
    }
}

fn hicon_from_rgba(icon: &Image<'_>) -> Option<HICON> {
    let width = icon.width();
    let height = icon.height();
    if width == 0 || height == 0 {
        return None;
    }
    let mut rgba = icon.rgba().to_vec();
    if rgba.len() % PIXEL_SIZE != 0 {
        return None;
    }
    let pixel_count = rgba.len() / PIXEL_SIZE;
    let expected = (width as usize).checked_mul(height as usize)?;
    if pixel_count != expected {
        return None;
    }

    let mut and_mask = Vec::with_capacity(pixel_count);
    let pixels: &mut [Pixel] = unsafe {
        std::slice::from_raw_parts_mut(rgba.as_mut_ptr().cast::<Pixel>(), pixel_count)
    };
    for pixel in pixels.iter_mut() {
        and_mask.push(pixel.a.wrapping_sub(u8::MAX));
        pixel.to_bgra();
    }

    let w = i32::try_from(width).ok()?;
    let h = i32::try_from(height).ok()?;
    unsafe {
        CreateIcon(
            None,
            w,
            h,
            1,
            (PIXEL_SIZE * 8) as u8,
            and_mask.as_ptr(),
            rgba.as_ptr(),
        )
        .ok()
    }
}

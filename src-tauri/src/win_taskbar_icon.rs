//! On Windows, Tao applies the window icon only as `ICON_SMALL` (title bar). The taskbar button
//! uses `ICON_BIG`; if it is never set, WebView2 leaves the default blue placeholder.

use std::mem;

use tauri::image::Image;
use tauri::WebviewWindow;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, SendMessageW, DestroyIcon, HICON, ICON_BIG, WM_SETICON,
};

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
    let Some(hicon) = hicon_from_rgba(icon) else {
        return;
    };
    unsafe {
        let prev = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
        if prev.0 != 0 {
            let _ = DestroyIcon(HICON(prev.0 as _));
        }
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

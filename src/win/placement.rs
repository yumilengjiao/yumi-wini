//! Applying computed layout geometry to real windows.
//!
//! The layout engine computes pixel rectangles; this module pushes them
//! to the actual HWNDs with `SetWindowPos`. We never touch Z-order or
//! activation from here — focus is the OS's (and later the focus
//! module's) business.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
};

use crate::layout::geometry::TileRect;

/// Move/resize windows to their computed tiles. Skips dead handles
/// silently (a close race with WinEvents is normal).
pub fn apply_geometry(rects: &[TileRect]) {
    for r in rects {
        let hwnd = HWND(r.id as *mut core::ffi::c_void);
        if !super::api::is_alive(hwnd) {
            continue;
        }
        let ok = unsafe {
            SetWindowPos(
                hwnd,
                None,
                r.x,
                r.y,
                r.w,
                r.h,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            )
        };
        if ok.is_err() {
            log::warn!("SetWindowPos failed for window {}", r.id);
        }
    }
}

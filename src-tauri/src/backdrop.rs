//! Windows 11 window backdrop (Phase 7).
//!
//! window-vibrancy is the Phase 7-only approved dependency. We try Mica (the
//! Win11 material), fall back to acrylic on older builds, and otherwise
//! report "none" — the UI keeps an opaque background in that case, so a
//! failed backdrop can never leave a see-through window.

use std::sync::atomic::{AtomicU8, Ordering};

/// No OS backdrop: the UI paints an opaque background.
pub const BACKDROP_NONE: u8 = 0;
/// Windows 11 Mica.
pub const BACKDROP_MICA: u8 = 1;
/// Windows 10/11 acrylic.
pub const BACKDROP_ACRYLIC: u8 = 2;

static BACKDROP: AtomicU8 = AtomicU8::new(BACKDROP_NONE);

/// Wire name for a backdrop code (the UI puts it in `data-backdrop`).
pub fn name(code: u8) -> &'static str {
    match code {
        BACKDROP_MICA => "mica",
        BACKDROP_ACRYLIC => "acrylic",
        _ => "none",
    }
}

/// Try Mica, then acrylic, then give up. Returns the accepted code.
#[cfg(target_os = "windows")]
fn apply(window: &tauri::WebviewWindow, dark: bool) -> u8 {
    if window_vibrancy::apply_mica(window, Some(dark)).is_ok() {
        return BACKDROP_MICA;
    }
    // Acrylic is deprecated on Win11 22H2+, but it is the only material left
    // on Windows 10, so trying it costs nothing and helps older machines.
    let tint = if dark {
        (18, 18, 18, 125)
    } else {
        (243, 245, 250, 125)
    };
    if window_vibrancy::apply_acrylic(window, Some(tint)).is_ok() {
        return BACKDROP_ACRYLIC;
    }
    BACKDROP_NONE
}

#[cfg(not(target_os = "windows"))]
fn apply(_window: &tauri::WebviewWindow, _dark: bool) -> u8 {
    BACKDROP_NONE
}

/// Apply for the given theme and remember what the OS accepted.
pub fn store(window: &tauri::WebviewWindow, dark: bool) -> &'static str {
    let code = apply(window, dark);
    BACKDROP.store(code, Ordering::Relaxed);
    name(code)
}

/// Which backdrop the OS accepted: "mica", "acrylic" or "none".
#[tauri::command]
pub fn backdrop_state() -> &'static str {
    name(BACKDROP.load(Ordering::Relaxed))
}

/// Re-tint the backdrop when the UI theme changes (Mica is dark or light,
/// never both), returning the material now in effect.
#[tauri::command]
pub fn set_backdrop_theme(window: tauri::WebviewWindow, dark: bool) -> &'static str {
    store(&window, dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrop_names_are_stable() {
        assert_eq!(name(BACKDROP_NONE), "none");
        assert_eq!(name(BACKDROP_MICA), "mica");
        assert_eq!(name(BACKDROP_ACRYLIC), "acrylic");
        // Unknown codes must never be reported as a material.
        assert_eq!(name(9), "none");
    }
}

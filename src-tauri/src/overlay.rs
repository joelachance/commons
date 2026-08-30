//! The panel: where it sits, and how it comes and goes.
//!
//! Commons is a capture surface, not an app. It has no Dock icon and no
//! Cmd-Tab entry, it follows the user onto every Space, and the one hotkey
//! both summons and dismisses it.
//!
//! The frame is computed once at setup and re-asserted on every show. macOS
//! anchors a hidden window's resize to its bottom edge and lets the position
//! drift, so trusting the window to remember where it was leaves the panel
//! creeping down the screen over a day's use.

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow};

/// Gap between the panel and the screen edges.
const MARGIN: f64 = 10.0;
/// A third of the screen, as asked for.
const WIDTH_FRACTION: f64 = 1.0 / 3.0;
/// NSFloatingWindowLevel: above ordinary windows, below the menu bar.
const LEVEL: isize = 3;

#[derive(Clone, Copy)]
pub struct Layout {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

static LAYOUT: std::sync::Mutex<Option<Layout>> = std::sync::Mutex::new(None);

/// Computes the frame from the work area - the screen minus the menu bar and
/// the Dock - so the panel never hides behind either.
pub fn install(win: &WebviewWindow) -> tauri::Result<()> {
    let monitor = win
        .primary_monitor()?
        .or(win.current_monitor()?)
        .expect("no monitor");
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let pos = area.position.to_logical::<f64>(scale);
    let size = area.size.to_logical::<f64>(scale);

    let width = (size.width * WIDTH_FRACTION).round();
    let layout = Layout {
        // Docked to the right edge: the panel is a margin, not a centrepiece,
        // and the right side is where a scratch surface belongs on a desktop
        // whose windows are already anchored left.
        x: pos.x + size.width - width - MARGIN,
        y: pos.y + MARGIN,
        width,
        height: size.height - MARGIN * 2.0,
    };
    *LAYOUT.lock().unwrap() = Some(layout);

    let ns_win = win.ns_window()? as *mut AnyObject;
    unsafe {
        // canJoinAllSpaces | fullScreenAuxiliary: follow the user to every
        // Space, including over a fullscreen app.
        let _: () = msg_send![ns_win, setCollectionBehavior: 257usize];
        let _: () = msg_send![ns_win, setLevel: LEVEL];
    }
    apply(win, layout);
    Ok(())
}

fn apply(win: &WebviewWindow, l: Layout) {
    let _ = win.set_position(LogicalPosition::new(l.x, l.y));
    let _ = win.set_size(LogicalSize::new(l.width, l.height));
}

pub fn show(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if let Some(l) = *LAYOUT.lock().unwrap() {
        apply(&win, l);
    }
    let _ = win.show();
    if let Ok(ns_win) = win.ns_window() {
        let ns_win = ns_win as *mut AnyObject;
        unsafe {
            let _: () = msg_send![ns_win, setLevel: LEVEL];
            // Unlike a heads-up display, a notes panel is useless without the
            // caret, so this one takes focus rather than ordering in behind it.
            let app_cls: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app_cls, activateIgnoringOtherApps: true];
            let _: () = msg_send![ns_win, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
        }
    }
    let _ = win.set_focus();
    // The page focuses its input and plays the entrance on this.
    use tauri::Emitter;
    let _ = win.emit("commons:shown", ());
}

pub fn hide(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    use tauri::Emitter;
    let _ = win.emit("commons:hidden", ());
    let _ = win.hide();
    unsafe {
        // Hand the keyboard back to whatever the user was doing before.
        let ns_app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![ns_app, hide: std::ptr::null::<AnyObject>()];
    }
}

pub fn toggle(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide(app);
    } else {
        show(app);
    }
}

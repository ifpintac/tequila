//! What a Mac needs of the application beyond the window egui draws.

/// Quit - Cmd+Q, and Quit in the application menu - asks about unsaved
/// work, as closing the window does.
///
/// winit's application menu sends AppKit's `terminate:`, which ends the
/// application without the window ever being asked to close: eframe calls
/// `on_exit` straight away (its own comment says as much), and "Save your
/// changes?" never comes. Sent `performClose:` to the main window instead,
/// Quit does what the window's close button does: the window is asked,
/// Tequila asks the person, and the window closing ends the application.
/// The Dock's Quit and logging out still end it unasked; `end_session` keeps
/// the crash journal for those, so the work comes back on the next launch.
///
/// Called once the window, and with it the menu, exists.
#[cfg(target_os = "macos")]
pub fn quit_asks_first(window: &impl raw_window_handle::HasWindowHandle) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send, sel};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: plain AppKit messages on the main thread, where eframe calls
    // its creation callback; every object is checked for nil before use, and
    // the menu item only refers to the window, which lives as long as the
    // application does.
    unsafe {
        let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
        let ns_window: *mut AnyObject = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let menu: *mut AnyObject = msg_send![application, mainMenu];
        if menu.is_null() {
            return;
        }
        let count: isize = msg_send![menu, numberOfItems];
        for index in 0..count {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: index];
            let submenu: *mut AnyObject = msg_send![item, submenu];
            if submenu.is_null() {
                continue;
            }
            let nobody = std::ptr::null_mut::<AnyObject>();
            let at: isize = msg_send![submenu, indexOfItemWithTarget: nobody, andAction: sel!(terminate:)];
            if at >= 0 {
                let quit: *mut AnyObject = msg_send![submenu, itemAtIndex: at];
                let _: () = msg_send![quit, setTarget: ns_window];
                let _: () = msg_send![quit, setAction: sel!(performClose:)];
                return;
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn quit_asks_first<T>(_window: &T) {}

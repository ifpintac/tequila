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

/// Files macOS has asked Tequila to open - double-clicked in Finder, dropped
/// on the Dock icon, chosen with Open With - until the window takes them
/// ([`take_opened`]).
static OPENED: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());

/// The window to wake when a file arrives while it waits for input.
static WAKE: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// The files asked for since the last call.
pub fn take_opened() -> Vec<std::path::PathBuf> {
    OPENED.lock().map(|mut opened| std::mem::take(&mut *opened)).unwrap_or_default()
}

/// Wake `ctx` when a file arrives, so a window idle in the background opens
/// it at once rather than at the next mouse move.
pub fn wake_on_open(ctx: &egui::Context) {
    let _ = WAKE.set(ctx.clone());
}

/// Take the files macOS asks Tequila to open. Called in `main`, before the
/// event loop starts.
///
/// macOS opens a file in an application by an Apple Event, "open documents",
/// not by naming it on the command line: double-clicking a project showed
/// Tequila the demo scene on a first launch, and nothing at all when it was
/// running already. A handler for that event is installed as AppKit is about
/// to finish launching - installed any earlier, AppKit installs its own over
/// it; any later, and the event that launched the application has come and
/// gone. winit's application delegate is its own and is left alone.
#[cfg(target_os = "macos")]
pub fn listen_for_opened_files() {
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, NSObject, Sel};
    use objc2::{class, msg_send, sel, ClassType};

    /// 'aevt' and 'odoc': the core event class and its open-documents event.
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    /// The event's direct object, and a file URL descriptor.
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    const FILE_URL: u32 = u32::from_be_bytes(*b"furl");

    extern "C-unwind" fn will_finish_launching(this: &AnyObject, _: Sel, _notification: *mut AnyObject) {
        // SAFETY: AppKit's documented way to handle an Apple Event; `this`
        // lives for the rest of the process (it is never released).
        unsafe {
            let manager: *mut AnyObject = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            if manager.is_null() {
                return;
            }
            let _: () = msg_send![
                manager,
                setEventHandler: this,
                andSelector: sel!(openDocuments:withReplyEvent:),
                forEventClass: CORE_EVENT_CLASS,
                andEventID: OPEN_DOCUMENTS
            ];
        }
    }

    extern "C-unwind" fn open_documents(_this: &AnyObject, _: Sel, event: *mut AnyObject, _reply: *mut AnyObject) {
        if event.is_null() {
            return;
        }
        let mut paths = Vec::new();
        // SAFETY: plain Foundation messages on the descriptors AppKit hands
        // the handler, on the main thread; each object is checked for nil.
        unsafe {
            let list: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
            if list.is_null() {
                return;
            }
            // A list of files, or one file on its own.
            let count: isize = msg_send![list, numberOfItems];
            let items: Vec<*mut AnyObject> = if count > 0 {
                (1..=count).map(|i| msg_send![list, descriptorAtIndex: i]).collect()
            } else {
                vec![list]
            };
            for item in items.into_iter().filter(|item| !item.is_null()) {
                let url: *mut AnyObject = msg_send![item, coerceToDescriptorType: FILE_URL];
                if url.is_null() {
                    continue;
                }
                let data: *mut AnyObject = msg_send![url, data];
                if data.is_null() {
                    continue;
                }
                let nothing = std::ptr::null_mut::<AnyObject>();
                let ns_url: *mut AnyObject = msg_send![class!(NSURL), URLWithDataRepresentation: data, relativeToURL: nothing];
                if ns_url.is_null() {
                    continue;
                }
                let path: *mut AnyObject = msg_send![ns_url, path];
                if path.is_null() {
                    continue;
                }
                let utf8: *const std::ffi::c_char = msg_send![path, UTF8String];
                if utf8.is_null() {
                    continue;
                }
                let text = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
                paths.push(std::path::PathBuf::from(text));
            }
        }
        if paths.is_empty() {
            return;
        }
        if let Ok(mut opened) = OPENED.lock() {
            opened.extend(paths);
        }
        if let Some(ctx) = WAKE.get() {
            ctx.request_repaint();
        }
    }

    // SAFETY: a class of Tequila's own, built once with methods whose
    // signatures are the selectors' (an object and two descriptors, and a
    // notification), then one instance of it kept for the life of the
    // process as the notification's observer and the event's handler.
    objc2::rc::autoreleasepool(|_| unsafe {
        let Some(mut builder) = ClassBuilder::new(c"TequilaOpenFiles", NSObject::class()) else {
            return;
        };
        builder.add_method(
            sel!(willFinishLaunching:),
            will_finish_launching as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(openDocuments:withReplyEvent:),
            open_documents as extern "C-unwind" fn(_, _, _, _),
        );
        let opener: &AnyClass = builder.register();
        let observer: Option<objc2::rc::Retained<AnyObject>> = msg_send![opener, new];
        let Some(observer) = observer else {
            return;
        };
        // Never released: the notification centre and the event manager
        // do not keep what they call.
        let observer = objc2::rc::Retained::into_raw(observer);
        let centre: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        // The notification's name is the constant's own.
        let name: *mut AnyObject = msg_send![
            class!(NSString),
            stringWithUTF8String: c"NSApplicationWillFinishLaunchingNotification".as_ptr()
        ];
        let anyone = std::ptr::null_mut::<AnyObject>();
        let _: () = msg_send![
            centre,
            addObserver: observer,
            selector: sel!(willFinishLaunching:),
            name: name,
            object: anyone
        ];
    });
}

#[cfg(not(target_os = "macos"))]
pub fn listen_for_opened_files() {}

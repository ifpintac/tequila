//! On a Mac with 3Dconnexion's driver (3DxWare) running: the device read
//! through the driver, as the other applications there read it, so the
//! driver's own settings for Tequila - its speed, and which axes move and
//! which way - apply here too, and the mouse goes on working everywhere
//! else.
//!
//! The driver takes the device for itself, so [`super::hid`] can't open it,
//! and offers instead a framework any application may call to be sent its
//! motion: `/Library/Frameworks/3DconnexionClient.framework`, installed with
//! the driver. Tequila ships nothing of it and is built without it: when the
//! device turns out to be held, the framework the person's own driver put
//! there is loaded, and the few functions and the one message read here are
//! declared below, in Tequila's own words. No part of 3Dconnexion's SDK - its
//! headers, samples or libraries - is used, copied or needed; the SDK's
//! licence binds those who download and use the SDK.
//!
//! What is called:
//!
//! - `SetConnexionHandlers(message, added, removed, separate_thread)`, zero
//!   when it worked (`InstallConnexionHandlers(message, added, removed)` on
//!   a driver older than version 10), and `CleanupConnexionHandlers()` to
//!   stop. Messages then come through a run loop: this thread's, which the
//!   reader turns, or the main thread's, which the window's event loop
//!   turns. Either way they come here.
//! - `RegisterConnexionClient(signature, name, mode, mask)`: the client's
//!   number, or zero when refused. The signature `'****'` matches the
//!   application by `name`, a Pascal string (a length byte, then the
//!   letters) of its executable's name: the driver sends motion only while
//!   that application is in front, and keeps its settings for it under
//!   that name. Mode 1 takes the buttons over, so the driver no longer acts
//!   on them inside Tequila and Tequila's own button settings apply; the
//!   mask `0x3FFF` asks for the six axes and the buttons.
//! - `SetConnexionClientButtonMask(client, 0xFFFFFFFF)`: every button.
//! - `UnregisterConnexionClient(client)`.
//!
//! A message comes as `message(product, type, argument)`. Type `'3dSR'` is
//! the device's state, a 48-byte record packed to two bytes ([`State`]):
//! version (u16), client (u16), command (u16), parameter (i16), value
//! (i32), time (u64), the raw report (8 bytes), the first eight buttons
//! (u16), the six axes (i16 each: X, Y, Z, then the turns about them), the
//! device's address (u16) and the buttons (u32). The state goes to every
//! client, so only one addressed to this client is read. Command 3 says the
//! axes have moved, as the driver's settings make them; command 2 that the
//! buttons have changed. The axes are the device's own, as its HID reports
//! have them, so they go through the same [`to_view`](super::report::to_view),
//! with room above full deflection for a driver set faster than standard.

// The decoding is plain and tested everywhere; the framework is a Mac's.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::report::{to_view_within, Event};

/// Where 3Dconnexion's driver installs its framework.
pub const FRAMEWORK: &str = "/Library/Frameworks/3DconnexionClient.framework/3DconnexionClient";

/// What the device list shows after the device's name while it is read
/// through the driver.
pub const THROUGH_DRIVER: &str = ", through 3Dconnexion's driver";

/// A four-letter code, as the framework's numbers are written.
const fn code(letters: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*letters)
}

/// The device's state, the one message read.
const MESSAGE_STATE: u32 = code(b"3dSR");
/// Any application, told by its name.
const ANY_SIGNATURE: u32 = code(b"****");
/// The buttons are Tequila's while it is in front.
const MODE_TAKE_OVER: u16 = 1;
/// The six axes and the buttons.
const MASK_ALL: u32 = 0x3FFF;
const ALL_BUTTONS: u32 = 0xFFFF_FFFF;
/// What a state says has changed.
const COMMAND_BUTTONS: u16 = 2;
const COMMAND_AXES: u16 = 3;

/// The driver's speed setting goes this far past full deflection before
/// the motion is held: about four times its standard.
pub const DRIVER_LIMIT: f32 = 6.0;

/// The device's state, as the driver sends it: 48 bytes, packed to two.
pub const STATE_LEN: usize = 48;
pub type State = [u8; STATE_LEN];

/// What a state means for the client numbered `client`: `None` for a state
/// meant for another client, or one that says nothing this reads.
pub fn decode(state: &State, client: u16) -> Option<Event> {
    let u16_at = |at: usize| u16::from_le_bytes([state[at], state[at + 1]]);
    if u16_at(2) != client {
        return None;
    }
    match u16_at(4) {
        COMMAND_AXES => {
            let axis = |i: usize| i16::from_le_bytes([state[30 + 2 * i], state[31 + 2 * i]]) as i32;
            Some(Event::Motion(to_view_within([axis(0), axis(1), axis(2), axis(3), axis(4), axis(5)], DRIVER_LIMIT)))
        }
        COMMAND_BUTTONS => Some(Event::Buttons(u32::from_le_bytes([state[44], state[45], state[46], state[47]]))),
        _ => None,
    }
}

/// A state as the driver would send it, for tests.
#[cfg(test)]
pub fn state(client: u16, command: u16, axes: [i16; 6], buttons: u32) -> State {
    let mut out = [0u8; STATE_LEN];
    out[2..4].copy_from_slice(&client.to_le_bytes());
    out[4..6].copy_from_slice(&command.to_le_bytes());
    for (i, v) in axes.iter().enumerate() {
        out[30 + 2 * i..32 + 2 * i].copy_from_slice(&v.to_le_bytes());
    }
    out[28..30].copy_from_slice(&((buttons & 0xFF) as u16).to_le_bytes());
    out[44..48].copy_from_slice(&buttons.to_le_bytes());
    out
}

/// The executable's name as a Pascal string, for the driver to know the
/// application by: `Tequila` in the application bundle.
pub fn client_name(executable: Option<&std::path::Path>) -> Vec<u8> {
    let name = executable.and_then(|p| p.file_name()).and_then(|n| n.to_str()).filter(|n| !n.is_empty()).unwrap_or("Tequila");
    let mut bytes: Vec<u8> = name.bytes().take(255).collect();
    bytes.insert(0, bytes.len() as u8);
    bytes
}

#[cfg(target_os = "macos")]
pub use framework::{api, run};

#[cfg(target_os = "macos")]
mod framework {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    type MessageHandler = extern "C" fn(u32, u32, *mut c_void);
    type DeviceHandler = extern "C" fn(u32);

    /// The framework's functions, as this calls them.
    #[derive(Clone, Copy)]
    pub struct Api {
        pub(super) set_handlers: Option<unsafe extern "C" fn(MessageHandler, DeviceHandler, DeviceHandler, u8) -> i16>,
        pub(super) install_handlers: Option<unsafe extern "C" fn(MessageHandler, DeviceHandler, DeviceHandler) -> i16>,
        pub(super) cleanup_handlers: unsafe extern "C" fn(),
        pub(super) register: unsafe extern "C" fn(u32, *const u8, u16, u32) -> u16,
        pub(super) button_mask: Option<unsafe extern "C" fn(u16, u32)>,
        pub(super) unregister: unsafe extern "C" fn(u16),
    }

    /// The driver's framework, loaded the first time it is asked for and
    /// kept: the framework keeps threads of its own, so it is never let go.
    /// `Err` says why it can't be used: not installed, or not loading.
    pub fn api() -> Result<Api, String> {
        static LOADED: OnceLock<Result<Api, String>> = OnceLock::new();
        LOADED.get_or_init(|| load(FRAMEWORK)).clone()
    }

    fn load(path: &str) -> Result<Api, String> {
        if !std::path::Path::new(path).exists() {
            return Err(format!("3Dconnexion's driver framework isn't installed ({path})."));
        }
        // SAFETY: the framework is 3Dconnexion's, installed with its
        // driver; loading it runs its initialisers, as in every application
        // that uses the driver.
        let library = unsafe { libloading::Library::new(path) }.map_err(|e| format!("3Dconnexion's driver framework wouldn't load: {e}"))?;
        let library: &'static libloading::Library = Box::leak(Box::new(library));
        // SAFETY: each symbol is given the type the framework's function
        // has (see the module's notes).
        unsafe {
            let need = |name: &str| format!("3Dconnexion's driver framework has no {name}: the driver may be too old.");
            Ok(Api {
                set_handlers: library.get(b"SetConnexionHandlers\0").ok().map(|s| *s),
                install_handlers: library.get(b"InstallConnexionHandlers\0").ok().map(|s| *s),
                cleanup_handlers: *library.get(b"CleanupConnexionHandlers\0").map_err(|_| need("CleanupConnexionHandlers"))?,
                register: *library.get(b"RegisterConnexionClient\0").map_err(|_| need("RegisterConnexionClient"))?,
                button_mask: library.get(b"SetConnexionClientButtonMask\0").ok().map(|s| *s),
                unregister: *library.get(b"UnregisterConnexionClient\0").map_err(|_| need("UnregisterConnexionClient"))?,
            })
        }
    }

    /// Where the framework's messages go while a client is registered: the
    /// client's number, and the reader's end of a channel.
    struct Sink {
        client: u16,
        events: mpsc::Sender<Event>,
    }

    static SINK: Mutex<Option<Sink>> = Mutex::new(None);

    extern "C" fn on_message(_product: u32, kind: u32, argument: *mut c_void) {
        if kind != MESSAGE_STATE || argument.is_null() {
            return;
        }
        let mut state = [0u8; STATE_LEN];
        // SAFETY: a state message's argument is the driver's state record,
        // `STATE_LEN` bytes, valid for the call.
        unsafe { std::ptr::copy_nonoverlapping(argument as *const u8, state.as_mut_ptr(), STATE_LEN) };
        if let Ok(sink) = SINK.lock() {
            if let Some(sink) = sink.as_ref() {
                if let Some(event) = decode(&state, sink.client) {
                    let _ = sink.events.send(event);
                }
            }
        }
    }

    extern "C" fn on_added(_product: u32) {}

    /// A device gone: let go of what it held.
    extern "C" fn on_removed(_product: u32) {
        if let Ok(sink) = SINK.lock() {
            if let Some(sink) = sink.as_ref() {
                let _ = sink.events.send(Event::Motion(Default::default()));
                let _ = sink.events.send(Event::Buttons(0));
            }
        }
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
    }

    /// `CFRunLoopRunInMode`'s answer when the run loop has nothing to wait
    /// for.
    const RUN_LOOP_EMPTY: i32 = 1;

    /// Read the device through the driver until `running` goes false or
    /// `present` - asked every two seconds - says it has gone. Each event
    /// is handed to `send`. `Err` says why the driver would not take
    /// Tequila on, for the person.
    pub fn run(api: &Api, running: &AtomicBool, present: &mut dyn FnMut() -> bool, send: &mut dyn FnMut(Event)) -> Result<(), String> {
        let (events, received) = mpsc::channel();
        if let Ok(mut sink) = SINK.lock() {
            *sink = Some(Sink { client: 0, events });
        }
        let forget = || {
            if let Ok(mut sink) = SINK.lock() {
                *sink = None;
            }
        };
        // SAFETY: the handlers are this module's, alive for the program.
        let installed = unsafe {
            match (api.set_handlers, api.install_handlers) {
                (Some(set), _) => set(on_message, on_added, on_removed, 0),
                (None, Some(install)) => install(on_message, on_added, on_removed),
                (None, None) => {
                    forget();
                    return Err("3Dconnexion's driver framework has no way to send messages: the driver may be too old.".into());
                }
            }
        };
        if installed != 0 {
            forget();
            return Err(format!("3Dconnexion's driver didn't take Tequila's messages (error {installed}). Is 3DxWare running?"));
        }
        let name = client_name(std::env::current_exe().ok().as_deref());
        // SAFETY: `name` outlives the call; the driver copies it.
        let client = unsafe { (api.register)(ANY_SIGNATURE, name.as_ptr(), MODE_TAKE_OVER, MASK_ALL) };
        if client == 0 {
            // SAFETY: the handlers were installed above.
            unsafe { (api.cleanup_handlers)() };
            forget();
            return Err("3Dconnexion's driver didn't take Tequila on as a client.".into());
        }
        if let Ok(mut sink) = SINK.lock() {
            if let Some(sink) = sink.as_mut() {
                sink.client = client;
            }
        }
        if let Some(button_mask) = api.button_mask {
            // SAFETY: `client` is the driver's own number for this client.
            unsafe { button_mask(client, ALL_BUTTONS) };
        }
        let mut looked = Instant::now();
        while running.load(Ordering::Relaxed) {
            // SAFETY: CoreFoundation's own constant, and a run loop turned
            // on its own thread.
            let turned = unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0) };
            // Nothing in this thread's run loop: the messages come on the
            // main thread's, so wait for them here instead.
            let wait = if turned == RUN_LOOP_EMPTY { Duration::from_millis(50) } else { Duration::ZERO };
            if let Ok(event) = received.recv_timeout(wait) {
                send(event);
            }
            for event in received.try_iter() {
                send(event);
            }
            if looked.elapsed() >= Duration::from_secs(2) {
                looked = Instant::now();
                if !present() {
                    break;
                }
            }
        }
        // SAFETY: this client, and the handlers installed above.
        unsafe {
            (api.unregister)(client);
            (api.cleanup_handlers)();
        }
        forget();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::AtomicU32;

        // A stand-in for the driver's framework, as it behaves: it keeps
        // the handlers it is given, numbers its client, and later sends
        // states through the handler.
        static HANDLER: Mutex<Option<MessageHandler>> = Mutex::new(None);
        static REGISTERED_NAME: Mutex<Vec<u8>> = Mutex::new(Vec::new());
        static CALLS: AtomicU32 = AtomicU32::new(0);
        const CLIENT: u16 = 7;

        unsafe extern "C" fn set_handlers(message: MessageHandler, _: DeviceHandler, _: DeviceHandler, _separate: u8) -> i16 {
            *HANDLER.lock().unwrap() = Some(message);
            0
        }
        unsafe extern "C" fn cleanup() {
            *HANDLER.lock().unwrap() = None;
            CALLS.fetch_or(1, Ordering::SeqCst);
        }
        // What the stand-in was asked, kept for the test to look at: a
        // panic inside it would abort the tests rather than fail one.
        static REGISTERED_AS: Mutex<Option<(u32, u16, u32)>> = Mutex::new(None);
        unsafe extern "C" fn register(signature: u32, name: *const u8, mode: u16, mask: u32) -> u16 {
            *REGISTERED_AS.lock().unwrap() = Some((signature, mode, mask));
            let len = *name as usize;
            *REGISTERED_NAME.lock().unwrap() = std::slice::from_raw_parts(name.add(1), len).to_vec();
            CLIENT
        }
        unsafe extern "C" fn button_mask(client: u16, mask: u32) {
            if (client, mask) == (CLIENT, 0xFFFF_FFFF) {
                CALLS.fetch_or(2, Ordering::SeqCst);
            }
        }
        unsafe extern "C" fn unregister(client: u16) {
            if client == CLIENT {
                CALLS.fetch_or(4, Ordering::SeqCst);
            }
        }

        /// The whole way through a stand-in for the driver: Tequila is
        /// registered by its executable's name, the states addressed to it
        /// arrive as events while another client's are passed over, and
        /// stopping lets go of the client and the handlers.
        #[test]
        fn states_from_the_driver_arrive_as_events() {
            let api = Api {
                set_handlers: Some(set_handlers),
                install_handlers: None,
                cleanup_handlers: cleanup,
                register,
                button_mask: Some(button_mask),
                unregister,
            };
            let running = std::sync::Arc::new(AtomicBool::new(true));
            let reader = {
                let running = running.clone();
                std::thread::spawn(move || {
                    let mut events = Vec::new();
                    let result = run(&api, &running, &mut || true, &mut |e| events.push(e));
                    (result, events)
                })
            };
            let handler = loop {
                if let Some(h) = *HANDLER.lock().unwrap() {
                    if SINK.lock().unwrap().as_ref().is_some_and(|s| s.client == CLIENT) {
                        break h;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let send = |s: State| handler(0xC635, code(b"3dSR"), s.as_ptr() as *mut c_void);
            send(state(CLIENT, 3, [700, 0, 0, 0, 0, 0], 0));
            send(state(3, 3, [0, 0, 350, 0, 0, 0], 0));
            send(state(CLIENT, 2, [0; 6], 0b10));
            handler(0xC635, code(b"3dPC"), std::ptr::null_mut());
            std::thread::sleep(Duration::from_millis(300));
            running.store(false, Ordering::Relaxed);
            let (result, events) = reader.join().unwrap();
            assert_eq!(result, Ok(()));
            let Some(Event::Motion(m)) = events.first() else { panic!("no motion: {events:?}") };
            assert_eq!(m.translation, [2.0, 0.0, 0.0], "twice full deflection, as a driver set fast sends it");
            assert_eq!(events.get(1), Some(&Event::Buttons(0b10)));
            assert_eq!(events.len(), 2, "another client's state is not read: {events:?}");
            let expected = std::env::current_exe().unwrap().file_name().unwrap().to_str().unwrap().as_bytes().to_vec();
            assert_eq!(*REGISTERED_NAME.lock().unwrap(), expected);
            assert_eq!(*REGISTERED_AS.lock().unwrap(), Some((code(b"****"), 1, 0x3FFF)), "any application by its name, the buttons taken over, all six axes");
            assert_eq!(CALLS.load(Ordering::SeqCst), 7, "the buttons asked for, the client and the handlers let go");
            assert!(SINK.lock().unwrap().is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The driver's state record read as the device's own axes: X right,
    /// Y toward the person, Z down; only a state meant for this client;
    /// buttons from the full mask.
    #[test]
    fn a_state_reads_as_the_hid_reports_do() {
        let Some(Event::Motion(m)) = decode(&state(5, 3, [350, 175, -350, 0, 0, 175], 0), 5) else { panic!() };
        assert_eq!(m.translation, [1.0, 1.0, 0.5], "right, lifted, and pulled halfway toward the person");
        assert_eq!(m.rotation, [0.0, -0.5, 0.0], "a clockwise twist, seen from above");
        assert_eq!(decode(&state(5, 3, [350, 0, 0, 0, 0, 0], 0), 6), None, "another client's");
        assert_eq!(decode(&state(5, 2, [0; 6], 0x103), 5), Some(Event::Buttons(0x103)));
        assert_eq!(decode(&state(5, 1, [350, 0, 0, 0, 0, 0], 0), 5), None, "raw data is not read");
        let Some(Event::Motion(fast)) = decode(&state(5, 3, [i16::MAX, 0, 0, 0, 0, 0], 0), 5) else { panic!() };
        assert_eq!(fast.translation[0], DRIVER_LIMIT, "a driver set very fast is held, further out than the device's own reports");
    }

    /// The driver knows the application by its executable's name, as a
    /// Pascal string.
    #[test]
    fn the_client_is_named_after_the_executable() {
        assert_eq!(client_name(Some(std::path::Path::new("/Applications/Tequila.app/Contents/MacOS/Tequila"))), b"\x07Tequila");
        assert_eq!(client_name(None), b"\x07Tequila");
        let long = "x".repeat(300);
        let name = client_name(Some(std::path::Path::new(&long)));
        assert_eq!((name[0], name.len()), (255, 256));
    }
}

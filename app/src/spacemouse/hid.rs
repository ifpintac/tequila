//! The device read directly, as a HID device, on Windows and on a Mac
//! (`hidapi`: Windows' own HID calls, and IOKit).
//!
//! On Windows any number of programs may read a HID device at once, each
//! getting every report, so this reads alongside 3Dconnexion's own driver
//! (3DxWare) when it is installed. On a Mac 3DxWare takes the device for
//! itself, and nothing else can read it while it runs; without 3DxWare,
//! this opens the device for itself in turn - which also stops macOS
//! moving the pointer with it.

use super::report::{device_name, Decoder, Event};

/// Read devices until told to stop: the first 3D mouse found, opened, read
/// until it goes, and looked for again every two seconds meanwhile. Each
/// event is handed to `send`, and `status` told what is being read.
pub fn run(running: &std::sync::atomic::AtomicBool, send: &mut dyn FnMut(Event), status: &mut dyn FnMut(Option<String>)) {
    use std::sync::atomic::Ordering;
    let mut api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(error) => {
            status(Some(format!("HID unavailable: {error}")));
            return;
        }
    };
    while running.load(Ordering::Relaxed) {
        let _ = api.refresh_devices();
        let found = api.device_list().find_map(|info| {
            device_name(info.vendor_id(), info.product_id(), Some((info.usage_page(), info.usage()))).map(|name| (name, info.clone()))
        });
        let Some((name, info)) = found else {
            status(None);
            sleep_while(running, 2000);
            continue;
        };
        let device = match info.open_device(&api) {
            Ok(device) => device,
            Err(error) => {
                status(Some(format!("{name} could not be opened: {error}")));
                sleep_while(running, 2000);
                continue;
            }
        };
        status(Some(name));
        let mut decoder = Decoder::default();
        let mut buffer = [0u8; 64];
        while running.load(Ordering::Relaxed) {
            match device.read_timeout(&mut buffer, 250) {
                Ok(0) => {}
                Ok(n) => {
                    if let Some(event) = decoder.feed(&buffer[..n]) {
                        send(event);
                    }
                }
                // Unplugged, or asleep: let go of what it held, and look
                // for it again.
                Err(_) => break,
            }
        }
        for event in decoder.reset() {
            send(event);
        }
        status(None);
    }
}

fn sleep_while(running: &std::sync::atomic::AtomicBool, millis: u64) {
    for _ in 0..millis / 100 {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

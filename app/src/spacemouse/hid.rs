//! The device read directly, as a HID device, on Windows and on a Mac
//! (`hidapi`: Windows' own HID calls, and IOKit).
//!
//! On Windows any number of programs may read a HID device at once, each
//! getting every report, so this reads alongside 3Dconnexion's own driver
//! (3DxWare) when it is installed. On a Mac 3DxWare takes the device for
//! itself, and nothing else can open it while it runs: the device is then
//! read through the driver ([`super::connexion`]), this still looking every
//! two seconds whether it is plugged in. Without 3DxWare, this opens the
//! device for itself in turn - which also stops macOS moving the pointer
//! with it.

use super::report::{device_name, Decoder, Event};
use super::OpenProblem;

/// Read devices until told to stop: the first 3D mouse found, opened, read
/// until it goes, and looked for again every two seconds meanwhile. Each
/// event is handed to `send`, `status` told what is being read, and
/// `problem` why a device that is there could not be opened, or that it
/// could.
pub fn run(
    running: &std::sync::atomic::AtomicBool,
    send: &mut dyn FnMut(Event),
    status: &mut dyn FnMut(Option<String>),
    problem: &mut dyn FnMut(Option<OpenProblem>),
) {
    use std::sync::atomic::Ordering;
    let mut api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(error) => {
            eprintln!("Tequila: HID is unavailable, so no 3D mouse can be read: {error}");
            problem(Some(OpenProblem { message: format!("3D mice can't be read here: HID is unavailable ({error})."), details: None }));
            return;
        }
    };
    // The last error written to the log, so a retry does not write it again.
    let mut logged: Option<String> = None;
    while running.load(Ordering::Relaxed) {
        let _ = api.refresh_devices();
        let found = api.device_list().find_map(|info| {
            device_name(info.vendor_id(), info.product_id(), Some((info.usage_page(), info.usage()))).map(|name| (name, info.clone()))
        });
        let Some((name, info)) = found else {
            status(None);
            problem(None);
            sleep_while(running, 2000);
            continue;
        };
        let device = match info.open_device(&api) {
            Ok(device) => device,
            Err(error) => {
                let error = error.to_string();
                // On a Mac, 3Dconnexion's driver holding the device: read
                // it through the driver, for as long as it stays plugged in.
                #[cfg(target_os = "macos")]
                let error = if super::held_by_driver(&error) {
                    match through_driver(&mut api, &info, &name, running, send, status, problem) {
                        Ok(()) => {
                            logged = None;
                            continue;
                        }
                        Err(why) => format!("{why}\n{error}"),
                    }
                } else {
                    error
                };
                // Tried again every two seconds, and said once: the log
                // keeps hidapi's own words for whoever reads it.
                if logged.as_deref() != Some(error.as_str()) {
                    eprintln!("Tequila: the 3D mouse ({name}) could not be opened: {error}");
                    logged = Some(error.clone());
                }
                status(None);
                problem(Some(OpenProblem::of(&name, &error)));
                sleep_while(running, 2000);
                continue;
            }
        };
        logged = None;
        problem(None);
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

/// The device, held by 3Dconnexion's driver, read through the driver until
/// it is unplugged or reading stops. `Err` says why the driver couldn't be
/// used, for the person.
#[cfg(target_os = "macos")]
fn through_driver(
    api: &mut hidapi::HidApi,
    info: &hidapi::DeviceInfo,
    name: &str,
    running: &std::sync::atomic::AtomicBool,
    send: &mut dyn FnMut(Event),
    status: &mut dyn FnMut(Option<String>),
    problem: &mut dyn FnMut(Option<OpenProblem>),
) -> Result<(), String> {
    let driver = super::connexion::api()?;
    problem(None);
    status(Some(format!("{name}{}", super::connexion::THROUGH_DRIVER)));
    let path = info.path().to_owned();
    let mut present = || {
        let _ = api.refresh_devices();
        api.device_list().any(|d| d.path() == path.as_c_str())
    };
    let read = super::connexion::run(&driver, running, &mut present, send);
    // At rest, whatever the driver last said.
    send(Event::Motion(Default::default()));
    send(Event::Buttons(0));
    status(None);
    read
}

fn sleep_while(running: &std::sync::atomic::AtomicBool, millis: u64) {
    for _ in 0..millis / 100 {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

//! spacenavd, the free 3D mouse daemon on Linux (and on a Mac, where it is
//! installed): its socket, read as libspnav reads it.
//!
//! The daemon listens on a Unix socket - `$SPNAV_SOCKET`, else
//! `$XDG_RUNTIME_DIR/spnav.sock`, else `/var/run/spnav.sock` - and sends
//! each event as eight 32-bit integers in the machine's own byte order:
//!
//! - type 0, motion: then x, y, z, rx, ry, rz, and the milliseconds since
//!   the last motion event;
//! - type 1 and 2, a button pressed and released: then its number.
//!
//! That is the protocol's first version, which every spacenavd speaks to a
//! client that asks for nothing else: this one never sends the request to
//! change it. The axes are the device's own, as its HID reports have them,
//! unless the daemon's own configuration swaps or inverts them; so they go
//! through the same [`to_view`](super::report::to_view).

// Read on Linux and macOS; Windows has no spacenavd.
#![cfg_attr(not(unix), allow(dead_code))]

use super::report::{to_view, Event};

/// One packet, in bytes.
pub const PACKET: usize = 32;

/// What a packet says; `None` for one this does not read.
pub fn decode(packet: &[u8; PACKET]) -> Option<SpnavEvent> {
    let word = |i: usize| i32::from_ne_bytes([packet[i * 4], packet[i * 4 + 1], packet[i * 4 + 2], packet[i * 4 + 3]]);
    match word(0) {
        0 => Some(SpnavEvent::Motion([word(1), word(2), word(3), word(4), word(5), word(6)])),
        1 | 2 => {
            let button = word(1);
            (0..32).contains(&button).then_some(SpnavEvent::Button(button as u8, word(0) == 1))
        }
        _ => None,
    }
}

/// A packet's meaning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpnavEvent {
    /// The six axes, in the device's frame and units.
    Motion([i32; 6]),
    /// A button by number, pressed or released.
    Button(u8, bool),
}

/// The daemon's events as the HID reader's: motion in the view's frame,
/// and buttons as a mask, kept across packets.
#[derive(Default)]
pub struct Reader {
    buttons: u32,
}

impl Reader {
    pub fn event(&mut self, event: SpnavEvent) -> Event {
        match event {
            SpnavEvent::Motion(axes) => Event::Motion(to_view(axes)),
            SpnavEvent::Button(button, pressed) => {
                if pressed {
                    self.buttons |= 1 << button;
                } else {
                    self.buttons &= !(1 << button);
                }
                Event::Buttons(self.buttons)
            }
        }
    }
}

/// Where the daemon's socket may be, in the order libspnav tries them.
pub fn socket_paths() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = std::env::var_os("SPNAV_SOCKET").filter(|p| !p.is_empty()) {
        out.push(path.into());
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty()) {
        out.push(std::path::PathBuf::from(runtime).join("spnav.sock"));
    }
    out.push("/var/run/spnav.sock".into());
    out
}

/// Read the daemon until told to stop, connecting again whenever it goes:
/// each event handed to `send`, and `status` told what is being read.
#[cfg(unix)]
pub fn run(running: &std::sync::atomic::AtomicBool, send: &mut dyn FnMut(Event), status: &mut dyn FnMut(Option<String>)) {
    use std::io::Read;
    use std::sync::atomic::Ordering;
    while running.load(Ordering::Relaxed) {
        let connected = socket_paths().into_iter().find_map(|path| std::os::unix::net::UnixStream::connect(&path).ok().map(|s| (s, path)));
        let Some((mut stream, path)) = connected else {
            status(None);
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        };
        // A timeout, so a quiet daemon still lets the thread see it is to
        // stop.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(250)));
        status(Some(format!("spacenavd ({})", path.display())));
        let mut reader = Reader::default();
        let mut packet = [0u8; PACKET];
        let mut have = 0;
        while running.load(Ordering::Relaxed) {
            match stream.read(&mut packet[have..]) {
                Ok(0) => break,
                Ok(n) => {
                    have += n;
                    if have == PACKET {
                        have = 0;
                        if let Some(event) = decode(&packet) {
                            send(reader.event(event));
                        }
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => {}
                Err(_) => break,
            }
        }
        // Gone: whatever it held is let go.
        send(Event::Motion(Default::default()));
        send(Event::Buttons(0));
        status(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(words: [i32; 8]) -> [u8; PACKET] {
        let mut out = [0u8; PACKET];
        for (i, w) in words.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_ne_bytes());
        }
        out
    }

    /// A motion packet reads as the same motion as the HID reports with
    /// those numbers, and buttons keep a mask across packets.
    #[test]
    fn the_daemons_packets_read_as_the_devices_reports() {
        let motion = decode(&packet([0, 350, 0, -175, 0, 0, 175, 16])).expect("motion");
        assert_eq!(motion, SpnavEvent::Motion([350, 0, -175, 0, 0, 175]));
        let mut reader = Reader::default();
        let Event::Motion(m) = reader.event(motion) else { panic!() };
        assert_eq!(m.translation, [1.0, 0.5, 0.0], "right, lifted halfway");
        assert_eq!(m.rotation, [0.0, -0.5, 0.0], "twisted clockwise from above");
        let pressed = decode(&packet([1, 1, 0, 0, 0, 0, 0, 0])).expect("a press");
        assert_eq!(reader.event(pressed), Event::Buttons(0b10));
        assert_eq!(reader.event(decode(&packet([1, 0, 0, 0, 0, 0, 0, 0])).unwrap()), Event::Buttons(0b11));
        assert_eq!(reader.event(decode(&packet([2, 1, 0, 0, 0, 0, 0, 0])).unwrap()), Event::Buttons(0b01));
        assert_eq!(decode(&packet([3, 0, 0, 0, 0, 0, 0, 0])), None, "a device event of the newer protocol");
        assert_eq!(decode(&packet([1, 99, 0, 0, 0, 0, 0, 0])), None, "no such button");
    }

    #[test]
    fn the_socket_is_looked_for_where_libspnav_looks() {
        let paths = socket_paths();
        assert_eq!(paths.last().map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/var/run/spnav.sock"));
    }
}

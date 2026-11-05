//! What a 3Dconnexion device sends, read without 3Dconnexion's SDK: its
//! raw HID input reports, as the open-source readers of these devices read
//! them, and the motion in them turned to the view's own axes.
//!
//! The reports (USB HID, a Generic Desktop "multi-axis controller", usage
//! page 1, usage 8):
//!
//! - **Report 1, 6 bytes after the id**: translation, X, Y and Z, each a
//!   signed 16-bit little-endian number. The older devices (SpaceNavigator,
//!   SpaceExplorer, SpacePilot, the wired SpaceMouse Pro and Compact) send
//!   rotation separately, in **report 2**, the same shape: Rx, Ry, Rz.
//! - **Report 1, 12 bytes after the id**: all six at once, translation then
//!   rotation. The newer wireless devices and the Enterprise send this, and
//!   nothing in report 2. Told apart by the report's length.
//! - **Report 3**: the buttons, one bit each, little-endian, bit 0 the left
//!   button (Menu on the larger devices) and bit 1 the right (Fit).
//! - Anything else (battery, LEDs, keyboard keys of the Enterprise) is left
//!   alone.
//!
//! The device's own axes: X to the right, Y toward the person, Z down -
//! pushing the cap down is +Z - and each rotation right-handed about them.
//! Full deflection is about 350 either way. [`to_view`] turns that into the
//! view's axes, x right, y up and z out of the screen, as the cap moves the
//! model: [`Motion`].

// The device table is the HID reader's, which Linux does without.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

/// 3Dconnexion's own USB vendor id, and the one its devices had while the
/// company was part of Logitech.
pub const VENDOR_3DCONNEXION: u16 = 0x256F;
pub const VENDOR_LOGITECH: u16 = 0x046D;

/// The HID usage of a multi-axis controller: Generic Desktop, Multi-axis.
pub const USAGE_PAGE: u16 = 0x01;
pub const USAGE: u16 = 0x08;

/// About full deflection, in the device's units.
pub const FULL_SCALE: f32 = 350.0;

/// The 3Dconnexion devices sold under Logitech's vendor id, by product id;
/// Logitech's own mice and keyboards share the id, so these are named. The
/// ones under 3Dconnexion's id are all theirs, told by their HID usage.
const LOGITECH_ERA: [(u16, &str); 12] = [
    (0xC603, "SpaceMouse Plus XT"),
    (0xC605, "CadMan"),
    (0xC606, "SpaceMouse Classic"),
    (0xC621, "SpaceBall 5000"),
    (0xC623, "SpaceTraveller"),
    (0xC625, "SpacePilot"),
    (0xC626, "SpaceNavigator"),
    (0xC627, "SpaceExplorer"),
    (0xC628, "SpaceNavigator for Notebooks"),
    (0xC629, "SpacePilot Pro"),
    (0xC62B, "SpaceMouse Pro"),
    (0xC640, "nulooq"),
];

/// Names for the devices under 3Dconnexion's own id, for the preferences
/// to say what is plugged in; an unnamed one is read all the same.
const OWN: [(u16, &str); 13] = [
    (0xC62E, "SpaceMouse Wireless (cabled)"),
    (0xC62F, "SpaceMouse Wireless"),
    (0xC631, "SpaceMouse Pro Wireless (cabled)"),
    (0xC632, "SpaceMouse Pro Wireless"),
    (0xC633, "SpaceMouse Enterprise"),
    (0xC635, "SpaceMouse Compact"),
    (0xC636, "SpaceMouse Module"),
    (0xC638, "SpaceMouse Pro Wireless (Bluetooth)"),
    (0xC63A, "SpaceMouse Wireless"),
    (0xC641, "SpaceMouse"),
    (0xC652, "Universal Receiver"),
    (0xC658, "Universal Receiver"),
    (0xC62D, "SpaceMouse"),
];

/// Whether a HID device is a 3Dconnexion 3D mouse to read, and what to call
/// it. `usage` is its usage page and usage, where the system says; a device
/// is read only on its multi-axis interface, since a receiver or an
/// Enterprise has others (its keyboard keys, its battery).
pub fn device_name(vendor: u16, product: u16, usage: Option<(u16, u16)>) -> Option<String> {
    if usage.is_some_and(|u| u != (USAGE_PAGE, USAGE)) {
        return None;
    }
    match vendor {
        VENDOR_LOGITECH => LOGITECH_ERA.iter().find(|(id, _)| *id == product).map(|(_, name)| name.to_string()),
        VENDOR_3DCONNEXION => Some(
            OWN.iter()
                .find(|(id, _)| *id == product)
                .map_or_else(|| format!("3Dconnexion device {product:04X}"), |(_, name)| name.to_string()),
        ),
        _ => None,
    }
}

/// The six axes in the view's frame, as fractions of full deflection: the
/// way the cap moves the model. `translation` is x right, y up, z out of
/// the screen; `rotation` is right-handed about the same axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
}

impl Motion {
    /// All six, translation first.
    pub fn axes(&self) -> [f32; 6] {
        let [x, y, z] = self.translation;
        let [rx, ry, rz] = self.rotation;
        [x, y, z, rx, ry, rz]
    }

    #[cfg(test)]
    pub fn is_rest(&self) -> bool {
        self.axes().iter().all(|v| *v == 0.0)
    }
}

/// The device's six numbers - X right, Y toward the person, Z down, and
/// rotations about them - as the view's motion. X stays x; down is -y; and
/// toward the person is out of the screen, +z. The rotations follow their
/// axes: about Z (down) is minus about y (up).
pub fn to_view(device: [i32; 6]) -> Motion {
    let f = |v: i32| (v as f32 / FULL_SCALE).clamp(-1.5, 1.5);
    let [x, y, z, rx, ry, rz] = device;
    Motion { translation: [f(x), -f(z), f(y)], rotation: [f(rx), -f(rz), f(ry)] }
}

/// What a report says.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The cap has moved: the motion as it now stands.
    Motion(Motion),
    /// Buttons, as a mask: bit 0 the left button, bit 1 the right.
    Buttons(u32),
}

/// Reads one device's reports, keeping what an older device sends in two
/// reports so each report gives the whole motion.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    device: [i32; 6],
    buttons: u32,
}

fn int16(bytes: &[u8], at: usize) -> i32 {
    i16::from_le_bytes([bytes[at], bytes[at + 1]]) as i32
}

impl Decoder {
    /// One input report, its id first. `None` for a report that changes
    /// nothing this reads: an unknown id, a short report, the buttons as
    /// they were.
    pub fn feed(&mut self, report: &[u8]) -> Option<Event> {
        let (&id, body) = report.split_first()?;
        match id {
            1 if body.len() >= 12 => {
                for axis in 0..6 {
                    self.device[axis] = int16(body, axis * 2);
                }
                Some(Event::Motion(to_view(self.device)))
            }
            1 | 2 if body.len() >= 6 => {
                let first = if id == 1 { 0 } else { 3 };
                for axis in 0..3 {
                    self.device[first + axis] = int16(body, axis * 2);
                }
                Some(Event::Motion(to_view(self.device)))
            }
            3 if !body.is_empty() => {
                let mut mask = [0u8; 4];
                let n = body.len().min(4);
                mask[..n].copy_from_slice(&body[..n]);
                let buttons = u32::from_le_bytes(mask);
                (buttons != self.buttons).then(|| {
                    self.buttons = buttons;
                    Event::Buttons(buttons)
                })
            }
            _ => None,
        }
    }

    /// The device put back at rest, as when it is unplugged: what that
    /// changes.
    pub fn reset(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if self.device != [0; 6] {
            self.device = [0; 6];
            out.push(Event::Motion(Motion::default()));
        }
        if self.buttons != 0 {
            self.buttons = 0;
            out.push(Event::Buttons(0));
        }
        out
    }
}

/// A report as a device would send it, for tests: `id`, then each value
/// as a signed 16-bit little-endian number.
#[cfg(test)]
pub fn report(id: u8, values: &[i16]) -> Vec<u8> {
    let mut out = vec![id];
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The devices are told by vendor, product and interface: every one of
    /// 3Dconnexion's own on its multi-axis interface, only the named ones
    /// under Logitech's id, and never a keyboard or a receiver's other
    /// interfaces.
    #[test]
    fn the_3dconnexion_devices_and_only_they_are_read() {
        assert_eq!(device_name(0x046D, 0xC626, Some((1, 8))).as_deref(), Some("SpaceNavigator"));
        assert_eq!(device_name(0x046D, 0xC62B, None).as_deref(), Some("SpaceMouse Pro"));
        assert_eq!(device_name(0x256F, 0xC635, Some((1, 8))).as_deref(), Some("SpaceMouse Compact"));
        assert_eq!(device_name(0x256F, 0xC6FF, Some((1, 8))).as_deref(), Some("3Dconnexion device C6FF"), "a new one is read all the same");
        assert_eq!(device_name(0x046D, 0xC52B, Some((1, 8))), None, "a Logitech receiver is no 3D mouse");
        assert_eq!(device_name(0x256F, 0xC652, Some((1, 6))), None, "a receiver's keyboard interface is not read");
        assert_eq!(device_name(0x1234, 0xC626, Some((1, 8))), None);
    }

    /// An older device's two reports make one motion, each axis where it
    /// belongs: pushing right is +x, pulling toward the person is out of
    /// the screen, pressing down is -y, and twisting clockwise from above
    /// turns the model clockwise, about -y.
    #[test]
    fn translation_and_rotation_reports_make_one_motion() {
        let mut decoder = Decoder::default();
        let Some(Event::Motion(m)) = decoder.feed(&report(1, &[350, 175, -350])) else { panic!("no motion") };
        assert_eq!(m.translation, [1.0, 1.0, 0.5], "right, lifted, and pulled halfway toward the person");
        assert_eq!(m.rotation, [0.0, 0.0, 0.0]);
        let Some(Event::Motion(m)) = decoder.feed(&report(2, &[0, 0, 175])) else { panic!("no motion") };
        assert_eq!(m.translation, [1.0, 1.0, 0.5], "the translation is kept while the rotation comes");
        assert_eq!(m.rotation, [0.0, -0.5, 0.0], "a clockwise twist, seen from above");
        let Some(Event::Motion(m)) = decoder.feed(&report(2, &[-350, 70, 0])) else { panic!("no motion") };
        assert_eq!(m.rotation, [-1.0, 0.0, 0.2], "tilted forward, rolled a little to the left");
    }

    /// The newer devices' one report of all six reads the same as the two.
    #[test]
    fn a_combined_report_carries_all_six() {
        let mut two = Decoder::default();
        two.feed(&report(1, &[10, -20, 30]));
        let Some(Event::Motion(split)) = two.feed(&report(2, &[-40, 50, -60])) else { panic!() };
        let mut one = Decoder::default();
        let Some(Event::Motion(combined)) = one.feed(&report(1, &[10, -20, 30, -40, 50, -60])) else { panic!() };
        assert_eq!(split, combined);
        // Back at rest.
        let Some(Event::Motion(rest)) = one.feed(&report(1, &[0; 6])) else { panic!() };
        assert!(rest.is_rest());
    }

    /// Buttons come as a mask, said only when it changes; a short or
    /// unknown report changes nothing; unplugging lets go of everything.
    #[test]
    fn buttons_are_a_mask_said_when_it_changes() {
        let mut decoder = Decoder::default();
        assert_eq!(decoder.feed(&[3, 0b10, 0]), Some(Event::Buttons(2)), "the right button, Fit");
        assert_eq!(decoder.feed(&[3, 0b10, 0]), None, "the same again says nothing");
        assert_eq!(decoder.feed(&[3, 0, 1]), Some(Event::Buttons(0x100)), "a ninth button, in the second byte");
        assert_eq!(decoder.feed(&[3, 0, 0, 0, 0, 0, 0]), Some(Event::Buttons(0)));
        assert_eq!(decoder.feed(&[1, 1, 2]), None, "too short to be a report 1");
        assert_eq!(decoder.feed(&[0x17, 80]), None, "a battery report");
        assert_eq!(decoder.feed(&[]), None);
        decoder.feed(&report(1, &[100, 0, 0]));
        decoder.feed(&[3, 1]);
        assert_eq!(decoder.reset(), [Event::Motion(Motion::default()), Event::Buttons(0)]);
        assert_eq!(decoder.reset(), []);
    }

    /// Values past full deflection are kept within reason.
    #[test]
    fn far_past_full_deflection_is_clamped() {
        let m = to_view([i16::MAX as i32, i16::MIN as i32, 0, 0, 0, 0]);
        assert_eq!(m.translation, [1.5, 0.0, -1.5]);
    }
}

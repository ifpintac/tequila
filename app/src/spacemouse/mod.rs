//! A 3Dconnexion 3D mouse (SpaceMouse, SpaceNavigator and their kin):
//! turning, panning and zooming the view with one hand while the other
//! keeps the mouse, as CAD applications do (`docs/04` §6.1).
//!
//! No part of 3Dconnexion's SDK is used. The device is read where it can
//! be read openly:
//!
//! - **Windows and macOS**: its HID reports, on a thread of its own
//!   ([`hid`]). On Windows this works beside 3Dconnexion's driver.
//! - **A Mac with 3Dconnexion's driver**: the driver takes the device for
//!   itself, so it is read through the driver, by the framework the driver
//!   installed ([`connexion`]); the driver's own settings then apply.
//! - **Linux, and a Mac with spacenavd**: the free daemon's socket
//!   ([`spnav`]).
//!
//! The reader hands over what the device says ([`report`]); each frame the
//! viewport takes the motion, smoothed, through [`navigate`] into its orbit
//! camera, and keeps drawing while the cap is off centre. The two main
//! buttons do what the preferences say. Everything here was written and
//! tested against synthetic reports: no device was at hand.

pub mod connexion;
pub mod navigate;
pub mod report;
pub mod spnav;
#[cfg(any(windows, target_os = "macos"))]
mod hid;

use navigate::{ButtonAction, Pivot, Settings, Smoothed};
use report::{Decoder, Event, Motion};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A motion not repeated or put back at rest for this long is taken as
/// ended: a lost report must not leave the view turning.
const STALE: std::time::Duration = std::time::Duration::from_secs(2);

/// What the reader has seen, for the frame to take.
#[derive(Default)]
struct Shared {
    motion: Motion,
    at: Option<std::time::Instant>,
    /// Button masks not yet handed to the frame, oldest first.
    pressed: Vec<u32>,
    /// The device being read, or `None` while there is none.
    device: Option<String>,
    /// Why a 3D mouse that is there is not being read, when one is not.
    problem: Option<OpenProblem>,
}

/// What 3Dconnexion's driver holding the device on a Mac comes to, in the
/// preferences, when Tequila couldn't read it through the driver either.
pub const HELD_BY_DRIVER: &str = "3Dconnexion's driver is using the 3D mouse, and Tequila couldn't connect to the driver. Updating 3DxWare may help; or quit 3DxWare, and Tequila reads the mouse itself.";

/// Why a 3D mouse was found and could not be read: what to tell the
/// person, and the system's own words, when they are not the message.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenProblem {
    pub message: String,
    pub details: Option<String>,
}

impl OpenProblem {
    /// Device `name` would not open, saying `error`.
    ///
    /// On a Mac, 3Dconnexion's driver (3DxWare) opens the device for
    /// itself, and IOKit refuses anyone else: hidapi passes on
    /// `kIOReturnExclusiveAccess`, 0xE00002C5, sometimes as its number and
    /// sometimes in words. That one is said plainly, with the raw text kept
    /// as details; anything else is said as it was.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    pub fn of(name: &str, error: &str) -> OpenProblem {
        if held_by_driver(error) {
            OpenProblem { message: HELD_BY_DRIVER.into(), details: Some(error.to_string()) }
        } else {
            OpenProblem { message: format!("{name} could not be opened: {error}"), details: None }
        }
    }
}

/// Whether an open's error says another program has the device to itself.
pub fn held_by_driver(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("e00002c5") || error.contains("exclusive access") || error.contains("exclusiveaccess")
}

/// A synthetic motion from the scripting channel: held until a moment.
struct Held {
    motion: Motion,
    until: std::time::Instant,
}

/// What a frame asks of the application, besides moving the camera.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The view moved this frame.
    pub moved: bool,
    /// A motion has just come to rest: one gesture, one step to undo.
    pub ended: bool,
    /// A button was pressed whose action is this.
    pub action: Option<ButtonAction>,
}

/// The 3D mouse, from its reader to the camera.
pub struct SpaceMouse {
    pub settings: Settings,
    shared: Arc<Mutex<Shared>>,
    running: Arc<AtomicBool>,
    started: bool,
    smoothed: Smoothed,
    moving: bool,
    /// The point turning goes about, chosen when a motion starts.
    pivot: Option<tequila_core::math::Vec3>,
    held: Option<Held>,
    /// Buttons as they were last frame.
    buttons: u32,
    /// The preferences window, open.
    pub preferences_open: bool,
}

impl Default for SpaceMouse {
    fn default() -> Self {
        SpaceMouse {
            settings: Settings::default(),
            shared: Arc::default(),
            running: Arc::new(AtomicBool::new(true)),
            started: false,
            smoothed: Smoothed::default(),
            moving: false,
            pivot: None,
            held: None,
            buttons: 0,
            preferences_open: false,
        }
    }
}

impl Drop for SpaceMouse {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl SpaceMouse {
    /// With the person's settings.
    pub fn load() -> SpaceMouse {
        let mut mouse = SpaceMouse::default();
        mouse.settings = load_settings();
        mouse
    }

    /// Start reading, once, if the settings allow: on a thread of its own,
    /// waking the window whenever the device says something.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.started || !self.settings.enabled {
            return;
        }
        self.started = true;
        let shared = self.shared.clone();
        let running = self.running.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("tequila-3d-mouse".into()).spawn(move || read(&running, &shared, &ctx));
    }

    /// The device being read, if any.
    pub fn device(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.device.clone())
    }

    /// Why a 3D mouse that is there is not being read, if one is not.
    pub fn problem(&self) -> Option<OpenProblem> {
        self.shared.lock().ok().and_then(|s| s.problem.clone())
    }

    /// The axes as the view takes them now, smoothed, for the preferences
    /// to show.
    pub fn axes(&self) -> [f32; 6] {
        self.smoothed.axes
    }

    /// Feed reports as a device would send them, for the scripting
    /// channel: the motion they end on held for `seconds`, then let go;
    /// buttons pressed and released at once.
    pub fn inject(&mut self, reports: &[Vec<u8>], seconds: f32) {
        let mut decoder = Decoder::default();
        let mut motion = None;
        for r in reports {
            match decoder.feed(r) {
                Some(Event::Motion(m)) => motion = Some(m),
                Some(Event::Buttons(mask)) => {
                    if let Ok(mut shared) = self.shared.lock() {
                        shared.pressed.push(mask);
                        shared.pressed.push(0);
                    }
                }
                None => {}
            }
        }
        if let Some(motion) = motion {
            self.held = Some(Held { motion, until: std::time::Instant::now() + std::time::Duration::from_secs_f32(seconds.clamp(0.0, 30.0)) });
        }
    }

    /// One frame: the device's motion, smoothed, into `camera` over `dt`
    /// seconds; a newly pressed button's action. `pivot` is asked for the
    /// point to turn about when a motion starts. Keeps the window drawing
    /// while anything moves.
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        camera: &mut crate::orbit::OrbitCamera,
        dt: f32,
        aspect: f32,
        pivot: impl FnOnce() -> Option<tequila_core::math::Vec3>,
    ) -> Frame {
        let mut out = Frame::default();
        let now = std::time::Instant::now();
        let (device, masks) = match self.shared.lock() {
            Ok(mut shared) => {
                let fresh = shared.at.is_some_and(|at| now.duration_since(at) < STALE);
                let motion = if fresh { shared.motion } else { Motion::default() };
                (motion, std::mem::take(&mut shared.pressed))
            }
            Err(_) => (Motion::default(), Vec::new()),
        };
        let held = match &self.held {
            Some(held) if now < held.until => Some(held.motion),
            Some(_) => {
                self.held = None;
                None
            }
            None => None,
        };
        let raw = held.unwrap_or(device);
        let target = if self.settings.enabled { self.settings.shape(raw.axes()) } else { [0.0; 6] };
        let moving = self.smoothed.follow(target, dt.clamp(0.0, 0.1));
        if moving {
            if !self.moving {
                self.pivot = match self.settings.pivot {
                    Pivot::Model => pivot(),
                    Pivot::View => None,
                };
            }
            let about = self.pivot.unwrap_or(camera.target);
            out.moved = navigate::drive(camera, self.smoothed.mean, dt.clamp(0.0, 0.1), aspect, about, self.settings.sensitivity);
            ctx.request_repaint();
        } else if self.moving {
            out.ended = true;
            self.pivot = None;
        }
        if self.held.is_some() {
            ctx.request_repaint();
        }
        self.moving = moving;
        // A button newly down: its action.
        for mask in masks {
            let pressed = mask & !self.buttons;
            self.buttons = mask;
            for (bit, action) in self.settings.buttons.iter().enumerate() {
                if pressed & (1 << bit) != 0 && *action != ButtonAction::Nothing && self.settings.enabled {
                    out.action = Some(*action);
                }
            }
        }
        out
    }

    /// The preferences window: the 3D mouse's settings, with what the
    /// device is doing shown live. Saved as they change.
    pub fn preferences(&mut self, ctx: &egui::Context) {
        if !self.preferences_open {
            return;
        }
        let mut open = true;
        let before = self.settings;
        let device = self.device();
        let problem = self.problem();
        let axes = self.axes();
        egui::Window::new("Preferences").open(&mut open).collapsible(false).resizable(false).default_width(360.0).show(ctx, |ui| {
            crate::theme::section(ui, "3D mouse");
            let p = crate::theme::p();
            match (&device, &problem, self.settings.enabled) {
                (_, _, false) => {
                    ui.label(egui::RichText::new("Off").small().color(crate::theme::dim()));
                }
                (Some(name), _, true) => {
                    ui.label(egui::RichText::new(format!("Connected: {name}")).small().color(p.success));
                    if name.ends_with(connexion::THROUGH_DRIVER) {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("3Dconnexion's own settings for Tequila apply as well as these.").small().color(crate::theme::dim()),
                            )
                            .wrap(),
                        );
                    }
                }
                (None, Some(problem), true) => {
                    ui.add(egui::Label::new(egui::RichText::new(&problem.message).small().color(p.busy)).wrap());
                    if let Some(details) = &problem.details {
                        egui::CollapsingHeader::new(egui::RichText::new("Details").small()).id_salt("3d-mouse-problem").show(ui, |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(details).small().monospace().color(crate::theme::dim())).wrap().selectable(true));
                        });
                    }
                }
                (None, None, true) => {
                    ui.label(egui::RichText::new("No 3D mouse found. Plug one in, or start spacenavd on Linux.").small().color(crate::theme::dim()));
                }
            }
            ui.add(crate::field::Checkbox::new(&mut self.settings.enabled, "Move the view with a 3D mouse"))
                .on_hover_text("A 3Dconnexion SpaceMouse, SpaceNavigator or the like: turn, pan and zoom with one hand while the other keeps the mouse.");
            ui.add_enabled_ui(self.settings.enabled, |ui| {
                egui::Grid::new("3d-mouse").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Speed");
                    ui.add(crate::field::Slider::new(&mut self.settings.sensitivity, 0.1..=4.0).logarithmic(true).fixed_decimals(2).suffix("×"))
                        .on_hover_text("How fast the view moves for a push of the cap.");
                    ui.end_row();
                    ui.label("Dead zone");
                    let mut percent = self.settings.dead_zone * 100.0;
                    if ui
                        .add(crate::field::Slider::new(&mut percent, 0.0..=40.0).fixed_decimals(0).suffix("%"))
                        .on_hover_text("How far the cap can move from the middle before the view moves: more for a cap that rests a little off centre.")
                        .changed()
                    {
                        self.settings.dead_zone = percent / 100.0;
                    }
                    ui.end_row();
                    ui.label("Turn about");
                    crate::theme::segmented(ui, &mut self.settings.pivot, &[(Pivot::Model, "Selection or model"), (Pivot::View, "View centre")]);
                    ui.end_row();
                });
                ui.add(crate::field::Checkbox::new(&mut self.settings.dominant, "Only the strongest axis"))
                    .on_hover_text("One move at a time: the axis pushed furthest moves the view and the rest wait, for a straight pan or a pure turn.");
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Axes").strong());
                ui.label(egui::RichText::new("The cap moves the model. The bars show what the view takes from it now.").small().color(crate::theme::dim()));
                egui::Grid::new("3d-mouse-axes").num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (i, name) in navigate::AXES.iter().enumerate() {
                        ui.label(*name);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, p.field);
                        let v = axes[i].clamp(-1.0, 1.0);
                        let middle = rect.center().x;
                        let end = middle + v * rect.width() * 0.5;
                        ui.painter().rect_filled(egui::Rect::from_x_y_ranges(middle.min(end)..=middle.max(end), rect.y_range()), 3.0, p.accent);
                        ui.painter().line_segment([egui::pos2(middle, rect.top()), egui::pos2(middle, rect.bottom())], egui::Stroke::new(1.0_f32, p.line_strong));
                        if i == 5 {
                            ui.label(egui::RichText::new("the horizon stays level").small().color(crate::theme::dim()))
                                .on_hover_text("The orbit camera keeps the horizon level, as the mouse's orbit does: the roll is shown and moves nothing.");
                        } else {
                            ui.add(crate::field::Checkbox::new(&mut self.settings.invert[i], "Invert"));
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Buttons").strong());
                egui::Grid::new("3d-mouse-buttons").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (i, name) in ["Left (Menu)", "Right (Fit)"].iter().enumerate() {
                        ui.label(*name);
                        egui::ComboBox::from_id_salt(("3d-mouse-button", i)).selected_text(self.settings.buttons[i].label()).show_ui(ui, |ui| {
                            for action in ButtonAction::ALL {
                                ui.selectable_value(&mut self.settings.buttons[i], action, action.label());
                            }
                        });
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            if ui.button("Defaults").clicked() {
                self.settings = Settings { enabled: self.settings.enabled, ..Settings::default() };
            }
        });
        if !open {
            self.preferences_open = false;
        }
        if self.settings != before {
            save_settings(&self.settings);
            if self.settings.enabled {
                self.start(ctx);
            }
        }
    }
}

/// The reader: the device's events into `shared`, the window woken for
/// each, until `running` goes false.
fn read(running: &AtomicBool, shared: &Mutex<Shared>, ctx: &egui::Context) {
    let mut send = |event: Event| {
        if let Ok(mut s) = shared.lock() {
            match event {
                Event::Motion(motion) => {
                    s.motion = motion;
                    s.at = Some(std::time::Instant::now());
                }
                Event::Buttons(mask) => s.pressed.push(mask),
            }
        }
        ctx.request_repaint();
    };
    let mut status = |device: Option<String>| {
        if let Ok(mut s) = shared.lock() {
            s.device = device;
        }
    };
    #[cfg(any(windows, target_os = "macos"))]
    let mut problem = |problem: Option<OpenProblem>| {
        if let Ok(mut s) = shared.lock() {
            s.problem = problem;
        }
    };
    // spacenavd first where there may be one: on Linux it is the way in,
    // and on a Mac it is the one that works beside nothing else.
    #[cfg(target_os = "linux")]
    spnav::run(running, &mut send, &mut status);
    #[cfg(target_os = "macos")]
    {
        if spnav::socket_paths().iter().any(|p| p.exists()) {
            spnav::run(running, &mut send, &mut status);
        } else {
            hid::run(running, &mut send, &mut status, &mut problem);
        }
    }
    #[cfg(windows)]
    hid::run(running, &mut send, &mut status, &mut problem);
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (running, &mut send, &mut status);
    }
}

// ---- The settings, kept beside the person's library --------------------------

fn settings_path() -> std::path::PathBuf {
    crate::library_tab::user_library_path()
        .parent()
        .map(|p| p.join("3d-mouse.txt"))
        .unwrap_or_else(|| std::env::temp_dir().join("tequila-3d-mouse.txt"))
}

fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path()).map(|t| settings_from_text(&t)).unwrap_or_default()
}

fn save_settings(settings: &Settings) {
    let path = settings_path();
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(path, settings_to_text(settings));
}

/// The settings as lines of `key=value`.
pub fn settings_to_text(s: &Settings) -> String {
    let invert: Vec<&str> = (0..6).filter(|&i| s.invert[i]).map(|i| ["pan_x", "pan_y", "zoom", "tilt", "spin", "roll"][i]).collect();
    format!(
        "enabled={}\nspeed={}\ndead_zone={}\ninvert={}\ndominant={}\npivot={}\nleft_button={}\nright_button={}\n",
        if s.enabled { "on" } else { "off" },
        s.sensitivity,
        s.dead_zone,
        invert.join(","),
        if s.dominant { "on" } else { "off" },
        match s.pivot {
            Pivot::Model => "model",
            Pivot::View => "view",
        },
        s.buttons[0].key(),
        s.buttons[1].key(),
    )
}

/// Read back; anything unknown or missing keeps its default.
pub fn settings_from_text(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "enabled" => s.enabled = value != "off",
            "speed" => s.sensitivity = value.parse().ok().filter(|v: &f32| v.is_finite()).map_or(s.sensitivity, |v| v.clamp(0.1, 4.0)),
            "dead_zone" => s.dead_zone = value.parse().ok().filter(|v: &f32| v.is_finite()).map_or(s.dead_zone, |v| v.clamp(0.0, 0.4)),
            "invert" => {
                s.invert = [false; 6];
                for name in value.split(',') {
                    if let Some(i) = ["pan_x", "pan_y", "zoom", "tilt", "spin", "roll"].iter().position(|n| *n == name.trim()) {
                        s.invert[i] = true;
                    }
                }
            }
            "dominant" => s.dominant = value == "on",
            "pivot" => s.pivot = if value == "view" { Pivot::View } else { Pivot::Model },
            "left_button" => s.buttons[0] = ButtonAction::from_key(value).unwrap_or(s.buttons[0]),
            "right_button" => s.buttons[1] = ButtonAction::from_key(value).unwrap_or(s.buttons[1]),
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    #[test]
    fn the_settings_are_kept_and_read_back() {
        let s = Settings {
            enabled: false,
            sensitivity: 2.5,
            dead_zone: 0.12,
            invert: [true, false, true, false, false, true],
            dominant: true,
            pivot: Pivot::View,
            buttons: [ButtonAction::FitView, ButtonAction::Nothing],
        };
        assert_eq!(settings_from_text(&settings_to_text(&s)), s);
        assert_eq!(settings_from_text("nonsense\nspeed=99\ndead_zone=NaN\n"), Settings { sensitivity: 4.0, ..Settings::default() });
        assert_eq!(settings_from_text(""), Settings::default());
    }

    /// The whole way from a device's reports to the camera, as the
    /// scripting channel's synthetic input goes: a twist held for half a
    /// second turns the view, keeps the window drawing, comes to rest as
    /// one gesture; a button press says its action once.
    #[test]
    fn reports_drive_the_camera_from_end_to_end() {
        let ctx = egui::Context::default();
        let mut mouse = SpaceMouse::default();
        let mut camera = crate::orbit::OrbitCamera::framing(vec3(0.0, 0.0, 0.0), 1.0, 1.5);
        let before = camera;
        // A clockwise twist, seen from above, at full deflection.
        mouse.inject(&[report::report(1, &[0, 0, 0, 0, 0, 350])], 0.5);
        // Frames as the window draws them while the twist is held, each
        // taking the time it really took; then a second of frames after it
        // is let go, however busy the machine running the test.
        let mut frames = Vec::new();
        let started = std::time::Instant::now();
        let mut last = started;
        while started.elapsed().as_secs_f32() < 0.6 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            let now = std::time::Instant::now();
            frames.push(mouse.frame(&ctx, &mut camera, (now - last).as_secs_f32(), 1.5, || Some(vec3(0.0, 0.0, 0.0))));
            last = now;
        }
        for _ in 0..60 {
            frames.push(mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || Some(vec3(0.0, 0.0, 0.0))));
        }
        assert!(frames.iter().any(|f| f.moved));
        assert!(camera.yaw > before.yaw + 0.2, "the model did not turn: {} -> {}", before.yaw, camera.yaw);
        assert_eq!(frames.iter().filter(|f| f.ended).count(), 1, "one gesture, one end");
        assert!(!frames.last().unwrap().moved, "still moving after the cap was let go");

        mouse.inject(&[vec![3, 0b10]], 0.0);
        let f = mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || None);
        assert_eq!(f.action, Some(ButtonAction::FitView), "the right button fits the view");
        let f = mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || None);
        assert_eq!(f.action, None, "said once");
        mouse.inject(&[vec![3, 0b01]], 0.0);
        assert_eq!(mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || None).action, Some(ButtonAction::NextStudio));

        // Off, nothing moves.
        mouse.settings.enabled = false;
        let still = camera;
        mouse.inject(&[report::report(1, &[300, 0, 0, 0, 0, 0])], 0.2);
        for _ in 0..10 {
            mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || None);
        }
        assert_eq!(camera.camera.camera_to_world, still.camera.camera_to_world);
    }

    /// On a Mac, 3Dconnexion's driver holding the device is said plainly,
    /// hidapi's own words kept as details, however hidapi puts it: the
    /// IOKit code as a number, or in words. Any other failure is said as
    /// it was, and nothing is kept aside.
    #[test]
    fn the_driver_holding_the_device_is_said_plainly() {
        for error in [
            "hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002C5) (iokit/common) exclusive access and device already open",
            "IOHIDDeviceOpen failed: (0xe00002c5)",
            "Failed to open device: kIOReturnExclusiveAccess",
            "Exclusive access and device already open",
        ] {
            let problem = OpenProblem::of("SpaceMouse Compact", error);
            assert_eq!(problem.message, HELD_BY_DRIVER, "{error}");
            assert_eq!(problem.details.as_deref(), Some(error));
        }
        for error in ["hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002E2) (iokit/common) not permitted", "Access is denied. (0x80070005)", ""] {
            assert!(!held_by_driver(error), "{error}");
            let problem = OpenProblem::of("SpaceMouse Compact", error);
            assert_eq!(problem.message, format!("SpaceMouse Compact could not be opened: {error}"));
            assert_eq!(problem.details, None);
        }
        assert!(HELD_BY_DRIVER.starts_with("3Dconnexion's driver is using the 3D mouse"));
    }

    /// A motion the device never puts back at rest - a lost report - stops
    /// on its own.
    #[test]
    fn a_motion_left_hanging_stops() {
        let ctx = egui::Context::default();
        let mouse = SpaceMouse::default();
        {
            let mut shared = mouse.shared.lock().unwrap();
            shared.motion = report::to_view([350, 0, 0, 0, 0, 0]);
            shared.at = Some(std::time::Instant::now() - STALE - std::time::Duration::from_millis(10));
        }
        let mut mouse = mouse;
        let mut camera = crate::orbit::OrbitCamera::framing(vec3(0.0, 0.0, 0.0), 1.0, 1.5);
        assert!(!mouse.frame(&ctx, &mut camera, 1.0 / 60.0, 1.5, || None).moved);
    }
}

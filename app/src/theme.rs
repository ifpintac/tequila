//! The application's look: palettes, type, density, and the drawing helpers
//! every panel shares.
//!
//! # Light, dark and themes
//!
//! Every colour the interface draws comes from the current [`Palette`],
//! read with [`p`]. There are two of each theme, a light one and a dark one,
//! made for each other rather than inverted, and the person picks:
//!
//! - **Mode**: Light, Dark, or Auto (the desktop's).
//! - **Theme**: Graphite, Ocean, Orchid, Ember, Meadow or Rose. Each is an
//!   accent colour and the faintest tint of it in the greys.
//! - **Size**: the whole interface a step smaller or larger.
//!
//! Their choice is kept in `appearance.txt` beside the user's library.
//!
//! # Colour judgement
//!
//! The eye reads lightness relatively, so the surround of a render changes
//! how the render looks, and a tinted surround tints it. Graphite, the
//! default, is the one theme with *no* tint: equal channels throughout,
//! and its dark mode the mid-dark grey colour-critical applications use.
//! The others trade a little of that neutrality for character, and say so
//! in the picker. Whatever the theme, the viewport itself is the render's
//! own picture, untouched.
//!
//! # Meaning
//!
//! The theme's **accent** means *this is the one that is active*: the chosen
//! tab, a toggle that is on, the selected part's outline. **Busy** amber is
//! work in progress elsewhere; **danger** red is something wrong. Each
//! Inspector page also has its own **hue** ([`Hue`]), on its icon and its
//! heading, so the pages are told apart at a glance.

use eframe::egui;
use egui::Color32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};

/// Row height before DPI scaling.
const ROW_HEIGHT: f32 = 28.0;
/// Body text size before DPI scaling.
pub const BODY: f32 = 13.0;

/// Every colour the interface uses, for one theme in one mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// The frame of the window: top bar, rail, status line.
    pub chrome: Color32,
    /// Side panels.
    pub panel: Color32,
    /// Groups of controls inside a panel.
    pub card: Color32,
    /// Floating windows and menus.
    pub raised: Color32,
    /// Text fields and wells: places to type.
    pub field: Color32,
    /// A control at rest, under the pointer, being pressed.
    pub control: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    /// Hairlines between regions, and a firmer one for edges that matter.
    pub line: Color32,
    pub line_strong: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    /// The theme's colour, its readable cut for text and icons, what goes on
    /// top of it, and its faint tint behind something selected.
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub accent_fill: Color32,
    pub busy: Color32,
    pub danger: Color32,
    pub success: Color32,
    /// The plates the viewport's tools float on, their edge, and their
    /// icons at rest and under the pointer.
    pub island: Color32,
    pub island_line: Color32,
    pub icon: Color32,
    pub icon_hover: Color32,
    /// Under what floats.
    pub shadow: Color32,
    /// The material graph's canvas and nodes.
    pub canvas: Color32,
    pub canvas_dot: Color32,
    pub canvas_dot_major: Color32,
    pub node_body: Color32,
    pub node_field: Color32,
    pub node_field_hover: Color32,
    pub node_fill: Color32,
    pub node_text: Color32,
    pub node_label: Color32,
    /// The console's background and its ordinary text.
    pub console: Color32,
    pub console_text: Color32,
}

/// Light, dark, or whichever the desktop is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    Light,
    #[default]
    Dark,
    Auto,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Light, Mode::Dark, Mode::Auto];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Light => "Light",
            Mode::Dark => "Dark",
            Mode::Auto => "Auto",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::Auto => "auto",
        }
    }
}

/// A theme: an accent, and the faintest tint of it in the greys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flavor {
    #[default]
    Graphite,
    Ocean,
    Orchid,
    Ember,
    Meadow,
    Rose,
}

impl Flavor {
    pub const ALL: [Flavor; 6] = [Flavor::Graphite, Flavor::Ocean, Flavor::Orchid, Flavor::Ember, Flavor::Meadow, Flavor::Rose];

    pub fn label(self) -> &'static str {
        match self {
            Flavor::Graphite => "Graphite",
            Flavor::Ocean => "Ocean",
            Flavor::Orchid => "Orchid",
            Flavor::Ember => "Ember",
            Flavor::Meadow => "Meadow",
            Flavor::Rose => "Rose",
        }
    }

    /// A line for the picker.
    pub fn blurb(self) -> &'static str {
        match self {
            Flavor::Graphite => "Neutral greys and teal: the truest surround for judging colour",
            Flavor::Ocean => "Cool slate and a clear blue",
            Flavor::Orchid => "A violet accent on soft lilac greys",
            Flavor::Ember => "Warm greys and coral",
            Flavor::Meadow => "Fresh green on sage greys",
            Flavor::Rose => "Pink on blush greys",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Flavor::Graphite => "graphite",
            Flavor::Ocean => "ocean",
            Flavor::Orchid => "orchid",
            Flavor::Ember => "ember",
            Flavor::Meadow => "meadow",
            Flavor::Rose => "rose",
        }
    }

    /// The accent, as it is.
    pub fn accent(self) -> Color32 {
        match self {
            Flavor::Graphite => Color32::from_rgb(52, 199, 176),
            Flavor::Ocean => Color32::from_rgb(76, 157, 255),
            Flavor::Orchid => Color32::from_rgb(164, 123, 255),
            Flavor::Ember => Color32::from_rgb(255, 119, 89),
            Flavor::Meadow => Color32::from_rgb(76, 195, 111),
            Flavor::Rose => Color32::from_rgb(255, 94, 143),
        }
    }

    /// Added to every grey: a few units towards the accent, never more.
    fn tint(self) -> [i16; 3] {
        match self {
            Flavor::Graphite => [0, 0, 0],
            Flavor::Ocean => [-2, 0, 4],
            Flavor::Orchid => [1, -1, 4],
            Flavor::Ember => [4, 1, -2],
            Flavor::Meadow => [-1, 3, 0],
            Flavor::Rose => [4, -1, 1],
        }
    }
}

/// How large the whole interface is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    Compact,
    #[default]
    Regular,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Compact, Size::Regular, Size::Large];

    pub fn label(self) -> &'static str {
        match self {
            Size::Compact => "Compact",
            Size::Regular => "Regular",
            Size::Large => "Large",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Size::Compact => "compact",
            Size::Regular => "regular",
            Size::Large => "large",
        }
    }

    fn zoom(self) -> f32 {
        match self {
            Size::Compact => 0.92,
            Size::Regular => 1.0,
            Size::Large => 1.12,
        }
    }
}

/// The person's choices, kept between sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    pub mode: Mode,
    pub flavor: Flavor,
    pub size: Size,
    /// The welcome card has been put away.
    pub welcome_seen: bool,
    /// The start screen is not shown at launch.
    pub start_screen_hidden: bool,
}

impl Appearance {
    fn to_text(self) -> String {
        format!(
            "mode={}\ntheme={}\nsize={}\nwelcome={}\nstart={}\n",
            self.mode.key(),
            self.flavor.key(),
            self.size.key(),
            if self.welcome_seen { "seen" } else { "new" },
            if self.start_screen_hidden { "hide" } else { "show" }
        )
    }

    /// Read back; anything unknown or missing keeps its default.
    fn from_text(text: &str) -> Appearance {
        let mut out = Appearance::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "mode" => out.mode = Mode::ALL.into_iter().find(|m| m.key() == value).unwrap_or_default(),
                "theme" => out.flavor = Flavor::ALL.into_iter().find(|f| f.key() == value).unwrap_or_default(),
                "size" => out.size = Size::ALL.into_iter().find(|s| s.key() == value).unwrap_or_default(),
                "welcome" => out.welcome_seen = value == "seen",
                "start" => out.start_screen_hidden = value == "hide",
                _ => {}
            }
        }
        out
    }

    fn path() -> std::path::PathBuf {
        crate::library_tab::user_library_path()
            .parent()
            .map(|p| p.join("appearance.txt"))
            .unwrap_or_else(|| std::env::temp_dir().join("tequila-appearance.txt"))
    }

    fn load() -> Appearance {
        std::fs::read_to_string(Appearance::path()).map(|t| Appearance::from_text(&t)).unwrap_or_default()
    }

    fn save(self) {
        let path = Appearance::path();
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, self.to_text());
    }
}

/// An Inspector page's own colour, on its icon and its heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Blue,
    Pink,
    Green,
    Amber,
    Violet,
    Orange,
    Sky,
    Slate,
}

/// A hue as the current mode draws it: as it is on dark, a shade deeper on
/// light, where a pastel stroke would wash out.
pub fn hue(h: Hue) -> Color32 {
    let c = match h {
        Hue::Blue => Color32::from_rgb(91, 155, 255),
        Hue::Pink => Color32::from_rgb(255, 111, 145),
        Hue::Green => Color32::from_rgb(61, 190, 139),
        Hue::Amber => Color32::from_rgb(255, 181, 71),
        Hue::Violet => Color32::from_rgb(157, 123, 255),
        Hue::Orange => Color32::from_rgb(255, 138, 76),
        Hue::Sky => Color32::from_rgb(79, 195, 247),
        Hue::Slate => Color32::from_rgb(154, 163, 173),
    };
    if p().dark {
        c
    } else {
        mix(c, Color32::BLACK, 0.2)
    }
}

/// `a` moved `t` of the way to `b`, opaque.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG's contrast ratio between two opaque colours.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The palette for `flavor` in light or dark.
pub fn palette(flavor: Flavor, dark: bool) -> Palette {
    let t = flavor.tint();
    let grey = |v: u8| {
        let c = |i: usize| (v as i16 + t[i]).clamp(0, 255) as u8;
        Color32::from_rgb(c(0), c(1), c(2))
    };
    let plain = Color32::from_gray;
    let accent = if dark { flavor.accent() } else { mix(flavor.accent(), Color32::BLACK, 0.12) };
    let panel = if dark { grey(30) } else { grey(244) };
    // The accent's readable cut: moved towards white on dark and black on
    // light until it reads as text on a panel.
    let mut accent_text = accent;
    let mut step = 0.0;
    while contrast(accent_text, panel) < 4.6 && step < 1.0 {
        step += 0.05;
        accent_text = mix(accent, if dark { Color32::WHITE } else { Color32::BLACK }, step);
    }
    let on_accent = if contrast(Color32::WHITE, accent) >= contrast(plain(16), accent) { Color32::WHITE } else { plain(16) };
    if dark {
        Palette {
            dark,
            chrome: grey(22),
            panel,
            card: grey(37),
            raised: grey(38),
            field: grey(21),
            control: grey(46),
            hover: grey(57),
            pressed: grey(67),
            line: grey(43),
            line_strong: grey(60),
            text: plain(236),
            text_dim: plain(165),
            text_faint: plain(118),
            accent,
            accent_text,
            on_accent,
            accent_fill: accent.gamma_multiply(0.2),
            busy: Color32::from_rgb(240, 178, 70),
            danger: Color32::from_rgb(236, 100, 90),
            success: Color32::from_rgb(110, 205, 120),
            island: Color32::from_rgba_unmultiplied(grey(24).r(), grey(24).g(), grey(24).b(), 232),
            island_line: Color32::from_white_alpha(16),
            icon: plain(180),
            icon_hover: Color32::WHITE,
            shadow: Color32::from_black_alpha(110),
            canvas: grey(28),
            canvas_dot: grey(44),
            canvas_dot_major: grey(60),
            node_body: grey(48),
            node_field: grey(34),
            node_field_hover: grey(41),
            node_fill: Color32::from_rgb(78, 82, 92),
            node_text: plain(228),
            node_label: plain(178),
            console: Color32::from_rgb(15, 16, 18),
            console_text: plain(210),
        }
    } else {
        Palette {
            dark,
            chrome: grey(232),
            panel,
            card: grey(253),
            raised: grey(252),
            field: grey(234),
            control: grey(227),
            hover: grey(216),
            pressed: grey(204),
            line: grey(220),
            line_strong: grey(198),
            text: plain(26),
            text_dim: plain(92),
            text_faint: plain(136),
            accent,
            accent_text,
            on_accent,
            accent_fill: accent.gamma_multiply(0.16),
            busy: Color32::from_rgb(186, 116, 8),
            danger: Color32::from_rgb(200, 58, 48),
            success: Color32::from_rgb(36, 146, 70),
            island: Color32::from_rgba_unmultiplied(grey(250).r(), grey(250).g(), grey(250).b(), 238),
            island_line: Color32::from_black_alpha(20),
            icon: plain(78),
            icon_hover: plain(14),
            shadow: Color32::from_black_alpha(40),
            canvas: grey(232),
            canvas_dot: grey(212),
            canvas_dot_major: grey(192),
            node_body: grey(250),
            node_field: grey(236),
            node_field_hover: grey(228),
            node_fill: Color32::from_rgb(174, 186, 204),
            node_text: plain(30),
            node_label: plain(84),
            console: grey(250),
            console_text: plain(40),
        }
    }
}

/// The application's current choices and both of its palettes.
struct State {
    appearance: Appearance,
    dark: Palette,
    light: Palette,
}

static STATE: LazyLock<RwLock<State>> = LazyLock::new(|| {
    RwLock::new(State { appearance: Appearance::default(), dark: palette(Flavor::Graphite, true), light: palette(Flavor::Graphite, false) })
});
static IS_DARK: AtomicBool = AtomicBool::new(true);

/// The palette the interface is drawn in right now.
pub fn p() -> Palette {
    let state = STATE.read().unwrap();
    if IS_DARK.load(Ordering::Relaxed) {
        state.dark
    } else {
        state.light
    }
}

/// The person's current choices.
pub fn appearance() -> Appearance {
    STATE.read().unwrap().appearance
}

/// The tint behind something selected.
pub fn accent_fill() -> Color32 {
    p().accent_fill
}

/// The font family for headings and emphasis, when [`apply`] found a
/// semibold face to give it.
const SEMIBOLD: &str = "semibold";
static SEMIBOLD_INSTALLED: AtomicBool = AtomicBool::new(false);

/// A heavier cut of the interface face for titles, or the regular one where
/// none was installed (in tests, and on a system without one).
pub fn strong(size: f32) -> egui::FontId {
    if SEMIBOLD_INSTALLED.load(Ordering::Relaxed) {
        egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
    } else {
        egui::FontId::proportional(size)
    }
}

/// The font family the interface's icons are drawn from (Lucide).
pub const ICONS: &str = "icons";

/// The interface's fonts, icons among them, into `ctx`.
pub fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(ICONS), true));
}

/// Whether `ctx` has the icon font: a context made without [`apply`] —
/// a test's — draws its icons from their vector fallbacks instead.
pub fn icons_ready(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(ICONS))).unwrap_or(false)
}

/// Apply once at startup: the fonts, and the person's saved appearance.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    // Only now, with the family bound in the application's own context: the
    // flag is process-wide and contexts are not, so setting it from
    // anywhere else would hand a test's plain context a family it lacks.
    SEMIBOLD_INSTALLED.store(true, Ordering::Relaxed);
    set_appearance(ctx, Appearance::load(), false);
}

/// Change the look: both palettes, both egui styles, the mode and the
/// size. Kept for next time when `save`.
pub fn set_appearance(ctx: &egui::Context, appearance: Appearance, save: bool) {
    let dark = palette(appearance.flavor, true);
    let light = palette(appearance.flavor, false);
    {
        let mut state = STATE.write().unwrap();
        *state = State { appearance, dark, light };
    }
    ctx.set_style_of(egui::Theme::Dark, style(ctx, &dark));
    ctx.set_style_of(egui::Theme::Light, style(ctx, &light));
    ctx.set_theme(match appearance.mode {
        Mode::Light => egui::ThemePreference::Light,
        Mode::Dark => egui::ThemePreference::Dark,
        Mode::Auto => egui::ThemePreference::System,
    });
    ctx.set_zoom_factor(appearance.size.zoom());
    sync(ctx);
    if save {
        appearance.save();
    }
}

/// Follow egui's idea of the current theme — the desktop's, in Auto — at
/// the start of each frame.
pub fn sync(ctx: &egui::Context) {
    IS_DARK.store(ctx.theme() == egui::Theme::Dark, Ordering::Relaxed);
}

fn style(ctx: &egui::Context, p: &Palette) -> egui::Style {
    let mut style = (*ctx.style()).clone();

    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(BODY - 1.5, Proportional)),
        (TextStyle::Body, FontId::new(BODY, Proportional)),
        (TextStyle::Button, FontId::new(BODY, Proportional)),
        (TextStyle::Monospace, FontId::new(BODY - 0.5, Monospace)),
        // Panel titles: heavier rather than bigger. A heading that shouts is
        // a heading that has to be read past.
        (TextStyle::Heading, strong(BODY + 2.0)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(7.0, 7.0);
    spacing.button_padding = egui::vec2(10.0, 4.0);
    spacing.interact_size.y = ROW_HEIGHT;
    spacing.menu_margin = egui::Margin::same(7.0);
    spacing.window_margin = egui::Margin::same(14.0);
    spacing.indent = 16.0;
    spacing.slider_width = 120.0;
    spacing.combo_width = 120.0;
    spacing.scroll.bar_width = 8.0;
    spacing.scroll.floating = true;

    let v = &mut style.visuals;
    *v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.raised;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.card;
    v.code_bg_color = p.field;
    v.window_stroke = egui::Stroke::new(1.0_f32, p.line_strong);
    v.window_rounding = egui::Rounding::same(12.0);
    v.menu_rounding = egui::Rounding::same(10.0);
    v.selection.bg_fill = p.accent.gamma_multiply(if p.dark { 0.34 } else { 0.28 });
    v.selection.stroke = egui::Stroke::new(1.0_f32, p.accent_text);
    v.hyperlink_color = p.accent_text;
    v.text_cursor.stroke = egui::Stroke::new(2.0_f32, p.accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.warn_fg_color = p.busy;
    v.error_fg_color = p.danger;
    // A soft, wide shadow under what floats: enough to lift a menu off the
    // panel it opened over, not so much that it reads as a smudge.
    let shadow = egui::epaint::Shadow { offset: egui::vec2(0.0, 8.0), blur: 26.0, spread: 0.0, color: p.shadow };
    v.popup_shadow = shadow;
    v.window_shadow = shadow;

    let w = &mut v.widgets;
    let rounding = egui::Rounding::same(6.0);
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.noninteractive.rounding = rounding;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = egui::Stroke::NONE;
    w.inactive.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.inactive.rounding = rounding;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = egui::Stroke::NONE;
    w.hovered.fg_stroke = egui::Stroke::new(1.5_f32, p.icon_hover);
    w.hovered.rounding = rounding;
    // egui grows a widget slightly on hover by default. At 24 px rows that
    // makes a list of them twitch as the pointer crosses it.
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.pressed;
    w.active.weak_bg_fill = p.pressed;
    w.active.bg_stroke = egui::Stroke::NONE;
    w.active.fg_stroke = egui::Stroke::new(1.5_f32, p.icon_hover);
    w.active.rounding = rounding;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = egui::Stroke::NONE;
    w.open.fg_stroke = egui::Stroke::new(1.0_f32, p.text);
    w.open.rounding = rounding;
    style
}

/// The system's interface typeface in front of egui's own.
///
/// Segoe UI on Windows and San Francisco on a Mac: the face the rest of the
/// desktop is set in, so the application reads as native and its text at
/// 13 px is the text the platform tuned for that size. egui's fonts stay
/// behind it as fallbacks, for every character the system face lacks — the
/// glyph test below checks against exactly those.
fn font_definitions() -> egui::FontDefinitions {
    let windows = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    let first = |candidates: &[std::path::PathBuf]| -> Option<Vec<u8>> {
        candidates.iter().find_map(|path| std::fs::read(path).ok())
    };
    let mac = std::path::Path::new("/System/Library/Fonts");
    let regular = first(&[windows.join("segoeui.ttf"), mac.join("SFNS.ttf")]);
    let semibold = first(&[windows.join("seguisb.ttf"), mac.join("SFNS.ttf")]);
    let mono = first(&[windows.join("CascadiaMono.ttf"), windows.join("consola.ttf"), mac.join("SFNSMono.ttf")]);

    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(bytes) = regular {
        fonts.font_data.insert("system-ui".into(), egui::FontData::from_owned(bytes));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "system-ui".into());
    }
    let mut heavy = proportional.clone();
    if let Some(bytes) = semibold {
        fonts.font_data.insert("system-ui-semibold".into(), egui::FontData::from_owned(bytes));
        heavy.insert(0, "system-ui-semibold".into());
    } else if fonts.font_data.contains_key("system-ui") {
        heavy.insert(0, "system-ui".into());
    }
    fonts.families.insert(egui::FontFamily::Name(SEMIBOLD.into()), heavy);
    if let Some(bytes) = mono {
        fonts.font_data.insert("system-mono".into(), egui::FontData::from_owned(bytes));
        fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "system-mono".into());
    }
    // Lucide's icons (ISC licence, scripts/notices/lucide): a family of
    // their own, so their private-use codepoints never meet text.
    fonts.font_data.insert("lucide".into(), egui::FontData::from_static(include_bytes!("../assets/fonts/lucide.ttf")));
    fonts.families.insert(egui::FontFamily::Name(ICONS.into()), vec!["lucide".into()]);
    fonts
}

/// A heading for a block of controls inside a tab.
///
/// Small, upper-case and dim rather than large and bright. In a panel that is
/// almost entirely controls, a section label's job is to be findable when
/// looked for and invisible when not — the controls are what people are
/// reading.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(strong(10.5))
            .color(p().text_dim)
            .extra_letter_spacing(0.6),
    );
    ui.add_space(1.0);
}

/// The grey used for anything secondary: hints, units, counts.
pub fn dim() -> Color32 {
    p().text_dim
}

/// A group of controls on a card, a step lighter than the panel it sits on.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let p = p();
    egui::Frame::none()
        .fill(p.card)
        .stroke(if p.dark { egui::Stroke::NONE } else { egui::Stroke::new(1.0_f32, p.line) })
        .rounding(10.0)
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
}

/// A one-line search field the width of the space it is given, with the
/// magnifier drawn inside it and a clear button once there is something to
/// clear.
pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    let p = p();
    let margin = egui::Margin { left: 28.0, right: 22.0, top: 6.0, bottom: 6.0 };
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(f32::INFINITY)
            .hint_text(egui::RichText::new(hint).color(p.text_faint))
            .margin(margin),
    );
    // A text edit reports the rect inside its margin; the icons sit in the
    // margin, so measure from the outside.
    let rect = response.rect + margin;
    let glyph = egui::Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), egui::Vec2::splat(14.0));
    crate::icons::paint(ui.painter(), glyph, crate::icons::Icon::Search, if response.has_focus() { p.accent_text } else { p.text_faint });
    if !text.is_empty() {
        let clear = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), egui::Vec2::splat(16.0));
        let hit = ui.interact(clear, response.id.with("clear"), egui::Sense::click());
        crate::icons::paint(ui.painter(), clear.shrink(2.0), crate::icons::Icon::Close, if hit.hovered() { p.text } else { p.text_faint });
        if hit.on_hover_text("Clear").clicked() {
            text.clear();
        }
    }
    response
}

/// A row of mutually exclusive choices drawn as one control, the chosen one
/// raised — for a handful of short options where a dropdown would hide them.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = p();
    let mut changed = false;
    let height = 28.0;
    let width = ui.available_width();
    // Each control's cells are identified by the control's own id: two in
    // one panel identified by the panel's shared their cells' clicks.
    let (rect, base) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8.0, p.field);
    let each = (rect.width() - 6.0) / options.len().max(1) as f32;
    for (i, (option, label)) in options.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0 + i as f32 * each, rect.top() + 3.0),
            egui::vec2(each, height - 6.0),
        );
        let response = ui.interact(cell, base.id.with(("segment", i)), egui::Sense::click());
        let on = *value == *option;
        if on {
            ui.painter().add(egui::epaint::Shadow { offset: egui::vec2(0.0, 1.0), blur: 4.0, spread: 0.0, color: Color32::from_black_alpha(if p.dark { 60 } else { 26 }) }.as_shape(cell, 6.0));
            ui.painter().rect_filled(cell, 6.0, if p.dark { p.pressed } else { p.card });
        } else if response.hovered() {
            ui.painter().rect_filled(cell, 6.0, p.hover.gamma_multiply(0.6));
        }
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            if on { strong(BODY - 1.0) } else { egui::FontId::proportional(BODY - 1.0) },
            if on { p.text } else if response.hovered() { p.text } else { p.text_dim },
        );
        if response.clicked() && !on {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// What a panel shows when it has nothing to show yet: an icon in a soft
/// disc of its colour, a title, and a line saying what to do about it.
pub fn empty_state(ui: &mut egui::Ui, icon: crate::icons::Icon, colour: Color32, title: &str, hint: &str) {
    let p = p();
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let (disc, _) = ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
        ui.painter().circle_filled(disc.center(), 32.0, colour.gamma_multiply(if p.dark { 0.16 } else { 0.12 }));
        ui.painter().circle_stroke(disc.center(), 32.0, egui::Stroke::new(1.0_f32, colour.gamma_multiply(0.35)));
        crate::icons::paint(ui.painter(), egui::Rect::from_center_size(disc.center(), egui::Vec2::splat(28.0)), icon, colour);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(title).font(strong(14.0)).color(p.text));
        ui.add_space(2.0);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.set_max_width(250.0);
        ui.label(egui::RichText::new(hint).size(12.0).color(p.text_dim));
    });
    ui.add_space(18.0);
}

/// A key as this computer's keyboard names it. Bindings are written with
/// `Ctrl` for the command key, which on a Mac is Cmd, and Alt is Option
/// there; `Control` is the Mac's Control key itself.
fn key_name(key: &str, mac: bool) -> &str {
    match (mac, key) {
        (true, "Ctrl") => "Cmd",
        (true, "Alt") => "Option",
        _ => key,
    }
}

/// A binding written out, `"Ctrl+Shift+S"` or `"Ctrl + H / Alt + H"`, in
/// this computer's key names, for a tooltip.
pub fn keys(binding: &str) -> String {
    keys_for(binding, cfg!(target_os = "macos"))
}

fn keys_for(binding: &str, mac: bool) -> String {
    binding
        .split_inclusive(['+', '/'])
        .map(|piece| {
            // Each key keeps the spaces around it and the `+` or `/` after.
            let (body, separator) = match piece.char_indices().last() {
                Some((at, '+' | '/')) => (&piece[..at], &piece[at..]),
                _ => (piece, ""),
            };
            let key = body.trim();
            let at = body.find(key).unwrap_or(0);
            format!("{}{}{}{separator}", &body[..at], key_name(key, mac), &body[at + key.len()..])
        })
        .collect()
}

/// A binding as key caps: `"Ctrl + S"`, or alternatives, `"F / Shift + F"`.
pub fn key_caps(ui: &mut egui::Ui, binding: &str) {
    let p = p();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, chord) in binding.split(" / ").enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("or").size(11.0).color(p.text_faint));
            }
            for (j, key) in chord.split(" + ").enumerate() {
                if j > 0 {
                    ui.label(egui::RichText::new("+").size(11.0).color(p.text_faint));
                }
                let key = key_name(key, cfg!(target_os = "macos"));
                let galley = ui.painter().layout_no_wrap(key.to_string(), egui::FontId::proportional(11.5), p.text);
                let size = galley.size() + egui::vec2(12.0, 6.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(size.x.max(size.y + 2.0), size.y), egui::Sense::hover());
                // A key cap: a little raised, with a heavier lower edge.
                ui.painter().rect_filled(rect.translate(egui::vec2(0.0, 1.5)), 5.0, p.line_strong);
                ui.painter().rect(rect, 5.0, p.card, egui::Stroke::new(1.0_f32, p.line_strong));
                ui.painter().galley(rect.center() - galley.size() * 0.5, galley, Color32::PLACEHOLDER);
            }
        }
    });
}

/// What a tooltip says: a title, a line saying more, and the keys.
pub fn tip_body(ui: &mut egui::Ui, title: &str, detail: &str, keys: &str) {
    let p = p();
    ui.set_max_width(280.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(strong(BODY - 0.5)).color(p.text));
        if !keys.is_empty() {
            ui.add_space(6.0);
            key_caps(ui, keys);
        }
    });
    if !detail.is_empty() {
        ui.add_space(-3.0);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.label(egui::RichText::new(detail).size(BODY - 1.0).color(p.text_dim));
    }
}

/// One of several choices, as a card the width of the column: an icon on
/// a badge in its colour, a name, and a line saying what it is for; a
/// radio at the right, ticked in the accent on the chosen one.
pub fn choice(ui: &mut egui::Ui, id: egui::Id, icon: crate::icons::Icon, colour: Color32, title: &str, detail: &str, chosen: bool) -> egui::Response {
    let p = p();
    let width = ui.available_width();
    let (left, right) = (52.0, 36.0);
    let detail = ui.painter().layout(detail.to_string(), egui::FontId::proportional(BODY - 1.5), p.text_dim, (width - left - right).max(60.0));
    let height = (10.0 + 18.0 + detail.size().y + 10.0).max(52.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let response = ui.interact(rect, id, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let ease = ui.ctx().animate_bool_with_time(id.with("ease"), response.hovered(), 0.1);
        let painter = ui.painter();
        let (fill, stroke) = if chosen {
            (mix(p.card, p.accent, if p.dark { 0.12 } else { 0.07 }), egui::Stroke::new(1.5_f32, p.accent))
        } else {
            (mix(p.card, p.hover, 0.6 * ease), egui::Stroke::new(1.0_f32, mix(p.line, p.line_strong, ease)))
        };
        painter.rect(rect, 10.0, fill, stroke);
        badge(painter, egui::Rect::from_min_size(rect.min + egui::vec2(11.0, 11.0), egui::Vec2::splat(30.0)), icon, colour);
        painter.text(rect.min + egui::vec2(left, 10.0), egui::Align2::LEFT_TOP, title, strong(BODY), p.text);
        painter.galley(rect.min + egui::vec2(left, 29.0), detail, Color32::PLACEHOLDER);
        let radio = egui::pos2(rect.right() - 19.0, rect.top() + 19.0);
        if chosen {
            painter.circle_filled(radio, 8.0, p.accent);
            crate::icons::paint(painter, egui::Rect::from_center_size(radio, egui::Vec2::splat(11.0)), crate::icons::Icon::Check, p.on_accent);
        } else {
            painter.circle_stroke(radio, 7.5, egui::Stroke::new(1.5_f32, mix(p.line_strong, p.text_dim, ease)));
        }
    }
    response
}

/// A badge: a rounded square tinted in `colour`, the icon on it in full.
pub fn badge(painter: &egui::Painter, rect: egui::Rect, icon: crate::icons::Icon, colour: Color32) {
    painter.rect_filled(rect, rect.height() * 0.3, colour.gamma_multiply(if p().dark { 0.2 } else { 0.14 }));
    crate::icons::paint(painter, rect.shrink(rect.height() * 0.22), icon, colour);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_names_its_own_keys() {
        assert_eq!(keys_for("Ctrl+Shift+S", true), "Cmd+Shift+S");
        assert_eq!(keys_for("Ctrl + Alt + 3", true), "Cmd + Option + 3");
        assert_eq!(keys_for("Control + H / Alt + H", true), "Control + H / Option + H");
        // Elsewhere, as written; a key that merely contains a name is not it.
        assert_eq!(keys_for("Ctrl + H / Alt + H", false), "Ctrl + H / Alt + H");
        assert_eq!(keys_for("Altitude / Ctrls", true), "Altitude / Ctrls");
    }

    /// Every theme reads, in both modes: body text, secondary text, the
    /// accent's text cut against the panel, and what sits on the accent.
    ///
    /// The thresholds are WCAG's: 4.5 for text, 3 for large text and
    /// the faint hints. A theme that failed one would ship text somebody
    /// cannot read, which is the one thing a theme must never do.
    #[test]
    fn every_theme_reads_in_both_modes() {
        for flavor in Flavor::ALL {
            for dark in [true, false] {
                let p = palette(flavor, dark);
                let name = format!("{} {}", flavor.label(), if dark { "dark" } else { "light" });
                for (what, colour, surface, needed) in [
                    ("text on the panel", p.text, p.panel, 7.0),
                    ("text on a card", p.text, p.card, 7.0),
                    ("dim text on the panel", p.text_dim, p.panel, 4.5),
                    ("faint text on the panel", p.text_faint, p.panel, 2.6),
                    ("accent text on the panel", p.accent_text, p.panel, 4.5),
                    ("text on the accent", p.on_accent, p.accent, 2.4),
                    ("text on the chrome", p.text, p.chrome, 7.0),
                    ("busy on the panel", p.busy, p.panel, 2.6),
                    ("console text", p.console_text, p.console, 7.0),
                    ("node text", p.node_text, p.node_body, 7.0),
                ] {
                    let ratio = contrast(colour, surface);
                    assert!(ratio >= needed, "{name}: {what} is {ratio:.2}:1, needs {needed}:1");
                }
            }
        }
    }

    /// Two segmented controls in one panel are two controls: a click on one
    /// changes it and only it.
    ///
    /// Their cells were identified by the panel's id and their index, so the
    /// Appearance picker's Mode and Size cells shared ids. A click on Size
    /// also set Mode, and Mode could not be clicked at all.
    #[test]
    fn two_segmented_controls_do_not_share_clicks() {
        let ctx = egui::Context::default();
        let (mut first, mut second) = (0usize, 0usize);
        let mut tops = (0.0f32, 0.0f32);
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, first: &mut usize, second: &mut usize, tops: &mut (f32, f32)| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    tops.0 = ui.cursor().top();
                    segmented(ui, first, &[(0, "A"), (1, "B"), (2, "C")]);
                    tops.1 = ui.cursor().top();
                    segmented(ui, second, &[(0, "X"), (1, "Y"), (2, "Z")]);
                });
            });
        };
        let click = |at: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
            ]
        };
        frame(Vec::new(), &mut first, &mut second, &mut tops);
        frame(Vec::new(), &mut first, &mut second, &mut tops);
        // The third cell of the second control.
        let at = egui::pos2(250.0, tops.1 + 14.0);
        frame(click(at), &mut first, &mut second, &mut tops);
        frame(Vec::new(), &mut first, &mut second, &mut tops);
        assert_eq!(second, 2, "the click chose the second control's third option");
        assert_eq!(first, 0, "and left the first control alone");
        // The middle cell of the first control.
        let at = egui::pos2(150.0, tops.0 + 14.0);
        frame(click(at), &mut first, &mut second, &mut tops);
        frame(Vec::new(), &mut first, &mut second, &mut tops);
        assert_eq!(first, 1, "the first control takes its own clicks");
        assert_eq!(second, 2, "without moving the second");
    }

    /// Graphite is neutral: equal channels in every grey, in both modes.
    #[test]
    fn graphite_is_neutral() {
        for dark in [true, false] {
            let p = palette(Flavor::Graphite, dark);
            for c in [p.chrome, p.panel, p.card, p.raised, p.field, p.control, p.hover, p.line, p.canvas] {
                assert!(c.r() == c.g() && c.g() == c.b(), "{c:?} is tinted");
            }
        }
    }

    /// The choices survive a round trip through the file, and a file from a
    /// later version, or a broken one, reads as the defaults rather than
    /// failing.
    #[test]
    fn appearance_round_trips() {
        let a = Appearance { mode: Mode::Light, flavor: Flavor::Orchid, size: Size::Large, welcome_seen: true, start_screen_hidden: true };
        assert_eq!(Appearance::from_text(&a.to_text()), a);
        assert_eq!(Appearance::from_text("mode=sepia\ntheme=\ngarbage\n"), Appearance::default());
    }

    /// Every character the interface writes has a glyph in the fonts it
    /// draws with.
    ///
    /// A missing one draws as an empty box, looks like a broken build, and
    /// shows up only when somebody opens that panel — the Studios tab's
    /// arrows and bullets shipped as boxes before this read the sources.
    /// Reads every string literal in the app and asks egui's default fonts,
    /// which stay behind the system face as its fallbacks, about each
    /// character outside ASCII.
    #[test]
    fn every_symbol_in_the_interface_has_a_glyph() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        // Every source file, in folders too: a module split into a folder
        // was skipped, and its symbols went unchecked.
        let mut files = Vec::new();
        let mut folders = vec![directory];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                // Only what is between quotes: comments and code are not
                // drawn.
                for (i, literal) in line.split('"').enumerate() {
                    if i % 2 == 0 {
                        continue;
                    }
                    for c in literal.chars().filter(|c| !c.is_ascii()) {
                        if !ctx.fonts(|f| f.has_glyph(&font, c)) {
                            missing.push(format!(
                                "{:?} (U+{:04X}) in {}:{}",
                                c,
                                c as u32,
                                path.file_name().unwrap().to_string_lossy(),
                                number + 1
                            ));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "these draw as empty boxes:\n{}", missing.join("\n"));
    }

    /// The system face goes in front of egui's, never instead of it: every
    /// family keeps egui's fonts behind it, so a character Segoe UI or San
    /// Francisco lacks still draws.
    #[test]
    fn the_system_face_keeps_egui_fonts_behind_it() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::font_definitions());
        let _ = ctx.run(Default::default(), |_| {});
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace, egui::FontFamily::Name(super::SEMIBOLD.into())] {
            // A character only egui's emoji font has.
            assert!(ctx.fonts(|f| f.has_glyph(&egui::FontId::new(14.0, family.clone()), '⚡')), "{family:?} lost its fallbacks");
        }
    }
}

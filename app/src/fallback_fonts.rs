//! Fonts for the writing systems the interface face lacks: Chinese,
//! Japanese and Korean, and Devanagari (Hindi, Marathi, Nepali).
//!
//! Segoe UI and San Francisco cover Latin, Greek and Cyrillic, so Spanish,
//! French, Russian or Greek names draw in the interface face. Neither has
//! Han characters, kana, Hangul or Devanagari, and egui draws an empty box
//! for a character none of its fonts has: a folder, a file or a part named
//! in those read as a row of boxes. Every desktop already has fonts for
//! them, 5 to 20 MB each, so they are found among the system's own fonts as
//! Tequila starts ([`candidates`]) rather than shipped with it.
//!
//! They go *after* every font the interface had, so a character any of
//! those has still comes from it: Latin text, symbols and emoji look exactly
//! as they did, and only what drew as a box changes.
//!
//! # Han characters
//!
//! Chinese, Japanese and Korean share Unicode's Han characters but draw a
//! good many of them differently, and a name should look right to the
//! person who wrote it. The four fonts are ordered by the person's language
//! ([`han_order`]): kanji from the Japanese font on a Japanese desktop,
//! hanzi from the Traditional Chinese one in Taiwan or Hong Kong, and from
//! the Simplified Chinese one anywhere else, with the others behind it for
//! what it lacks (Hangul is only in the Korean font).
//!
//! # Cost
//!
//! The files are mapped rather than read: what is used of them is paged in
//! as its glyphs are drawn, the rest of a 20 MB font costs nothing, and the
//! pages are the ones the system's own text drawing already has in memory.
//! Finding and mapping them all is a few milliseconds of start-up (2 ms for
//! Windows' ten faces, 132 MB of files), so they are loaded at once rather
//! than the first time one is needed.
//!
//! # Devanagari
//!
//! egui draws one glyph per character, left to right, and does no shaping.
//! Devanagari's vowel signs above and below a letter sit on it well enough,
//! as the fonts draw them to; its conjuncts (the half letters of न्द) show
//! as the full letters with the virama between them, a spelling a Hindi
//! reader understands but would not write; and the vowel sign ि, which is
//! written before the consonant it follows, is drawn after it unless the
//! text is put in drawing order first ([`drawing_order`]), which names in
//! lists are.

use eframe::egui;
use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The systems whose font folders and file names differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    /// Linux and the other Unixes: fontconfig's folders.
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }
}

/// A writing system the interface face has no letters for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    SimplifiedChinese,
    TraditionalChinese,
    Japanese,
    Korean,
    Devanagari,
}

impl Script {
    /// The name its font goes by among egui's fonts.
    fn key(self) -> &'static str {
        match self {
            Script::SimplifiedChinese => "zh-hans",
            Script::TraditionalChinese => "zh-hant",
            Script::Japanese => "ja",
            Script::Korean => "ko",
            Script::Devanagari => "deva",
        }
    }
}

/// A font file to look for, and which face in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The file's name, looked for in the platform's font folders.
    pub file: &'static str,
    /// The face's name - its full name, or its family and style - for a
    /// collection of several (`.ttc`). The first face when none is called
    /// that.
    pub face: &'static str,
}

/// A [`Candidate`], as a literal: a `const fn` call is not promoted to a
/// static, and the lists below are borrowed for the life of the program.
macro_rules! font {
    ($file:expr, $face:expr) => {
        Candidate { file: $file, face: $face }
    };
}

/// The fonts to look for, best first: the first one found is used. `bold`
/// for headings, which take the regular face where there is no bold one.
///
/// Each platform's own interface fonts for the script come first - the
/// faces its menus and file windows show these characters in - then the
/// ones a person or a distribution commonly adds.
pub fn candidates(platform: Platform, script: Script, bold: bool) -> &'static [Candidate] {
    use Platform::*;
    use Script::*;
    match (platform, script, bold) {
        (Windows, SimplifiedChinese, false) => &[font!("msyh.ttc", "Microsoft YaHei UI")],
        (Windows, SimplifiedChinese, true) => &[font!("msyhbd.ttc", "Microsoft YaHei UI Bold")],
        (Windows, TraditionalChinese, false) => &[font!("msjh.ttc", "Microsoft JhengHei UI")],
        (Windows, TraditionalChinese, true) => &[font!("msjhbd.ttc", "Microsoft JhengHei UI Bold")],
        (Windows, Japanese, false) => &[font!("YuGothM.ttc", "Yu Gothic UI"), font!("meiryo.ttc", "Meiryo UI")],
        (Windows, Japanese, true) => &[font!("YuGothB.ttc", "Yu Gothic UI Semibold"), font!("meiryob.ttc", "Meiryo UI Bold")],
        (Windows, Korean, false) => &[font!("malgun.ttf", "Malgun Gothic")],
        (Windows, Korean, true) => &[font!("malgunbd.ttf", "Malgun Gothic Bold")],
        (Windows, Devanagari, false) => &[font!("Nirmala.ttc", "Nirmala UI")],
        (Windows, Devanagari, true) => &[font!("Nirmala.ttc", "Nirmala UI Bold")],

        // PingFang moved from the Fonts folder to the font services'
        // reserved one in macOS 12; Heiti and Arial Unicode are on every
        // version since 10.6.
        (Mac, SimplifiedChinese, false) => &[
            font!("PingFang.ttc", "PingFang SC"),
            font!("PingFangUI.ttc", "PingFang SC"),
            font!("Hiragino Sans GB.ttc", "Hiragino Sans GB W3"),
            font!("STHeiti Light.ttc", "Heiti SC Light"),
            font!("Arial Unicode.ttf", "Arial Unicode MS"),
        ],
        (Mac, SimplifiedChinese, true) => &[
            font!("PingFang.ttc", "PingFang SC Semibold"),
            font!("PingFangUI.ttc", "PingFang SC Semibold"),
            font!("Hiragino Sans GB.ttc", "Hiragino Sans GB W6"),
            font!("STHeiti Medium.ttc", "Heiti SC Medium"),
        ],
        (Mac, TraditionalChinese, false) => &[
            font!("PingFang.ttc", "PingFang TC"),
            font!("PingFangUI.ttc", "PingFang TC"),
            font!("STHeiti Light.ttc", "Heiti TC Light"),
            font!("Arial Unicode.ttf", "Arial Unicode MS"),
        ],
        (Mac, TraditionalChinese, true) => &[
            font!("PingFang.ttc", "PingFang TC Semibold"),
            font!("PingFangUI.ttc", "PingFang TC Semibold"),
            font!("STHeiti Medium.ttc", "Heiti TC Medium"),
        ],
        (Mac, Japanese, false) => &[
            font!("\u{30D2}\u{30E9}\u{30AE}\u{30CE}\u{89D2}\u{30B4}\u{30B7}\u{30C3}\u{30AF} W3.ttc", "Hiragino Sans W3"),
            font!("\u{30D2}\u{30E9}\u{30AE}\u{30CE}\u{89D2}\u{30B4} ProN W3.otf", "Hiragino Kaku Gothic ProN W3"),
            font!("Arial Unicode.ttf", "Arial Unicode MS"),
        ],
        (Mac, Japanese, true) => &[
            font!("\u{30D2}\u{30E9}\u{30AE}\u{30CE}\u{89D2}\u{30B4}\u{30B7}\u{30C3}\u{30AF} W6.ttc", "Hiragino Sans W6"),
            font!("\u{30D2}\u{30E9}\u{30AE}\u{30CE}\u{89D2}\u{30B4} ProN W6.otf", "Hiragino Kaku Gothic ProN W6"),
        ],
        (Mac, Korean, false) => &[
            font!("AppleSDGothicNeo.ttc", "Apple SD Gothic Neo"),
            font!("AppleGothic.ttf", "AppleGothic"),
            font!("Arial Unicode.ttf", "Arial Unicode MS"),
        ],
        (Mac, Korean, true) => &[font!("AppleSDGothicNeo.ttc", "Apple SD Gothic Neo Bold")],
        (Mac, Devanagari, false) => &[
            font!("Kohinoor.ttc", "Kohinoor Devanagari"),
            font!("DevanagariMT.ttc", "Devanagari MT"),
            font!("ITFDevanagari.ttc", "ITF Devanagari Book"),
            font!("Arial Unicode.ttf", "Arial Unicode MS"),
        ],
        (Mac, Devanagari, true) => &[
            font!("Kohinoor.ttc", "Kohinoor Devanagari Semibold"),
            font!("DevanagariMT.ttc", "Devanagari MT Bold"),
        ],

        // Noto Sans CJK (as one collection, or a file a language), then
        // Source Han Sans, its Adobe twin, then what older distributions
        // carried.
        (Linux, SimplifiedChinese, false) => &[
            font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK SC"),
            font!("NotoSansCJKsc-Regular.otf", "Noto Sans CJK SC"),
            font!("NotoSansSC-Regular.otf", "Noto Sans SC"),
            font!("SourceHanSans-Regular.ttc", "Source Han Sans SC"),
            font!("SourceHanSansSC-Regular.otf", "Source Han Sans SC"),
            font!("wqy-microhei.ttc", "WenQuanYi Micro Hei"),
            font!("wqy-zenhei.ttc", "WenQuanYi Zen Hei"),
            font!("DroidSansFallbackFull.ttf", "Droid Sans Fallback"),
        ],
        (Linux, SimplifiedChinese, true) => &[
            font!("NotoSansCJK-Bold.ttc", "Noto Sans CJK SC Bold"),
            font!("NotoSansCJKsc-Bold.otf", "Noto Sans CJK SC Bold"),
            font!("NotoSansSC-Bold.otf", "Noto Sans SC Bold"),
            font!("SourceHanSans-Bold.ttc", "Source Han Sans SC Bold"),
        ],
        (Linux, TraditionalChinese, false) => &[
            font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK TC"),
            font!("NotoSansCJKtc-Regular.otf", "Noto Sans CJK TC"),
            font!("NotoSansTC-Regular.otf", "Noto Sans TC"),
            font!("SourceHanSans-Regular.ttc", "Source Han Sans TC"),
            font!("SourceHanSansTC-Regular.otf", "Source Han Sans TC"),
        ],
        (Linux, TraditionalChinese, true) => &[
            font!("NotoSansCJK-Bold.ttc", "Noto Sans CJK TC Bold"),
            font!("NotoSansCJKtc-Bold.otf", "Noto Sans CJK TC Bold"),
            font!("NotoSansTC-Bold.otf", "Noto Sans TC Bold"),
        ],
        (Linux, Japanese, false) => &[
            font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK JP"),
            font!("NotoSansCJKjp-Regular.otf", "Noto Sans CJK JP"),
            font!("NotoSansJP-Regular.otf", "Noto Sans JP"),
            font!("SourceHanSans-Regular.ttc", "Source Han Sans"),
            font!("ipagp.ttf", "IPAPGothic"),
            font!("TakaoPGothic.ttf", "TakaoPGothic"),
        ],
        (Linux, Japanese, true) => &[
            font!("NotoSansCJK-Bold.ttc", "Noto Sans CJK JP Bold"),
            font!("NotoSansCJKjp-Bold.otf", "Noto Sans CJK JP Bold"),
            font!("NotoSansJP-Bold.otf", "Noto Sans JP Bold"),
        ],
        (Linux, Korean, false) => &[
            font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK KR"),
            font!("NotoSansCJKkr-Regular.otf", "Noto Sans CJK KR"),
            font!("NotoSansKR-Regular.otf", "Noto Sans KR"),
            font!("SourceHanSans-Regular.ttc", "Source Han Sans K"),
            font!("NanumGothic.ttf", "NanumGothic"),
            font!("UnDotum.ttf", "UnDotum"),
        ],
        (Linux, Korean, true) => &[
            font!("NotoSansCJK-Bold.ttc", "Noto Sans CJK KR Bold"),
            font!("NotoSansCJKkr-Bold.otf", "Noto Sans CJK KR Bold"),
            font!("NotoSansKR-Bold.otf", "Noto Sans KR Bold"),
            font!("NanumGothicBold.ttf", "NanumGothic Bold"),
        ],
        (Linux, Devanagari, false) => &[
            font!("NotoSansDevanagari-Regular.ttf", "Noto Sans Devanagari"),
            font!("NotoSansDevanagariUI-Regular.ttf", "Noto Sans Devanagari UI"),
            font!("Lohit-Devanagari.ttf", "Lohit Devanagari"),
            font!("FreeSans.ttf", "FreeSans"),
        ],
        (Linux, Devanagari, true) => &[
            font!("NotoSansDevanagari-Bold.ttf", "Noto Sans Devanagari Bold"),
            font!("NotoSansDevanagariUI-Bold.ttf", "Noto Sans Devanagari UI Bold"),
            font!("FreeSansBold.ttf", "FreeSans Bold"),
        ],
    }
}

/// The folders a platform keeps fonts in, the person's own among them.
/// `var` reads the environment.
pub fn folders(platform: Platform, var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let home = var("HOME").map(PathBuf::from);
    match platform {
        Platform::Windows => {
            let windows = var("WINDIR").or_else(|| var("SystemRoot")).map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
            let mut out = vec![windows.join("Fonts")];
            // Fonts installed "for me only" since Windows 10 1809.
            if let Some(local) = var("LOCALAPPDATA") {
                out.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
            }
            out
        }
        Platform::Mac => {
            let mut out: Vec<PathBuf> = [
                "/System/Library/Fonts",
                "/System/Library/Fonts/Supplemental",
                "/System/Library/PrivateFrameworks/FontServices.framework/Resources/Reserved",
                "/Library/Fonts",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect();
            out.extend(home.map(|h| h.join("Library").join("Fonts")));
            out
        }
        Platform::Linux => {
            // fontconfig's: the XDG data folders' `fonts`, and ~/.fonts.
            let data_home = var("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".local").join("share")));
            let data_dirs = var("XDG_DATA_DIRS")
                .map(|dirs| std::env::split_paths(&dirs).collect::<Vec<_>>())
                .filter(|dirs| !dirs.is_empty())
                .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
            let mut out: Vec<PathBuf> = data_home.into_iter().chain(data_dirs).map(|d| d.join("fonts")).collect();
            out.extend(home.map(|h| h.join(".fonts")));
            out.dedup();
            out
        }
    }
}

/// The order of the Han fonts for a person whose language is `locale`
/// (`zh-CN`, `ja_JP.UTF-8`, `zh-Hant-TW`...): the font of their own
/// language's forms first.
pub fn han_order(locale: &str) -> [Script; 4] {
    use Script::*;
    let tag = locale.split('.').next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or("");
    let traditional = parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo"));
    match language {
        "ja" => [Japanese, SimplifiedChinese, TraditionalChinese, Korean],
        "ko" => [Korean, TraditionalChinese, Japanese, SimplifiedChinese],
        "zh" if traditional => [TraditionalChinese, SimplifiedChinese, Japanese, Korean],
        _ => [SimplifiedChinese, TraditionalChinese, Japanese, Korean],
    }
}

/// The person's language, as the system says: the user locale on Windows,
/// the first preferred language on a Mac, the locale variables elsewhere.
/// Empty when it says nothing.
pub fn locale() -> String {
    let from_environment = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX");
    system_locale().filter(|l| !l.is_empty()).or(from_environment).unwrap_or_default()
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
    }
    // LOCALE_NAME_MAX_LENGTH.
    let mut buffer = [0u16; 85];
    // SAFETY: the buffer is as long as the length given.
    let written = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    (written > 1).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    use std::ffi::{c_char, c_void, CStr};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(object: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    // SAFETY: the array is released once, after the string borrowed from
    // it (a Get, not owned) is copied out.
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() {
            return None;
        }
        let mut out = None;
        if CFArrayGetCount(languages) > 0 {
            let first = CFArrayGetValueAtIndex(languages, 0);
            let mut buffer = [0 as c_char; 64];
            if !first.is_null() && CFStringGetCString(first, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) != 0 {
                out = Some(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
            }
        }
        CFRelease(languages);
        out
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_locale() -> Option<String> {
    None
}

/// A face found for a script.
#[derive(Clone)]
pub struct Face {
    pub script: Script,
    pub bold: bool,
    pub path: PathBuf,
    /// Which face of a collection.
    pub index: u32,
    bytes: &'static [u8],
}

impl Face {
    fn key(&self) -> String {
        format!("{}{}", self.script.key(), if self.bold { "-bold" } else { "" })
    }
}

/// The faces found, in the order they stand behind the interface's own
/// fonts.
#[derive(Clone, Default)]
pub struct Found {
    pub regular: Vec<Face>,
    /// For headings: each script's bold face, or its regular one where it
    /// has none.
    pub bold: Vec<Face>,
}

/// This computer's, looked for once.
pub fn found() -> &'static Found {
    static FOUND: OnceLock<Found> = OnceLock::new();
    FOUND.get_or_init(|| {
        let platform = Platform::current();
        find(platform, &folders(platform, |name| std::env::var_os(name)), &locale(), &map)
    })
}

/// Look in `folders` for `platform`'s fonts, ordered for `locale`, reading
/// each file found with `load`.
pub fn find(platform: Platform, folders: &[PathBuf], locale: &str, load: &dyn Fn(&Path) -> Option<&'static [u8]>) -> Found {
    let index = (platform == Platform::Linux).then(|| index_of(folders));
    let locate = |file: &str| -> Option<PathBuf> {
        match &index {
            Some(index) => index.iter().find(|(name, _)| name == file).map(|(_, path)| path.clone()),
            None => folders.iter().map(|folder| folder.join(file)).find(|path| path.is_file()),
        }
    };
    let mut opened: Vec<(PathBuf, &'static [u8])> = Vec::new();
    let mut face_of = |script: Script, bold: bool| -> Option<Face> {
        candidates(platform, script, bold).iter().find_map(|candidate| {
            let path = locate(candidate.file)?;
            let known = opened.iter().find(|(p, _)| *p == path).map(|(_, bytes)| *bytes);
            let bytes = match known {
                Some(bytes) => bytes,
                None => {
                    let bytes = load(&path)?;
                    opened.push((path.clone(), bytes));
                    bytes
                }
            };
            let index = face_named(bytes, candidate.face)?;
            Some(Face { script, bold, path, index, bytes })
        })
    };
    let order = han_order(locale).into_iter().chain([Script::Devanagari]);
    let mut found = Found::default();
    // One face standing in for several scripts (Arial Unicode, on an older
    // Mac) is put in once.
    let new = |faces: &[Face], face: &Face| !faces.iter().any(|f| f.path == face.path && f.index == face.index);
    for script in order {
        let Some(regular) = face_of(script, false) else { continue };
        let bold = face_of(script, true).unwrap_or_else(|| regular.clone());
        if new(&found.regular, &regular) {
            found.regular.push(regular);
        }
        if new(&found.bold, &bold) {
            found.bold.push(bold);
        }
    }
    found
}

/// Every candidate's file name under `folders` (fontconfig's are nested by
/// package), and where it is: the first of each name.
fn index_of(folders: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let wanted: std::collections::HashSet<&str> = [false, true]
        .into_iter()
        .flat_map(|bold| {
            [Script::SimplifiedChinese, Script::TraditionalChinese, Script::Japanese, Script::Korean, Script::Devanagari]
                .into_iter()
                .flat_map(move |script| candidates(Platform::Linux, script, bold).iter().map(|c| c.file))
        })
        .collect();
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = folders.iter().map(|f| (f.clone(), 0)).collect();
    // Front first, so an earlier folder (the person's own) wins.
    stack.reverse();
    while let Some((folder, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut deeper = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if depth < 6 {
                    deeper.push((path, depth + 1));
                }
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if wanted.contains(name) && !out.iter().any(|(n, _)| n == name) {
                    out.push((name.to_string(), path));
                }
            }
        }
        deeper.sort();
        stack.extend(deeper.into_iter().rev());
    }
    out
}

/// Map `path` for the life of the process; read it where it cannot be
/// mapped.
fn map(path: &Path) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: a system font, mapped read-only. Mapping is unsound only if
    // the file changes underneath, and font files are not rewritten in
    // place: Windows refuses to while they are mapped, macOS keeps them on
    // its read-only system volume, and a package manager replaces a file
    // rather than writing into it.
    match unsafe { memmap2::Mmap::map(&file) } {
        Ok(mapped) => Some(&**Box::leak(Box::new(mapped))),
        Err(_) => std::fs::read(path).ok().map(|bytes| &*Box::leak(bytes.into_boxed_slice())),
    }
}

/// Into `fonts`, behind every font already there: each script's regular
/// face for body text and code, and its bold one for `semibold`.
pub fn add(fonts: &mut egui::FontDefinitions, semibold: &egui::FontFamily) {
    add_found(fonts, found(), semibold);
}

fn add_found(fonts: &mut egui::FontDefinitions, found: &Found, semibold: &egui::FontFamily) {
    let mut put = |face: &Face, families: &[egui::FontFamily]| {
        let key = face.key();
        fonts.font_data.entry(key.clone()).or_insert_with(|| egui::FontData {
            font: Cow::Borrowed(face.bytes),
            index: face.index,
            tweak: Default::default(),
        });
        for family in families {
            let list = fonts.families.entry(family.clone()).or_default();
            if !list.contains(&key) {
                list.push(key.clone());
            }
        }
    };
    for face in &found.regular {
        put(face, &[egui::FontFamily::Proportional, egui::FontFamily::Monospace]);
    }
    for face in &found.bold {
        put(face, std::slice::from_ref(semibold));
    }
}

// --- reading a font file's names ---------------------------------------

/// The face of `bytes` called `wanted`, or the first face when it is one
/// face or none is called that. `None` when the face is not one egui can
/// draw from: it would stop the application at start-up rather than
/// decline.
pub fn face_named(bytes: &[u8], wanted: &str) -> Option<u32> {
    let offsets = face_offsets(bytes)?;
    let named = (offsets.len() > 1)
        .then(|| offsets.iter().position(|&offset| names_of(bytes, offset).is_some_and(|names| names.is(wanted))))
        .flatten()
        .unwrap_or(0);
    usable(bytes, offsets[named]).then_some(named as u32)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Where each face's table directory starts: one for a font, several for a
/// collection.
fn face_offsets(bytes: &[u8]) -> Option<Vec<usize>> {
    if bytes.get(..4)? == b"ttcf" {
        let count = u32_at(bytes, 8)? as usize;
        if count == 0 || count > 256 {
            return None;
        }
        (0..count).map(|i| u32_at(bytes, 12 + 4 * i).map(|o| o as usize)).collect()
    } else {
        Some(vec![0])
    }
}

/// The face's table of `tag`: where it starts and how long it is, inside
/// the file.
fn table(bytes: &[u8], face: usize, tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = u16_at(bytes, face + 4)? as usize;
    (0..count).find_map(|i| {
        let record = face + 12 + 16 * i;
        (bytes.get(record..record + 4)? == tag).then_some(())?;
        let (offset, length) = (u32_at(bytes, record + 8)? as usize, u32_at(bytes, record + 12)? as usize);
        (offset.checked_add(length)? <= bytes.len()).then_some((offset, length))
    })
}

/// Whether egui can draw from the face at `face`: an outline font with the
/// tables its reader insists on, and a size of em it accepts.
fn usable(bytes: &[u8], face: usize) -> bool {
    let version = u32_at(bytes, face);
    // TrueType outlines (version 1.0, or Apple's `true`), or CFF ones (`OTTO`).
    let outline = (matches!(version, Some(0x0001_0000) | Some(0x7472_7565)) && table(bytes, face, b"glyf").is_some() && table(bytes, face, b"loca").is_some())
        || (version == Some(0x4F54_544F) && (table(bytes, face, b"CFF ").is_some() || table(bytes, face, b"CFF2").is_some()));
    let units = table(bytes, face, b"head").filter(|(_, length)| *length >= 54).and_then(|(at, _)| u16_at(bytes, at + 18));
    outline
        && units.is_some_and(|u| (16..=16384).contains(&u))
        && [b"cmap", b"hhea", b"hmtx", b"maxp"].iter().all(|tag| table(bytes, face, tag).is_some())
}

/// The names a face goes by.
#[derive(Debug, Default, PartialEq)]
struct Names {
    family: Option<String>,
    style: Option<String>,
    full: Option<String>,
}

impl Names {
    /// Whether this is the face called `wanted`: by its full name, by its
    /// family and style, or by its family alone when it is the family's
    /// regular face. A name the Mac hides from font menus with a leading
    /// dot (".PingFang SC") counts as the name.
    fn is(&self, wanted: &str) -> bool {
        let same = |a: &str| a.trim_start_matches('.').eq_ignore_ascii_case(wanted);
        let family = self.family.as_deref().unwrap_or("").trim_start_matches('.');
        let style = self.style.as_deref().unwrap_or("Regular");
        self.full.as_deref().is_some_and(same)
            || same(&format!("{family} {style}"))
            || (same(family) && ["Regular", "Normal", "Book", "Roman"].iter().any(|s| s.eq_ignore_ascii_case(style)))
    }
}

/// The face's names from its `name` table, in English where it has them in
/// several languages: the typographic family and style where there are
/// (a family of more than four weights), else the plain ones.
fn names_of(bytes: &[u8], face: usize) -> Option<Names> {
    let (at, length) = table(bytes, face, b"name")?;
    let name = bytes.get(at..at + length)?;
    let count = u16_at(name, 2)? as usize;
    let strings = u16_at(name, 4)? as usize;
    // Best so far for each name id: (rank, text). Windows' English first,
    // then Unicode's, then Windows' in another language, then the Mac's.
    let mut best: [(u8, Option<String>); 5] = Default::default();
    for i in 0..count {
        let record = 6 + 12 * i;
        let (platform, language, id) = (u16_at(name, record)?, u16_at(name, record + 4)?, u16_at(name, record + 6)?);
        let slot = match id {
            1 => 0,
            2 => 1,
            4 => 2,
            16 => 3,
            17 => 4,
            _ => continue,
        };
        let rank = match (platform, language) {
            (3, 0x0409) => 4,
            (0, _) => 3,
            (3, _) => 2,
            (1, 0) => 1,
            _ => continue,
        };
        if rank <= best[slot].0 {
            continue;
        }
        let (size, offset) = (u16_at(name, record + 8)? as usize, u16_at(name, record + 10)? as usize);
        let Some(raw) = name.get(strings + offset..strings + offset + size) else { continue };
        let text = if platform == 1 {
            raw.iter().map(|&b| if b.is_ascii() { b as char } else { '?' }).collect()
        } else {
            let units: Vec<u16> = raw.chunks_exact(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
            String::from_utf16_lossy(&units)
        };
        best[slot] = (rank, Some(text));
    }
    let [family, style, full, typographic_family, typographic_style] = best.map(|(_, text)| text);
    Some(Names { family: typographic_family.or(family), style: typographic_style.or(style), full })
}

// --- Devanagari in drawing order ------------------------------------------

/// `text` with each Devanagari vowel sign ि (written before the consonant it
/// follows in the text) moved in front of that consonant, for egui to draw
/// left to right where a shaping engine would put it. Text with none comes
/// back as it is, unallocated.
///
/// Before the consonant alone, not the whole cluster: egui shows a
/// conjunct as its letters with the virama between them, and the sign
/// belongs before the last of those letters (as a shaper puts it when a
/// zero-width non-joiner breaks the conjunct).
pub fn drawing_order(text: &str) -> Cow<'_, str> {
    const SIGN_I: char = '\u{093F}';
    if !text.contains(SIGN_I) {
        return Cow::Borrowed(text);
    }
    let consonant = |c: char| matches!(c, '\u{0915}'..='\u{0939}' | '\u{0958}'..='\u{095F}' | '\u{0978}'..='\u{097F}');
    let nukta = |c: char| c == '\u{093C}';
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    for c in text.chars() {
        if c == SIGN_I {
            // Back past a nukta to its consonant.
            let mut at = out.len();
            while at > 0 && nukta(out[at - 1]) {
                at -= 1;
            }
            if at > 0 && consonant(out[at - 1]) {
                out.insert(at - 1, c);
                continue;
            }
        }
        out.push(c);
    }
    Cow::Owned(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The test strings are written with escapes: the glyph test in `theme`
    // reads every literal in the application's source, and egui's own
    // fonts, which it asks, have no CJK or Devanagari.

    /// 测试 日本語 한국어 हिन्दी Español
    const SAMPLE: &str = "\u{6D4B}\u{8BD5} \u{65E5}\u{672C}\u{8A9E} \u{D55C}\u{AD6D}\u{C5B4} \u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940} Espa\u{00F1}ol";

    /// Each platform looks for its own interface fonts first, for every
    /// script and both weights, and Linux for Noto.
    #[test]
    fn each_platform_looks_for_its_own_fonts() {
        use Platform::*;
        use Script::*;
        let first = |platform, script, bold| candidates(platform, script, bold)[0];
        assert_eq!(first(Windows, SimplifiedChinese, false), font!("msyh.ttc", "Microsoft YaHei UI"));
        assert_eq!(first(Windows, TraditionalChinese, false), font!("msjh.ttc", "Microsoft JhengHei UI"));
        assert_eq!(first(Windows, Japanese, false), font!("YuGothM.ttc", "Yu Gothic UI"));
        assert!(candidates(Windows, Japanese, false).iter().any(|c| c.face == "Meiryo UI"));
        assert_eq!(first(Windows, Korean, false), font!("malgun.ttf", "Malgun Gothic"));
        assert_eq!(first(Windows, Devanagari, false), font!("Nirmala.ttc", "Nirmala UI"));
        assert_eq!(first(Windows, Devanagari, true), font!("Nirmala.ttc", "Nirmala UI Bold"));
        assert_eq!(first(Mac, SimplifiedChinese, false).face, "PingFang SC");
        assert_eq!(first(Mac, TraditionalChinese, false).face, "PingFang TC");
        assert_eq!(first(Mac, Japanese, false).face, "Hiragino Sans W3");
        assert_eq!(first(Mac, Korean, false).face, "Apple SD Gothic Neo");
        assert_eq!(first(Mac, Devanagari, false).face, "Kohinoor Devanagari");
        assert_eq!(first(Linux, SimplifiedChinese, false), font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK SC"));
        assert_eq!(first(Linux, Japanese, false), font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK JP"));
        assert_eq!(first(Linux, Korean, false), font!("NotoSansCJK-Regular.ttc", "Noto Sans CJK KR"));
        assert_eq!(first(Linux, Devanagari, false), font!("NotoSansDevanagari-Regular.ttf", "Noto Sans Devanagari"));
        for platform in [Windows, Mac, Linux] {
            for script in [SimplifiedChinese, TraditionalChinese, Japanese, Korean, Devanagari] {
                for bold in [false, true] {
                    let list = candidates(platform, script, bold);
                    assert!(!list.is_empty(), "{platform:?} {script:?} {bold}");
                    // A variable font draws its default instance, which is
                    // not always a regular weight.
                    assert!(list.iter().all(|c| !c.file.contains("VF")), "{list:?}");
                }
            }
        }
    }

    /// Where each platform looks: Windows' Fonts folder and the person's
    /// own, the Mac's system, library and user folders, and fontconfig's.
    #[test]
    fn each_platform_looks_in_its_font_folders() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| OsString::from(*v));
        let windows = folders(Platform::Windows, env(&[("WINDIR", "D:\\Win"), ("LOCALAPPDATA", "C:\\Users\\Ana\\AppData\\Local")]));
        assert_eq!(windows, vec![PathBuf::from("D:\\Win").join("Fonts"), PathBuf::from("C:\\Users\\Ana\\AppData\\Local").join("Microsoft").join("Windows").join("Fonts")]);
        assert_eq!(folders(Platform::Windows, env(&[]))[0], PathBuf::from("C:\\Windows").join("Fonts"));
        let mac = folders(Platform::Mac, env(&[("HOME", "/Users/ana")]));
        assert_eq!(mac[0], PathBuf::from("/System/Library/Fonts"));
        assert!(mac.contains(&PathBuf::from("/Library/Fonts")));
        assert!(mac.contains(&PathBuf::from("/Users/ana/Library/Fonts")));
        let linux = folders(Platform::Linux, env(&[("HOME", "/home/ana")]));
        assert_eq!(
            linux,
            ["/home/ana/.local/share/fonts", "/usr/local/share/fonts", "/usr/share/fonts", "/home/ana/.fonts"].map(PathBuf::from).to_vec()
        );
    }

    /// A Japanese desktop draws kanji from the Japanese font, a Taiwanese or
    /// Hong Kong one from the Traditional Chinese font, a Korean one from
    /// the Korean font, and every other from the Simplified Chinese one.
    #[test]
    fn the_han_font_follows_the_language() {
        use Script::*;
        for (locale, first) in [
            ("zh-CN", SimplifiedChinese),
            ("zh_CN.UTF-8", SimplifiedChinese),
            ("zh-Hans-SG", SimplifiedChinese),
            ("zh-TW", TraditionalChinese),
            ("zh_HK.UTF-8", TraditionalChinese),
            ("zh-Hant-MO", TraditionalChinese),
            ("ja-JP", Japanese),
            ("ja_JP.UTF-8", Japanese),
            ("ko-KR", Korean),
            ("en-PH", SimplifiedChinese),
            ("es-ES", SimplifiedChinese),
            ("", SimplifiedChinese),
        ] {
            let order = han_order(locale);
            assert_eq!(order[0], first, "{locale}");
            // Each once.
            for script in [SimplifiedChinese, TraditionalChinese, Japanese, Korean] {
                assert_eq!(order.iter().filter(|s| **s == script).count(), 1, "{locale}");
            }
        }
    }

    /// A minimal font for the name and face tests: the tables egui insists
    /// on, empty, and a `name` table saying `family`, `style` and `full`.
    fn fake_face(family: &str, style: &str, full: &str) -> Vec<u8> {
        let mut name = Vec::new();
        let strings: Vec<(u16, Vec<u8>)> = [(1u16, family), (2, style), (4, full)]
            .iter()
            .map(|(id, text)| (*id, text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()))
            .collect();
        name.extend(0u16.to_be_bytes());
        name.extend((strings.len() as u16).to_be_bytes());
        name.extend((6 + 12 * strings.len() as u16).to_be_bytes());
        let mut offset = 0u16;
        for (id, text) in &strings {
            for v in [3u16, 1, 0x0409, *id, text.len() as u16, offset] {
                name.extend(v.to_be_bytes());
            }
            offset += text.len() as u16;
        }
        for (_, text) in &strings {
            name.extend(text);
        }
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&2048u16.to_be_bytes());
        let tables: Vec<(&[u8; 4], Vec<u8>)> =
            vec![(b"cmap", vec![0; 4]), (b"glyf", vec![0; 4]), (b"head", head), (b"hhea", vec![0; 36]), (b"hmtx", vec![0; 4]), (b"loca", vec![0; 4]), (b"maxp", vec![0; 6]), (b"name", name)];
        let mut out = Vec::new();
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0u8; 6]);
        let mut at = 12 + 16 * tables.len();
        let mut data = Vec::new();
        for (tag, bytes) in &tables {
            out.extend(*tag);
            out.extend(0u32.to_be_bytes());
            out.extend((at as u32).to_be_bytes());
            out.extend((bytes.len() as u32).to_be_bytes());
            data.extend(bytes);
            while data.len() % 4 != 0 {
                data.push(0);
            }
            at = 12 + 16 * tables.len() + data.len();
        }
        out.extend(data);
        out
    }

    /// Faces gathered into a collection, as a `.ttc` holds them.
    fn fake_collection(faces: &[Vec<u8>]) -> Vec<u8> {
        let header = 12 + 4 * faces.len();
        let mut out = b"ttcf".to_vec();
        out.extend(0x0001_0000u32.to_be_bytes());
        out.extend((faces.len() as u32).to_be_bytes());
        let mut at = header;
        let mut body = Vec::new();
        for face in faces {
            out.extend((at as u32).to_be_bytes());
            // The face's table offsets are from the start of the file.
            let mut face = face.clone();
            let count = u16::from_be_bytes([face[4], face[5]]) as usize;
            for i in 0..count {
                let record = 12 + 16 * i + 8;
                let offset = u32::from_be_bytes(face[record..record + 4].try_into().unwrap()) + at as u32;
                face[record..record + 4].copy_from_slice(&offset.to_be_bytes());
            }
            at += face.len();
            body.extend(face);
        }
        out.extend(body);
        out
    }

    /// The face asked for in a collection, by full name or by family and
    /// style; the first where none is called that; one face as it is; and
    /// nothing from a file egui could not read, which would stop the
    /// application rather than be passed over.
    #[test]
    fn the_face_asked_for_is_found_in_a_collection() {
        let collection = fake_collection(&[
            fake_face("Microsoft YaHei", "Regular", "Microsoft YaHei"),
            fake_face("Microsoft YaHei UI", "Regular", "Microsoft YaHei UI"),
            fake_face("Microsoft YaHei UI", "Bold", "Microsoft YaHei UI Bold"),
        ]);
        assert_eq!(face_named(&collection, "Microsoft YaHei UI"), Some(1));
        assert_eq!(face_named(&collection, "Microsoft YaHei UI Bold"), Some(2));
        assert_eq!(face_named(&collection, "microsoft yahei"), Some(0));
        assert_eq!(face_named(&collection, "Somebody Else"), Some(0));
        // A family's regular face by the family's name alone (Noto Sans CJK
        // SC is "Noto Sans CJK SC", "Regular").
        let noto = fake_collection(&[fake_face("Noto Sans CJK JP", "Regular", "Noto Sans CJK JP Regular"), fake_face("Noto Sans CJK SC", "Regular", "Noto Sans CJK SC Regular")]);
        assert_eq!(face_named(&noto, "Noto Sans CJK SC"), Some(1));
        // The Mac's hidden names.
        let pingfang = fake_collection(&[fake_face(".PingFang HK", "Regular", ".PingFang HK Regular"), fake_face(".PingFang SC", "Semibold", ".PingFang SC Semibold")]);
        assert_eq!(face_named(&pingfang, "PingFang SC Semibold"), Some(1));
        let single = fake_face("Malgun Gothic", "Regular", "Malgun Gothic");
        assert_eq!(face_named(&single, "Anything"), Some(0));
        assert_eq!(face_named(b"not a font at all", "Malgun Gothic"), None);
        assert_eq!(face_named(&single[..40], "Malgun Gothic"), None, "cut short");
        let mut no_outlines = single.clone();
        let glyf = no_outlines.windows(4).position(|w| w == b"glyf").unwrap();
        no_outlines[glyf..glyf + 4].copy_from_slice(b"xxxx");
        assert_eq!(face_named(&no_outlines, "Malgun Gothic"), None, "a bitmap font draws nothing");
    }

    /// From a folder of fonts: each script's best face, bold beside it or
    /// the regular face again, in the language's order, and Linux's found
    /// in the folders fontconfig nests them in.
    #[test]
    fn the_fonts_found_follow_the_lists() {
        let root = std::env::temp_dir().join(format!("tequila-fallback-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let windows = root.join("Windows");
        std::fs::create_dir_all(&windows).unwrap();
        let write = |path: PathBuf, bytes: Vec<u8>| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write(windows.join("msyh.ttc"), fake_collection(&[fake_face("Microsoft YaHei", "Regular", "Microsoft YaHei"), fake_face("Microsoft YaHei UI", "Regular", "Microsoft YaHei UI")]));
        write(windows.join("meiryo.ttc"), fake_collection(&[fake_face("Meiryo", "Regular", "Meiryo"), fake_face("Meiryo UI", "Regular", "Meiryo UI")]));
        write(windows.join("Nirmala.ttc"), fake_collection(&[fake_face("Nirmala UI", "Regular", "Nirmala UI"), fake_face("Nirmala UI", "Bold", "Nirmala UI Bold")]));
        let read = |path: &Path| std::fs::read(path).ok().map(|b| &*Box::leak(b.into_boxed_slice()));
        let found = find(Platform::Windows, &[windows.clone()], "ja-JP", &read);
        let summary = |faces: &[Face]| faces.iter().map(|f| (f.script, f.path.file_name().unwrap().to_string_lossy().into_owned(), f.index)).collect::<Vec<_>>();
        // No Yu Gothic: Meiryo UI, first for Japanese; no JhengHei or Malgun.
        assert_eq!(
            summary(&found.regular),
            vec![(Script::Japanese, "meiryo.ttc".into(), 1), (Script::SimplifiedChinese, "msyh.ttc".into(), 1), (Script::Devanagari, "Nirmala.ttc".into(), 0)]
        );
        // No bold YaHei or Meiryo: their regular faces again; Nirmala's bold.
        assert_eq!(
            summary(&found.bold),
            vec![(Script::Japanese, "meiryo.ttc".into(), 1), (Script::SimplifiedChinese, "msyh.ttc".into(), 1), (Script::Devanagari, "Nirmala.ttc".into(), 1)]
        );
        assert!(found.bold[2].bold && !found.bold[0].bold);

        let share = root.join("share").join("fonts");
        let cjk = [("Noto Sans CJK JP", "Noto Sans CJK JP Regular"), ("Noto Sans CJK KR", "Noto Sans CJK KR Regular"), ("Noto Sans CJK SC", "Noto Sans CJK SC Regular"), ("Noto Sans CJK TC", "Noto Sans CJK TC Regular")];
        write(share.join("opentype").join("noto").join("NotoSansCJK-Regular.ttc"), fake_collection(&cjk.map(|(f, n)| fake_face(f, "Regular", n))));
        write(share.join("truetype").join("noto").join("NotoSansDevanagari-Regular.ttf"), fake_face("Noto Sans Devanagari", "Regular", "Noto Sans Devanagari Regular"));
        let found = find(Platform::Linux, &[root.join("nowhere"), share.clone()], "zh_TW.UTF-8", &read);
        assert_eq!(
            summary(&found.regular),
            vec![
                (Script::TraditionalChinese, "NotoSansCJK-Regular.ttc".into(), 3),
                (Script::SimplifiedChinese, "NotoSansCJK-Regular.ttc".into(), 2),
                (Script::Japanese, "NotoSansCJK-Regular.ttc".into(), 0),
                (Script::Korean, "NotoSansCJK-Regular.ttc".into(), 1),
                (Script::Devanagari, "NotoSansDevanagari-Regular.ttf".into(), 0),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fonts found go behind every font the interface had, in each
    /// family, so a character any of those has still comes from it.
    #[test]
    fn the_fonts_go_behind_the_interface_fonts() {
        let face = |script, bold| Face { script, bold, path: PathBuf::from("x.ttc"), index: 1, bytes: b"" };
        let found = Found {
            regular: vec![face(Script::SimplifiedChinese, false), face(Script::Devanagari, false)],
            bold: vec![face(Script::SimplifiedChinese, true), face(Script::Devanagari, false)],
        };
        let semibold = egui::FontFamily::Name("semibold".into());
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(semibold.clone(), vec!["Ubuntu-Light".into()]);
        let before = fonts.families.clone();
        add_found(&mut fonts, &found, &semibold);
        for (family, extra) in [
            (egui::FontFamily::Proportional, vec!["zh-hans", "deva"]),
            (egui::FontFamily::Monospace, vec!["zh-hans", "deva"]),
            (semibold.clone(), vec!["zh-hans-bold", "deva"]),
        ] {
            let list = &fonts.families[&family];
            let had = &before[&family];
            assert_eq!(&list[..had.len()], &had[..], "{family:?}: what was there stays in front");
            assert_eq!(&list[had.len()..], &extra.iter().map(|s| s.to_string()).collect::<Vec<_>>()[..], "{family:?}");
        }
        assert_eq!(fonts.font_data["zh-hans"].index, 1);
    }

    /// Every character of a sample in five languages lays out with a glyph,
    /// none of them the empty box, on a computer that has the fonts; and
    /// Latin text lays out exactly as it did without them. Skipped for a
    /// script whose font this computer lacks.
    #[test]
    fn names_in_five_languages_lay_out_without_boxes() {
        let found = found();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::font_definitions());
        let _ = ctx.run(Default::default(), |_| {});
        let font = egui::FontId::proportional(13.0);
        // Which fonts have which characters: Hangul only the Korean one,
        // the Simplified forms of 测试 both Chinese ones.
        let covered = |c: char| -> bool {
            let fonts: &[Script] = match c {
                '\u{0900}'..='\u{097F}' => &[Script::Devanagari],
                '\u{AC00}'..='\u{D7AF}' => &[Script::Korean],
                '\u{4E00}'..='\u{9FFF}' => &[Script::SimplifiedChinese, Script::TraditionalChinese],
                _ => return true,
            };
            found.regular.iter().any(|f| fonts.contains(&f.script))
        };
        let mut checked = 0;
        for c in SAMPLE.chars().filter(|c| !c.is_whitespace()) {
            if !covered(c) {
                continue;
            }
            assert!(ctx.fonts(|f| f.has_glyph(&font, c)), "{c:?} (U+{:04X}) draws as a box", c as u32);
            checked += 1;
        }
        if found.regular.is_empty() {
            eprintln!("no CJK or Devanagari font on this computer: only the Spanish was checked");
        }
        assert!(checked >= "Espanol".len());

        // Latin, laid out with and without the fallbacks: the same glyphs in
        // the same places.
        let plain = egui::Context::default();
        let mut without = crate::theme::font_definitions();
        for family in without.families.values_mut() {
            family.retain(|name| !found.regular.iter().chain(&found.bold).any(|f| f.key() == *name));
        }
        plain.set_fonts(without);
        let _ = plain.run(Default::default(), |_| {});
        let lay = |ctx: &egui::Context| {
            ctx.fonts(|f| f.layout_no_wrap("Suela de cuero, a\u{00F1}o 2026 \u{00B7} \u{0393}\u{03B5}\u{03B9}\u{03AC} \u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}".into(), font.clone(), egui::Color32::WHITE))
        };
        let (with, without) = (lay(&ctx), lay(&plain));
        let places = |g: &egui::Galley| g.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.chr, g.pos, g.uv_rect.offset))).collect::<Vec<_>>();
        assert_eq!(places(&with), places(&without));
        assert_eq!(with.size(), without.size());
    }

    /// The vowel sign ि before its consonant (past a nukta, and before the
    /// last letter of a conjunct shown with its virama), and text without
    /// it untouched.
    #[test]
    fn the_vowel_sign_i_goes_before_its_consonant() {
        // हिन्दी: ह ि न ् द ी -> ि ह न ् द ी
        assert_eq!(drawing_order("\u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940}"), "\u{093F}\u{0939}\u{0928}\u{094D}\u{0926}\u{0940}");
        // किताब (book): क ि त ा ब
        assert_eq!(drawing_order("\u{0915}\u{093F}\u{0924}\u{093E}\u{092C}"), "\u{093F}\u{0915}\u{0924}\u{093E}\u{092C}");
        // ज़िला (district), with a nukta: ज ़ ि ल ा -> ि ज ़ ल ा
        assert_eq!(drawing_order("\u{091C}\u{093C}\u{093F}\u{0932}\u{093E}"), "\u{093F}\u{091C}\u{093C}\u{0932}\u{093E}");
        // स्थिति: स ् थ ि त ि -> स ् ि थ ि त
        assert_eq!(drawing_order("\u{0938}\u{094D}\u{0925}\u{093F}\u{0924}\u{093F}"), "\u{0938}\u{094D}\u{093F}\u{0925}\u{093F}\u{0924}");
        // A sign with no consonant before it stays where it is.
        assert_eq!(drawing_order("\u{093F}x"), "\u{093F}x");
        assert!(matches!(drawing_order("Espa\u{00F1}ol \u{6D4B}\u{8BD5}"), Cow::Borrowed(_)));
    }
}

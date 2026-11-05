//! Adobe Substance materials: `.sbsar` archives and `.sbs` sources, read
//! for their settings.
//!
//! A Substance material is a procedural graph authored in Substance 3D
//! Designer. Its pictures — colour, normal, roughness and the rest — are
//! made by Adobe's Substance engine from the graph and the values of its
//! inputs, and only that engine can make them. Tequila does not ship it and
//! does not imitate it. What Tequila can do on its own is read what a file
//! *says about itself*: its graphs, their inputs with their labels, groups,
//! types, defaults, ranges and widgets, their presets, their outputs and what
//! each output is for, and the picture it carries of itself. That is enough
//! to show a material's settings and to keep the person's choices; the maps
//! are rendered from those choices by Adobe's own command-line tool,
//! `sbsrender`, when it is installed (`bridge.rs`).
//!
//! # The files
//!
//! An `.sbsar` is a 7-Zip archive (Adobe's `sbscooker --no-archive` writes
//! the same content unpacked: "SBSASM and XML"). Inside are the compiled
//! graph (`.sbsasm`, which only the engine reads), an XML description of it,
//! and an icon per graph when it was published with one. The description's
//! root is `<sbsdescription>`, holding `<graphs><graph>` elements with
//! `<inputs><input>`, `<outputs><output>` and `<sbspresets><sbspreset>`; the
//! attribute names are the ones Adobe's Python API (pysbs, `sbsarchive`)
//! documents as members (`mPkgUrl` is `pkgurl`, `mAlterOutputs` is
//! `alteroutputs` and so on). An input's type is the `SBSARTypeEnum`:
//! float 1 to 4 are 0 to 3, integer 1 is 4, image 5, string 6, font 7, and
//! integer 2 to 4 are 8 to 10.
//!
//! An `.sbs` is Designer's own source, plain XML of a different shape: a
//! `<package>` whose `<content>` holds `<graph>` elements, every value in a
//! `v` attribute (`<identifier v="bricks"/>`), each input a `<paraminput>`
//! with a `<defaultValue>` of a typed constant (`<constantValueFloat1>`) and a
//! `<defaultWidget>` of named options (`min`, `max`, `step`, `clamp`), and
//! each output a `<graphoutput>` with its `<usages>`. Types there are pysbs's
//! `ParamTypeEnum`, flags rather than counts: boolean 4, integer 1 to 4 are
//! 16, 32, 64 and 128, float 1 to 4 are 256 to 2048, and the image entries
//! 1, 2 and 3. An `.sbs` does not list `$outputsize` and `$randomseed`:
//! the cooker adds them (`--expose-output-size` and `--expose-random-seed`
//! default to yes), so they are added here too.
//!
//! None of this has been checked against a file made by Adobe's tools: the
//! machine this was written on had neither the tools nor a sample. The
//! tests read synthetic files written from the published format
//! ([`fixture`]). Where the documentation is silent — how an `.sbs`
//! encodes a drop-down's items, where its presets live — the reader accepts
//! the plausible spellings and says so.

pub mod bridge;
#[doc(hidden)]
pub mod fake;
#[doc(hidden)]
pub mod fixture;

use crate::texture_set::{Map, Slot, TextureSet};
use crate::ImportError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which of the two kinds of file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A published, compiled material: what Substance Painter, Stager and
    /// the Substance Source library hand out.
    Sbsar,
    /// Designer's source, which Adobe's `sbscooker` turns into an `.sbsar`
    /// before it can be rendered.
    Sbs,
}

impl FileKind {
    pub fn of(path: &Path) -> Option<FileKind> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "sbsar" => Some(FileKind::Sbsar),
            "sbs" => Some(FileKind::Sbs),
            _ => None,
        }
    }
}

/// A Substance file, as it describes itself.
#[derive(Clone, Debug, PartialEq)]
pub struct SubstanceFile {
    pub kind: FileKind,
    /// The description's own format version, for the report.
    pub format_version: String,
    pub graphs: Vec<Graph>,
}

impl SubstanceFile {
    /// The graphs a person picks from: the ones not hidden from libraries.
    /// A file whose every graph is hidden offers them all rather than none.
    pub fn visible_graphs(&self) -> Vec<&Graph> {
        let shown: Vec<&Graph> = self.graphs.iter().filter(|g| !g.hidden).collect();
        if shown.is_empty() {
            self.graphs.iter().collect()
        } else {
            shown
        }
    }

    /// The graph `url` names, by its package URL or the identifier at its
    /// end; the first visible graph for `None`.
    pub fn graph(&self, url: Option<&str>) -> Option<&Graph> {
        match url {
            None => self.visible_graphs().into_iter().next(),
            Some(url) => self
                .graphs
                .iter()
                .find(|g| g.url == url)
                .or_else(|| self.graphs.iter().find(|g| g.identifier() == identifier_of(url))),
        }
    }
}

/// `pkg://folder/bricks` as `bricks`.
fn identifier_of(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// One graph: one material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    /// Its package URL, `pkg://bricks`, which is how `sbsrender` is told
    /// which graph to render (`--input-graph`).
    pub url: String,
    pub label: String,
    pub category: String,
    pub author: String,
    pub author_url: String,
    pub description: String,
    /// Tags, separated by spaces in the file.
    pub keywords: String,
    /// Hidden from libraries (`hideInLibrary`): a helper graph, not a
    /// material to pick.
    pub hidden: bool,
    /// The real-world size the author gave it, in centimetres across, down
    /// and deep, when the file says (`physicalSize`).
    pub physical_size: Option<[f32; 3]>,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub presets: Vec<Preset>,
    /// The picture it was published with, as the file's own bytes (a PNG,
    /// almost always).
    pub thumbnail: Option<Vec<u8>>,
}

impl Graph {
    pub fn identifier(&self) -> &str {
        identifier_of(&self.url)
    }

    /// The name to show: its label, or its identifier when it has none.
    pub fn name(&self) -> String {
        if self.label.trim().is_empty() {
            self.identifier().replace('_', " ")
        } else {
            self.label.trim().to_string()
        }
    }

    pub fn input(&self, identifier: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.identifier == identifier)
    }

    /// The inputs a person sets, in the file's order, grouped by their group
    /// in the order each group first appears. The system inputs
    /// (`$outputsize`, `$randomseed` and the rest) are left out: they are
    /// shown on their own.
    pub fn groups(&self) -> Vec<(String, Vec<&Input>)> {
        let mut groups: Vec<(String, Vec<&Input>)> = Vec::new();
        for input in self.inputs.iter().filter(|i| !i.is_system()) {
            match groups.iter_mut().find(|(g, _)| *g == input.group) {
                Some((_, members)) => members.push(input),
                None => groups.push((input.group.clone(), vec![input])),
            }
        }
        groups
    }

    /// Whether this graph's normal maps come out in DirectX's convention
    /// (green pointing down) for these values. A graph with a Normal format
    /// input (`normal_format`, as every Substance Source material has, or
    /// the newer system input `$normalformat`) says: 0 is DirectX, 1 OpenGL.
    /// One without draws DirectX's, the engine's default — Adobe's own
    /// USD plug-in assumes the same.
    pub fn normal_is_directx(&self, values: &BTreeMap<String, String>) -> bool {
        for id in ["normal_format", "$normalformat"] {
            if let Some(input) = self.input(id) {
                let value = values.get(id).and_then(|v| Value::parse(v).first().copied()).or_else(|| input.default.first());
                return value.unwrap_or(0.0).round() as i64 == 0;
            }
        }
        true
    }
}

/// What an input holds, as the file types it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// One to four floats.
    Float(u8),
    /// One to four integers.
    Integer(u8),
    /// A picture the graph takes in (`--set-entry`).
    Image,
    String,
    Font,
}

impl ValueType {
    /// From the `.sbsar` description's `type` attribute.
    fn from_sbsar(code: i64) -> Option<ValueType> {
        Some(match code {
            0..=3 => ValueType::Float(code as u8 + 1),
            4 => ValueType::Integer(1),
            5 => ValueType::Image,
            6 => ValueType::String,
            7 => ValueType::Font,
            8..=10 => ValueType::Integer(code as u8 - 6),
            _ => return None,
        })
    }

    /// The `.sbsar` code, for writing one.
    pub fn sbsar_code(self) -> i64 {
        match self {
            ValueType::Float(n) => n.clamp(1, 4) as i64 - 1,
            ValueType::Integer(1) => 4,
            ValueType::Integer(n) => n.clamp(2, 4) as i64 + 6,
            ValueType::Image => 5,
            ValueType::String => 6,
            ValueType::Font => 7,
        }
    }

    /// From an `.sbs` `<type>`: pysbs's `ParamTypeEnum` flags.
    fn from_sbs(code: i64) -> Option<ValueType> {
        Some(match code {
            1..=3 => ValueType::Image,
            4 | 16 => ValueType::Integer(1),
            32 => ValueType::Integer(2),
            64 => ValueType::Integer(3),
            128 => ValueType::Integer(4),
            256 => ValueType::Float(1),
            512 => ValueType::Float(2),
            1024 => ValueType::Float(3),
            2048 => ValueType::Float(4),
            16384 => ValueType::String,
            _ => return None,
        })
    }

    /// From the tag of an `.sbs` constant: `constantValueFloat3`.
    fn from_constant_tag(tag: &str) -> Option<ValueType> {
        let rest = tag.strip_prefix("constantValue")?;
        Some(match rest {
            "Bool" => ValueType::Integer(1),
            "String" => ValueType::String,
            "Int32" | "Int1" | "Int" => ValueType::Integer(1),
            _ => {
                if let Some(n) = rest.strip_prefix("Float").and_then(|n| n.parse::<u8>().ok()) {
                    ValueType::Float(n.clamp(1, 4))
                } else if let Some(n) = rest.strip_prefix("Int").and_then(|n| n.parse::<u8>().ok()) {
                    ValueType::Integer(n.clamp(1, 4))
                } else {
                    return None;
                }
            }
        })
    }

    pub fn components(self) -> usize {
        match self {
            ValueType::Float(n) | ValueType::Integer(n) => n as usize,
            _ => 0,
        }
    }
}

/// How the file asks for an input to be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Widget {
    Slider,
    Angle,
    Color,
    /// A drop-down of named whole numbers.
    ComboBox,
    /// The same choice as a row of buttons.
    EnumButtons,
    /// A switch: an integer that is 0 or 1.
    Toggle,
    Position,
    Image,
    Text,
    /// Anything else the file names — an offset, a transform — shown as
    /// plain numbers.
    Other,
}

impl Widget {
    fn from_name(name: &str) -> Widget {
        match name.to_ascii_lowercase().as_str() {
            "slider" => Widget::Slider,
            "angle" => Widget::Angle,
            "color" | "colour" => Widget::Color,
            "combobox" | "dropdownlist" | "dropdown" => Widget::ComboBox,
            "enumbuttons" => Widget::EnumButtons,
            "togglebutton" | "buttons" | "checkbox" => Widget::Toggle,
            "position" => Widget::Position,
            "image" => Widget::Image,
            "text" | "string" => Widget::Text,
            _ => Widget::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Widget::Slider => "slider",
            Widget::Angle => "angle",
            Widget::Color => "color",
            Widget::ComboBox => "combobox",
            Widget::EnumButtons => "enumbuttons",
            Widget::Toggle => "togglebutton",
            Widget::Position => "position",
            Widget::Image => "image",
            Widget::Text => "text",
            Widget::Other => "other",
        }
    }
}

/// A value: numbers, or text for a string input.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Numbers(Vec<f64>),
    Text(String),
}

impl Default for Value {
    fn default() -> Value {
        Value::Numbers(Vec::new())
    }
}

impl Value {
    /// Numbers from text, however the file separates them: commas in an
    /// `.sbsar`, spaces in an `.sbs` constant, semicolons in its widget
    /// options.
    pub fn parse(text: &str) -> Vec<f64> {
        text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .collect()
    }

    fn typed(text: &str, value_type: ValueType) -> Value {
        match value_type {
            ValueType::String | ValueType::Font | ValueType::Image => Value::Text(text.to_string()),
            _ => Value::Numbers(Value::parse(text)),
        }
    }

    pub fn numbers(&self) -> &[f64] {
        match self {
            Value::Numbers(n) => n,
            Value::Text(_) => &[],
        }
    }

    pub fn first(&self) -> Option<f64> {
        self.numbers().first().copied()
    }

    /// As `sbsrender --set-value` takes it, and as a material keeps it:
    /// numbers joined by commas, whole numbers without a decimal point.
    pub fn to_text(&self, value_type: ValueType) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Numbers(n) => format_numbers(n, matches!(value_type, ValueType::Integer(_))),
        }
    }
}

/// Numbers as `0.5,1,0.25`: integers rounded, floats to six places with
/// the trailing zeros taken off.
pub fn format_numbers(numbers: &[f64], integers: bool) -> String {
    numbers
        .iter()
        .map(|&v| {
            if integers {
                format!("{}", v.round() as i64)
            } else {
                let text = format!("{v:.6}");
                let text = text.trim_end_matches('0').trim_end_matches('.');
                if text == "-0" { "0".to_string() } else { text.to_string() }
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// One of a graph's inputs: a setting, or a picture it takes in.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub identifier: String,
    pub uid: String,
    pub label: String,
    /// Its group in the panel; `/` separates nested groups.
    pub group: String,
    pub description: String,
    pub value_type: ValueType,
    pub widget: Widget,
    pub default: Value,
    pub min: Option<Vec<f64>>,
    pub max: Option<Vec<f64>>,
    pub step: Option<f64>,
    /// Whether values outside min and max are refused rather than allowed.
    pub clamp: bool,
    /// A slider's names for its components (`label0` to `label3`), or a
    /// switch's for off and on.
    pub labels: Vec<String>,
    /// A drop-down's items: the number, and its name.
    pub choices: Vec<(i64, String)>,
    /// When to show it, as the file writes it (`input["has_dirt"]==true`).
    /// Kept for the report; not evaluated.
    pub visible_if: String,
    /// What an image input is for (`baseColor`, `mask`), as outputs' usages.
    pub usages: Vec<String>,
}

impl Input {
    fn new(identifier: &str, value_type: ValueType) -> Input {
        Input {
            identifier: identifier.to_string(),
            uid: String::new(),
            label: String::new(),
            group: String::new(),
            description: String::new(),
            value_type,
            widget: Widget::Other,
            default: match value_type {
                ValueType::String | ValueType::Font | ValueType::Image => Value::Text(String::new()),
                _ => Value::default(),
            },
            min: None,
            max: None,
            step: None,
            clamp: false,
            labels: Vec::new(),
            choices: Vec::new(),
            visible_if: String::new(),
            usages: Vec::new(),
        }
    }

    /// The engine's own inputs — `$outputsize`, `$randomseed`,
    /// `$pixelsize`, `$normalformat`... — named with a `$`.
    pub fn is_system(&self) -> bool {
        self.identifier.starts_with('$')
    }

    /// The name to show.
    pub fn name(&self) -> String {
        if !self.label.trim().is_empty() {
            return self.label.trim().to_string();
        }
        match self.identifier.as_str() {
            "$outputsize" => "Output size".into(),
            "$randomseed" => "Random seed".into(),
            other => other.trim_start_matches('$').replace('_', " "),
        }
    }

    /// What kind of control it wants, from its type and widget together.
    pub fn kind(&self) -> InputKind {
        match (self.value_type, self.widget) {
            (ValueType::Image, _) => InputKind::Image,
            (ValueType::String, _) => InputKind::Text,
            (ValueType::Font, _) => InputKind::Font,
            (ValueType::Float(n @ (3 | 4)), Widget::Color) => InputKind::Color { alpha: n == 4 },
            (ValueType::Float(1), Widget::Color) => InputKind::Float(1),
            (ValueType::Integer(1), Widget::Toggle) => InputKind::Toggle,
            (ValueType::Integer(1), Widget::ComboBox | Widget::EnumButtons) if !self.choices.is_empty() => InputKind::Choice,
            (ValueType::Float(n), _) => InputKind::Float(n),
            (ValueType::Integer(n), _) => InputKind::Integer(n),
        }
    }

    /// The value of this input in `values`, or its default.
    pub fn value_in(&self, values: &BTreeMap<String, String>) -> Value {
        match values.get(&self.identifier) {
            Some(text) => Value::typed(text, self.value_type),
            None => self.default.clone(),
        }
    }
}

/// [`Input::kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Float(u8),
    Integer(u8),
    Toggle,
    Color { alpha: bool },
    Choice,
    Text,
    Font,
    Image,
}

/// One of a graph's outputs: a map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub identifier: String,
    pub uid: String,
    pub label: String,
    pub group: String,
    /// What it is for: `baseColor`, `normal`, `roughness`, `metallic`,
    /// `height`, `ambientOcclusion`, `emissive`, `opacity`...
    pub usages: Vec<String>,
}

impl Output {
    /// The role it plays in a material, from its usages, or — for an
    /// output without any — from the words of its identifier, as a texture
    /// pack's files are read. `Err` with the reason for one Tequila does not
    /// use.
    pub fn slot(&self, directx: bool) -> Result<Slot, String> {
        let usages: Vec<String> = self.usages.iter().map(|u| u.to_ascii_lowercase()).collect();
        let has = |name: &str| usages.iter().any(|u| u == name);
        // Occlusion, roughness and metalness packed into one picture.
        if has("ambientocclusion") && has("roughness") && has("metallic") {
            return Ok(Slot::Packed);
        }
        for usage in &usages {
            if let Some(slot) = usage_slot(usage, directx) {
                return slot;
            }
        }
        if usages.iter().any(|u| u != "any" && !u.is_empty()) {
            return Err(format!("its use ({}) is not one Tequila's materials take", self.usages.join(", ")));
        }
        match crate::texture_set::classify(Path::new(&self.identifier)) {
            Ok((Slot::Normal { .. }, _, _)) => Ok(Slot::Normal { directx }),
            Ok((slot, _, _)) => Ok(slot),
            Err(_) => Err("it says nothing of what it is for".into()),
        }
    }
}

/// A Substance usage's role, if it has one: `Some(Err)` for a usage that is
/// known and not used.
fn usage_slot(usage: &str, directx: bool) -> Option<Result<Slot, String>> {
    Some(Ok(match usage {
        "basecolor" | "albedo" => Slot::BaseColor,
        // The specular/glossiness workflow's diffuse colour: the nearest
        // thing to a base colour that a metal/roughness material has.
        "diffuse" => Slot::BaseColor,
        "roughness" => Slot::Roughness,
        "glossiness" => Slot::Glossiness,
        "metallic" | "metalness" => Slot::Metalness,
        "normal" => Slot::Normal { directx },
        "height" | "displacement" => Slot::Height,
        "ambientocclusion" | "ao" => Slot::Occlusion,
        "opacity" => Slot::Opacity,
        "emissive" | "emission" => Slot::Emission,
        "specular" | "specularlevel" | "specularcolor" | "specularedgecolor" => {
            return Some(Err("a specular map (this is a metal/roughness material)".into()))
        }
        "anisotropylevel" | "anisotropyangle" => return Some(Err("an anisotropy map".into())),
        "coatweight" | "coatopacity" | "coatcolor" | "coatroughness" | "coatnormal" | "coatspecularlevel" => {
            return Some(Err("a coat map".into()))
        }
        "sheencolor" | "sheenopacity" | "sheenroughness" => return Some(Err("a sheen map".into())),
        "scattering" | "scatteringcolor" | "scatteringdistance" | "scatteringdistancescale" | "translucency" => {
            return Some(Err("a subsurface map".into()))
        }
        "transmissive" | "refraction" | "absorptioncolor" | "ior" => return Some(Err("a transmission map".into())),
        "mask" | "curvature" | "thickness" | "position" | "worldspacenormal" | "id" => {
            return Some(Err(format!("a `{usage}` map")))
        }
        _ => return None,
    }))
}

/// A named set of values the author saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub label: String,
    /// Input identifier, and the value as the file writes it.
    pub values: Vec<(String, String)>,
}

impl Preset {
    /// Its values in a material's terms: identifier to value text, typed
    /// by `graph`'s inputs, which drops any it does not have.
    pub fn values_for(&self, graph: &Graph) -> BTreeMap<String, String> {
        self.values
            .iter()
            .filter_map(|(id, text)| {
                let input = graph.input(id)?;
                Some((id.clone(), Value::typed(text, input.value_type).to_text(input.value_type)))
            })
            .collect()
    }
}

// --- reading ---------------------------------------------------------------

/// The largest description read: real ones are kilobytes.
const DESCRIPTION_LIMIT: u64 = 64 << 20;
/// The largest icon read.
const ICON_LIMIT: u64 = 16 << 20;

/// Read a Substance file for its graphs.
pub fn read(path: &Path) -> Result<SubstanceFile, ImportError> {
    let kind = FileKind::of(path).ok_or_else(|| ImportError::UnknownFormat(format!("{} is not an .sbsar or .sbs file", path.display())))?;
    let bytes = std::fs::read(path).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    match kind {
        FileKind::Sbsar => read_sbsar(&bytes),
        FileKind::Sbs => {
            let text = String::from_utf8_lossy(&bytes);
            read_sbs(&text)
        }
    }
}

/// An `.sbsar`'s bytes: the 7-Zip archive, its description and icons.
pub fn read_sbsar(bytes: &[u8]) -> Result<SubstanceFile, ImportError> {
    use std::io::Read;
    if !bytes.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        return Err(ImportError::Malformed("an .sbsar is a 7-Zip archive, and this is not one".into()));
    }
    let mut reader = sevenz_rust2::SevenZReader::new(std::io::Cursor::new(bytes), sevenz_rust2::Password::empty())
        .map_err(|e| ImportError::Malformed(format!("the .sbsar's archive could not be read: {e}")))?;
    let mut descriptions: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pictures: Vec<(String, Vec<u8>)> = Vec::new();
    reader
        .for_each_entries(|entry, data| {
            let name = entry.name().replace('\\', "/");
            let lower = name.to_ascii_lowercase();
            let limit = if lower.ends_with(".xml") {
                DESCRIPTION_LIMIT
            } else if [".png", ".jpg", ".jpeg"].iter().any(|e| lower.ends_with(e)) {
                ICON_LIMIT
            } else {
                // The compiled graph and anything else: read past, unread.
                std::io::copy(data, &mut std::io::sink())?;
                return Ok(true);
            };
            let mut out = Vec::new();
            data.take(limit + 1).read_to_end(&mut out)?;
            // Drain what is left, so a solid archive's next entry starts
            // where it should.
            std::io::copy(data, &mut std::io::sink())?;
            if out.len() as u64 > limit {
                return Ok(true);
            }
            if lower.ends_with(".xml") {
                descriptions.push((name, out));
            } else {
                pictures.push((name, out));
            }
            Ok(true)
        })
        .map_err(|e| ImportError::Malformed(format!("the .sbsar's archive could not be unpacked: {e}")))?;
    // The description is the XML whose root is `sbsdescription`: the first
    // at the archive's top level that is, then any.
    descriptions.sort_by_key(|(name, _)| name.matches('/').count());
    let (_, xml) = descriptions
        .iter()
        .find(|(_, xml)| String::from_utf8_lossy(xml).contains("<sbsdescription"))
        .ok_or_else(|| ImportError::Malformed("the .sbsar holds no description of its graphs".into()))?;
    let text = String::from_utf8_lossy(xml);
    let mut file = read_description(&text)?;
    // Each graph's icon, by the file name its `icon` attribute gives; a
    // file of one graph and one picture takes that picture.
    let icons = icon_names(&text);
    for (graph, icon) in file.graphs.iter_mut().zip(icons) {
        let wanted = icon.as_deref().map(|i| i.rsplit(['/', '\\']).next().unwrap_or(i).to_ascii_lowercase());
        graph.thumbnail = wanted.and_then(|w| {
            pictures
                .iter()
                .find(|(name, _)| name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase() == w)
                .map(|(_, bytes)| bytes.clone())
        });
    }
    if file.graphs.len() == 1 && file.graphs[0].thumbnail.is_none() && pictures.len() == 1 {
        file.graphs[0].thumbnail = Some(pictures[0].1.clone());
    }
    Ok(file)
}

fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, ImportError> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..roxmltree::ParsingOptions::default() };
    roxmltree::Document::parse_with_options(text, options).map_err(|e| ImportError::Malformed(format!("its description is not XML Tequila can read: {e}")))
}

/// Children of `node` named `name`.
fn children<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> + 'a {
    node.children().filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn child<'a, 'input>(node: roxmltree::Node<'a, 'input>, name: &'a str) -> Option<roxmltree::Node<'a, 'input>> {
    children(node, name).next()
}

fn attribute(node: roxmltree::Node, name: &str) -> String {
    node.attribute(name).unwrap_or_default().to_string()
}

/// The `icon` attribute of each graph, in order.
fn icon_names(text: &str) -> Vec<Option<String>> {
    let Ok(document) = parse_xml(text) else { return Vec::new() };
    let root = document.root_element();
    child(root, "graphs")
        .map(|graphs| children(graphs, "graph").map(|g| g.attribute("icon").map(str::to_string)).collect())
        .unwrap_or_default()
}

fn on(text: &str) -> bool {
    matches!(text.trim().to_ascii_lowercase().as_str(), "on" | "1" | "true" | "yes")
}

/// An `.sbsar` description: `<sbsdescription>`.
pub fn read_description(text: &str) -> Result<SubstanceFile, ImportError> {
    let document = parse_xml(text)?;
    let root = document.root_element();
    if root.tag_name().name() != "sbsdescription" {
        return Err(ImportError::Malformed(format!("its description starts with <{}>, not <sbsdescription>", root.tag_name().name())));
    }
    let mut file = SubstanceFile { kind: FileKind::Sbsar, format_version: attribute(root, "formatversion"), graphs: Vec::new() };
    let Some(graphs) = child(root, "graphs") else {
        return Ok(file);
    };
    for node in children(graphs, "graph") {
        let mut graph = Graph {
            url: attribute(node, "pkgurl"),
            label: attribute(node, "label"),
            category: attribute(node, "category"),
            author: attribute(node, "author"),
            author_url: attribute(node, "authorurl"),
            description: attribute(node, "description"),
            keywords: attribute(node, "keywords"),
            hidden: on(&attribute(node, "hideInLibrary")),
            physical_size: physical_size(&attribute(node, "physicalSize")),
            ..Graph::default()
        };
        // Group identifiers to labels, where the file names its groups.
        let group_labels: BTreeMap<String, String> = child(node, "guigroups")
            .map(|groups| {
                children(groups, "guigroup")
                    .map(|g| (attribute(g, "identifier"), attribute(g, "label")))
                    .filter(|(id, label)| !id.is_empty() && !label.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(inputs) = child(node, "inputs") {
            for element in children(inputs, "input") {
                if let Some(input) = sbsar_input(element, &group_labels) {
                    graph.inputs.push(input);
                }
            }
        }
        if let Some(outputs) = child(node, "outputs") {
            for element in children(outputs, "output") {
                let gui = child(element, "outputgui");
                graph.outputs.push(Output {
                    identifier: attribute(element, "identifier"),
                    uid: attribute(element, "uid"),
                    label: gui.map(|g| attribute(g, "label")).unwrap_or_default(),
                    group: gui.map(|g| attribute(g, "group")).unwrap_or_default(),
                    usages: gui.map(channel_usages).unwrap_or_default(),
                });
            }
        }
        if let Some(presets) = child(node, "sbspresets") {
            for preset in children(presets, "sbspreset") {
                graph.presets.push(Preset {
                    label: attribute(preset, "label"),
                    values: children(preset, "presetinput")
                        .map(|p| (attribute(p, "identifier"), attribute(p, "value")))
                        .filter(|(id, _)| !id.is_empty())
                        .collect(),
                });
            }
        }
        file.graphs.push(graph);
    }
    Ok(file)
}

/// `physicalSize="100,100,2"`.
fn physical_size(text: &str) -> Option<[f32; 3]> {
    let v = Value::parse(text);
    (v.len() >= 2 && v[0] > 0.0 && v[1] > 0.0).then(|| [v[0] as f32, v[1] as f32, v.get(2).copied().unwrap_or(0.0) as f32])
}

/// `<channels><channel names="baseColor"/></channels>`: the usages, each
/// `names` split in case it lists several.
fn channel_usages(node: roxmltree::Node) -> Vec<String> {
    let Some(channels) = child(node, "channels") else { return Vec::new() };
    children(channels, "channel")
        .flat_map(|c| {
            attribute(c, "names")
                .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn sbsar_input(element: roxmltree::Node, group_labels: &BTreeMap<String, String>) -> Option<Input> {
    let identifier = attribute(element, "identifier");
    if identifier.is_empty() {
        return None;
    }
    let value_type = element.attribute("type").and_then(|t| t.trim().parse::<i64>().ok()).and_then(ValueType::from_sbsar)?;
    let mut input = Input::new(&identifier, value_type);
    input.uid = attribute(element, "uid");
    input.default = Value::typed(&attribute(element, "default"), value_type);
    if let Some(gui) = child(element, "inputgui") {
        input.widget = Widget::from_name(&attribute(gui, "widget"));
        input.label = attribute(gui, "label");
        input.description = attribute(gui, "description");
        let group = attribute(gui, "group");
        input.group = group_labels.get(&group).cloned().unwrap_or(group);
        input.visible_if = attribute(gui, "visibleif");
        for widget in gui.children().filter(|c| c.is_element()) {
            let name = widget.tag_name().name().to_ascii_lowercase();
            match name.as_str() {
                "guislider" | "guiangle" | "guicolor" | "guiposition" => {
                    let numbers = |a: &str| widget.attribute(a).map(Value::parse).filter(|v| !v.is_empty());
                    input.min = numbers("min");
                    input.max = numbers("max");
                    input.step = numbers("step").and_then(|v| v.first().copied()).filter(|s| *s > 0.0);
                    input.clamp = on(&attribute(widget, "clamp"));
                    input.labels = (0..4).map(|i| attribute(widget, &format!("label{i}"))).collect();
                    while input.labels.last().is_some_and(String::is_empty) {
                        input.labels.pop();
                    }
                }
                "guibutton" | "guitogglebutton" => {
                    input.labels = vec![attribute(widget, "label0"), attribute(widget, "label1")];
                }
                "guicombobox" | "guienumbuttons" => {
                    input.choices = widget
                        .children()
                        .filter(|c| c.is_element())
                        .filter_map(|item| {
                            let value = item.attribute("value")?.trim().parse::<f64>().ok()?;
                            Some((value.round() as i64, attribute(item, "text")))
                        })
                        .collect();
                }
                "guiimage" => input.usages = channel_usages(widget),
                _ => {}
            }
        }
    }
    if value_type == ValueType::Image {
        input.widget = Widget::Image;
    }
    Some(input)
}

/// An `.sbs`: Designer's source.
pub fn read_sbs(text: &str) -> Result<SubstanceFile, ImportError> {
    let document = parse_xml(text)?;
    let root = document.root_element();
    if root.tag_name().name() != "package" {
        return Err(ImportError::Malformed(format!("an .sbs starts with <package>, and this starts with <{}>", root.tag_name().name())));
    }
    let v = |node: roxmltree::Node, name: &str| child(node, name).map(|c| attribute(c, "v")).unwrap_or_default();
    let mut file = SubstanceFile { kind: FileKind::Sbs, format_version: v(root, "formatVersion"), graphs: Vec::new() };
    // A package's graphs sit in its `<content>`, or in folders' (`<group>`)
    // own `<content>` inside it.
    let graphs = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "graph")
        .filter(|n| n.parent_element().is_some_and(|p| p.tag_name().name() == "content"));
    for node in graphs {
        let identifier = v(node, "identifier");
        if identifier.is_empty() {
            continue;
        }
        let attributes = child(node, "attributes");
        let attr = |name: &str| attributes.map(|a| v(a, name)).unwrap_or_default();
        let thumbnail = attributes.and_then(|a| child(a, "icon")).and_then(|icon| {
            use base64::Engine;
            let data = v(icon, "strdata");
            let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
            base64::engine::general_purpose::STANDARD.decode(cleaned).ok().filter(|b| !b.is_empty())
        });
        let mut graph = Graph {
            url: format!("pkg://{identifier}"),
            label: attr("label"),
            category: attr("category"),
            author: attr("author"),
            author_url: attr("authorURL"),
            description: attr("description"),
            keywords: attr("tags"),
            hidden: on(&attr("hideInLibrary")),
            physical_size: physical_size(&attr("physicalSize")),
            thumbnail,
            ..Graph::default()
        };
        // The cooker exposes the output size and the random seed unless told
        // not to; the size starts as the graph's own, when it states one.
        let mut size = vec![10.0, 10.0];
        if let Some(base) = child(node, "baseParameters") {
            for parameter in children(base, "parameter") {
                if v(parameter, "name") == "outputsize" && v(parameter, "relativeTo") == "0" {
                    if let Some(value) = child(parameter, "paramValue").and_then(|p| p.children().find(|c| c.is_element())) {
                        let parsed = Value::parse(&attribute(value, "v"));
                        if parsed.len() == 2 {
                            size = parsed;
                        }
                    }
                }
            }
        }
        graph.inputs.push(output_size_input(size));
        graph.inputs.push(random_seed_input());
        if let Some(inputs) = child(node, "paraminputs") {
            for element in children(inputs, "paraminput") {
                if let Some(input) = sbs_input(element) {
                    graph.inputs.push(input);
                }
            }
        }
        if let Some(outputs) = child(node, "graphOutputs") {
            for element in children(outputs, "graphoutput") {
                graph.outputs.push(Output {
                    identifier: v(element, "identifier"),
                    uid: v(element, "uid"),
                    label: child(element, "attributes").map(|a| v(a, "label")).unwrap_or_default(),
                    group: v(element, "group"),
                    usages: child(element, "usages")
                        .map(|u| children(u, "usage").map(|usage| v(usage, "name")).filter(|n| !n.is_empty()).collect())
                        .unwrap_or_default(),
                });
            }
        }
        // Presets: `<sbspresets><sbspreset>`, or `<presets><preset>`, each
        // with a label and `<presetinputs>` of identifier and value. Not
        // confirmed against a file Designer wrote.
        for presets in children(node, "sbspresets").chain(children(node, "presets")) {
            for preset in presets.children().filter(|c| c.is_element() && matches!(c.tag_name().name(), "sbspreset" | "preset")) {
                let label = preset.attribute("label").map(str::to_string).unwrap_or_else(|| v(preset, "label"));
                let holder = child(preset, "presetinputs").unwrap_or(preset);
                let values = children(holder, "presetinput")
                    .filter_map(|p| {
                        let id = v(p, "identifier");
                        let value = child(p, "value").and_then(|val| val.children().find(|c| c.is_element())).map(|c| attribute(c, "v"))?;
                        (!id.is_empty()).then_some((id, value))
                    })
                    .collect();
                graph.presets.push(Preset { label, values });
            }
        }
        file.graphs.push(graph);
    }
    Ok(file)
}

/// The output size input every cooked graph has, unless cooked without.
/// Its values are powers of two: 10 is 1024 pixels.
pub fn output_size_input(default: Vec<f64>) -> Input {
    let mut input = Input::new("$outputsize", ValueType::Integer(2));
    input.label = "Output size".into();
    input.widget = Widget::Slider;
    input.default = Value::Numbers(default);
    input.min = Some(vec![4.0, 4.0]);
    input.max = Some(vec![13.0, 13.0]);
    input.step = Some(1.0);
    input.clamp = true;
    input
}

/// The random seed every cooked graph has, unless cooked without.
pub fn random_seed_input() -> Input {
    let mut input = Input::new("$randomseed", ValueType::Integer(1));
    input.label = "Random seed".into();
    input.widget = Widget::Slider;
    input.default = Value::Numbers(vec![0.0]);
    input.min = Some(vec![0.0]);
    input.max = Some(vec![10000.0]);
    input.step = Some(1.0);
    input
}

fn sbs_input(element: roxmltree::Node) -> Option<Input> {
    let v = |node: roxmltree::Node, name: &str| child(node, name).map(|c| attribute(c, "v")).unwrap_or_default();
    let identifier = v(element, "identifier");
    if identifier.is_empty() {
        return None;
    }
    let constant = child(element, "defaultValue").and_then(|d| d.children().find(|c| c.is_element()));
    let code = v(element, "type").trim().parse::<i64>().ok();
    let value_type = code
        .and_then(ValueType::from_sbs)
        .or_else(|| constant.and_then(|c| ValueType::from_constant_tag(c.tag_name().name())))?;
    let mut input = Input::new(&identifier, value_type);
    input.uid = v(element, "uid");
    if let Some(attributes) = child(element, "attributes") {
        input.label = v(attributes, "label");
        input.description = v(attributes, "description");
    }
    input.group = v(element, "group").trim().to_string();
    input.visible_if = v(element, "visibleIf");
    if let Some(constant) = constant {
        input.default = Value::typed(&attribute(constant, "v"), value_type);
    }
    if let Some(widget) = child(element, "defaultWidget") {
        let name = v(widget, "name");
        input.widget = Widget::from_name(&name);
        let options: BTreeMap<String, String> = child(widget, "options")
            .map(|o| children(o, "option").map(|opt| (v(opt, "name"), v(opt, "value"))).collect())
            .unwrap_or_default();
        let numbers = |key: &str| options.get(key).map(|t| Value::parse(t)).filter(|v| !v.is_empty());
        input.min = numbers("min");
        input.max = numbers("max");
        input.step = numbers("step").and_then(|v| v.first().copied()).filter(|s| *s > 0.0);
        input.clamp = options.get("clamp").is_some_and(|c| on(c));
        input.labels = (0..4).filter_map(|i| options.get(&format!("label{i}")).cloned()).collect();
        // A drop-down's items: "value;label;value;label", with what looks
        // like a leading count or flag when the number of fields is odd —
        // the spelling the Automation Toolkit's cookbook uses. Not
        // confirmed against a file Designer wrote.
        if let Some(list) = options.get("parameters") {
            let mut fields: Vec<&str> = list.split(';').collect();
            if fields.len() % 2 == 1 {
                fields.remove(0);
            }
            input.choices = fields
                .chunks(2)
                .filter_map(|pair| Some((pair[0].trim().parse::<f64>().ok()?.round() as i64, pair.get(1)?.trim().to_string())))
                .collect();
            if input.widget == Widget::Other && !input.choices.is_empty() {
                input.widget = Widget::ComboBox;
            }
        }
        // A boolean (`type` 4) is a switch however its widget is named.
        if code == Some(4) {
            input.widget = Widget::Toggle;
        }
    }
    if value_type == ValueType::Image {
        input.widget = Widget::Image;
        input.usages = child(element, "usages")
            .map(|u| children(u, "usage").map(|usage| v(usage, "name")).filter(|n| !n.is_empty()).collect())
            .unwrap_or_default();
    }
    Some(input)
}

// --- a material from it ------------------------------------------------------

/// What each of a graph's outputs becomes: the texture set the material is
/// wired from, with each map at `file_of(output)`, and the outputs not used
/// and why. One output per role: the first wins.
///
/// The set is what a texture pack makes too, so a Substance material and a
/// pack go through one code path ([`crate::texture_set::material`]): the
/// same roles, the same wiring, the same mapping.
pub fn texture_set(graph: &Graph, directx: bool, file_of: impl Fn(&Output) -> PathBuf) -> (TextureSet, Vec<(String, String)>) {
    let mut set = TextureSet { name: graph.name(), ..TextureSet::default() };
    let mut unused = Vec::new();
    for output in &graph.outputs {
        match output.slot(directx) {
            Ok(slot) if set.map(slot).is_some() => unused.push((output.identifier.clone(), "another output plays the same part".to_string())),
            Ok(slot) => set.maps.push(Map { path: file_of(output), slot, resolution: 0 }),
            Err(reason) => unused.push((output.identifier.clone(), reason)),
        }
    }
    set.maps.sort_by_key(|m| m.slot);
    (set, unused)
}

/// The file an output's map is rendered to in `directory`, and the one its
/// map is read from: a DirectX normal map is turned into an OpenGL one
/// beside it, as a texture pack's is.
pub fn rendered_file(directory: &Path, output: &str) -> PathBuf {
    directory.join(format!("{}.png", safe_name(output)))
}

/// [`rendered_file`], converted to OpenGL's convention.
pub fn converted_file(directory: &Path, output: &str) -> PathBuf {
    directory.join(format!("{}_opengl.png", safe_name(output)))
}

/// An output identifier as a file name: only the characters that are safe
/// in one everywhere.
pub fn safe_name(identifier: &str) -> String {
    let name: String = identifier
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if name.is_empty() { "output".into() } else { name }
}

/// FNV-1a over `bytes`, as the document hashes CAD sources: stable across
/// builds and platforms.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests;

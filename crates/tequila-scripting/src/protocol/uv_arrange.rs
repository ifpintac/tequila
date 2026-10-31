//! The UV editor's arranging tools, as a script asks for them: keys of the
//! same `uv` object as the island tools' (`{"op":"ui","uv":{...}}`), read
//! here apart so they are found together. Absent keys are left alone.

use super::{opt_bool, opt_number, opt_string, optional, point, ProtocolError};
use tequila_format::json::Value;

/// What a script asks of the arranging tools, the Select menu and what
/// the editor shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UvArrange {
    /// The 2D cursor: put at a point `[u, v]`, on the selection's middle
    /// (`"selection"`), on the layout's (`"centre"`), or taken away
    /// (`false`).
    pub cursor: Option<UvCursor>,
    pub show_cursor: Option<bool>,
    /// What turns and scales are about: `"cursor"`, or `"frame"` - the
    /// Transform panel's chosen point and the selection's middle.
    pub pivot: Option<String>,
    /// What Align lines up against: `selection`, `island`, `layout` or
    /// `cursor`.
    pub align_to: Option<String>,
    /// Spread `across`, `down`, or `along` the picked path.
    pub distribute: Option<String>,
    /// How a spread spaces things: `gaps` or `centres` for islands, `even`
    /// or `part` along a path.
    pub spread: Option<String>,
    /// Edges nearly square to the layout made exactly so, within this many
    /// degrees (`true` for the panel's).
    pub square_edges: Option<Option<f32>>,
    pub upright: bool,
    pub sort: Option<Value>,
    pub match_size: Option<Value>,
    pub fit: Option<Value>,
    pub stack: Option<Value>,
    pub copy_shape: bool,
    pub paste_shape: Option<Value>,
    /// Two selected islands swapped, more shuffled.
    pub shuffle: bool,
    pub as_on_part: Option<Value>,
    pub density: Option<Value>,
    /// The Select menu: islands like the selected ones, within a
    /// tolerance in per cent (`true` for the menu's).
    pub select_similar: Option<Option<f32>>,
    pub select_borders: bool,
    /// Edges square to the layout: `across`, `down` or `either`.
    pub select_square: Option<String>,
    pub select_no_area: bool,
    pub select_flipped: bool,
    /// The texture behind the layout, made lighter or darker by so many
    /// stops.
    pub exposure: Option<f32>,
    /// A checker on the part in the viewport: `true`, `false`, or
    /// `{"squares": 8, "pattern": "arrows"}`.
    pub checker: Option<Value>,
    /// The padding overlay: `true`, `false`, or `{"texture": 2048, "px": 8,
    /// "suggest": true, "use_for_pack": true}`.
    pub padding: Option<Value>,
    /// The Select menu, open.
    pub select_menu: Option<bool>,
    /// Sort's options, open.
    pub sort_options: Option<bool>,
}

/// Where a script puts the 2D cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UvCursor {
    At([f32; 2]),
    Selection,
    Centre,
    Clear,
}

/// A key that is `true`, or an object of options: the options, `{}` for
/// `true`; none for `false` or absent.
fn options(value: &Value, key: &str, example: &str) -> Result<Option<Value>, ProtocolError> {
    match value.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(Value::Object(Default::default()))),
        Some(object @ Value::Object(_)) => Ok(Some(object.clone())),
        Some(_) => Err(ProtocolError(format!("`{key}` is true, or its options: {example}"))),
    }
}

/// `true` for the panel's own, or a number.
fn on_or_number(value: &Value, key: &str) -> Result<Option<Option<f32>>, ProtocolError> {
    match value.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => Ok(None),
        Some(Value::Bool(true)) => Ok(Some(None)),
        Some(_) => Ok(Some(Some(opt_number(value, key)?.ok_or_else(|| ProtocolError(format!("`{key}` is true or a number")))?))),
    }
}

pub(super) fn uv_arrange(value: &Value) -> Result<UvArrange, ProtocolError> {
    let cursor = match value.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(false)) => Some(UvCursor::Clear),
        Some(Value::String(s)) if s == "selection" => Some(UvCursor::Selection),
        Some(Value::String(s)) if s == "centre" || s == "center" => Some(UvCursor::Centre),
        Some(v) => Some(UvCursor::At(point(v).ok_or_else(|| ProtocolError("`cursor` is a point of the layout, [u, v], \"selection\", \"centre\" or false".into()))?)),
    };
    Ok(UvArrange {
        cursor,
        show_cursor: opt_bool(value, "show_cursor")?,
        pivot: opt_string(value, "pivot")?,
        align_to: opt_string(value, "align_to")?,
        distribute: opt_string(value, "distribute")?,
        spread: opt_string(value, "spread")?,
        square_edges: on_or_number(value, "square_edges")?,
        upright: opt_bool(value, "upright")?.unwrap_or(false),
        sort: options(value, "sort", "{\"by\": \"x\", \"flow\": \"row\", \"wrap\": \"width\"}")?,
        match_size: options(value, "match_size", "{\"to\": \"largest\", \"dimension\": \"width\"}")?,
        fit: options(value, "fit", "{\"each\": true}")?,
        stack: options(value, "stack", "{\"mirrored\": true, \"offset\": [0.05, 0]}")?,
        copy_shape: opt_bool(value, "copy_shape")?.unwrap_or(false),
        paste_shape: options(value, "paste_shape", "{\"in_place\": true}")?,
        shuffle: opt_bool(value, "shuffle")?.unwrap_or(false),
        as_on_part: options(value, "as_on_part", "{\"view\": \"front\"}")?,
        density: optional(value, "density", "an object, such as {\"texture\": 2048, \"get\": true} or {\"apply\": 10.24}", |v| matches!(v, Value::Object(_)).then(|| v.clone()))?,
        select_similar: on_or_number(value, "select_similar")?,
        select_borders: opt_bool(value, "select_borders")?.unwrap_or(false),
        select_square: opt_string(value, "select_square")?,
        select_no_area: opt_bool(value, "select_no_area")?.unwrap_or(false),
        select_flipped: opt_bool(value, "select_flipped")?.unwrap_or(false),
        exposure: opt_number(value, "exposure")?,
        checker: match value.get("checker") {
            None | Some(Value::Null) => None,
            Some(v @ (Value::Bool(_) | Value::Object(_))) => Some(v.clone()),
            Some(_) => return Err(ProtocolError("`checker` is true, false, or {\"squares\": 8, \"pattern\": \"arrows\"}".into())),
        },
        padding: match value.get("padding") {
            None | Some(Value::Null) => None,
            Some(v @ (Value::Bool(_) | Value::Object(_))) => Some(v.clone()),
            Some(_) => return Err(ProtocolError("`padding` is true, false, or {\"texture\": 2048, \"px\": 8}".into())),
        },
        select_menu: opt_bool(value, "select_menu")?,
        sort_options: opt_bool(value, "sort_options")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arranging_keys_read_as_written() {
        let value = tequila_format::json::parse(br#"{"cursor":[0.25,0.5],"pivot":"cursor","align_to":"cursor","sort":{"by":"x"},"fit":true,"square_edges":10,"select_similar":true,"checker":{"squares":8}}"#).unwrap();
        let read = uv_arrange(&value).unwrap();
        assert_eq!(read.cursor, Some(UvCursor::At([0.25, 0.5])));
        assert_eq!(read.pivot.as_deref(), Some("cursor"));
        assert!(read.sort.is_some() && read.fit.is_some() && read.stack.is_none());
        assert_eq!(read.square_edges, Some(Some(10.0)));
        assert_eq!(read.select_similar, Some(None));
        let off = tequila_format::json::parse(br#"{"cursor":false,"fit":false}"#).unwrap();
        let read = uv_arrange(&off).unwrap();
        assert_eq!(read.cursor, Some(UvCursor::Clear));
        assert!(read.fit.is_none());
        assert!(uv_arrange(&tequila_format::json::parse(br#"{"cursor":"somewhere"}"#).unwrap()).is_err());
        assert!(uv_arrange(&tequila_format::json::parse(br#"{"sort":3}"#).unwrap()).is_err());
    }
}

//! A conversation, in a shape both wire formats can be written from.

use tequila_format::json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// A request from the model to use one of its tools.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The provider's id for the call, echoed back with its result.
    pub id: String,
    pub name: String,
    /// The arguments, or why they could not be read.
    pub arguments: Result<Value, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    /// A PNG, base64-encoded.
    Image(String),
    Call(Call),
    Result {
        id: String,
        name: String,
        text: String,
        /// A PNG, base64-encoded: a screenshot the tool took.
        image: Option<String>,
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message { role: Role::User, parts: vec![Part::Text(text.into())] }
    }
}

/// How many images a request carries. A screenshot is a thousand or so
/// tokens and the conversation is resent in full every step, so after a few
/// checks the older pictures are costing more than they tell; the newest is
/// what the model reasons about.
pub const IMAGES_KEPT: usize = 2;

/// The conversation with all but the newest `keep` images replaced by a note.
pub fn with_recent_images(messages: &[Message], keep: usize) -> Vec<Message> {
    let mut seen = 0;
    let mut out: Vec<Message> = messages.to_vec();
    for message in out.iter_mut().rev() {
        for part in message.parts.iter_mut().rev() {
            match part {
                Part::Image(_) => {
                    seen += 1;
                    if seen > keep {
                        *part = Part::Text("[an earlier screenshot, left out to save space]".into());
                    }
                }
                Part::Result { image: image @ Some(_), text, .. } => {
                    seen += 1;
                    if seen > keep {
                        *image = None;
                        text.push_str(" [its screenshot has been left out to save space]");
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(n: u32) -> Part {
        Part::Result { id: n.to_string(), name: "tequila_screenshot".into(), text: format!("shot {n}"), image: Some("PNG".into()), is_error: false }
    }

    #[test]
    fn only_the_newest_images_are_sent() {
        let messages = vec![
            Message { role: Role::User, parts: vec![shot(1)] },
            Message { role: Role::User, parts: vec![shot(2)] },
            Message { role: Role::User, parts: vec![shot(3)] },
        ];
        let pruned = with_recent_images(&messages, 2);
        let images: Vec<bool> = pruned
            .iter()
            .map(|m| matches!(&m.parts[0], Part::Result { image: Some(_), .. }))
            .collect();
        assert_eq!(images, [false, true, true]);
        assert!(matches!(&pruned[0].parts[0], Part::Result { text, .. } if text.contains("left out")));
        // The original is untouched: the app still shows every picture.
        assert!(matches!(&messages[0].parts[0], Part::Result { image: Some(_), .. }));
    }
}

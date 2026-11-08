//! Server-sent events: how both wire formats stream a reply.
//!
//! A stream is lines. `data:` lines carry the event, `event:` names it, a
//! line starting with `:` is a comment (a keep-alive, usually), and a blank
//! line ends the event. Bytes arrive in whatever pieces the network makes,
//! so a line — or a character — can be split between two of them.

/// One event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// What `event:` named it; empty when nothing did.
    pub name: String,
    pub data: String,
}

#[derive(Default)]
pub struct Parser {
    line: Vec<u8>,
    name: String,
    data: String,
    has_data: bool,
}

impl Parser {
    pub fn new() -> Parser {
        Parser::default()
    }

    /// Read some more of the stream; the events it completed go on `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for &byte in bytes {
            if byte == b'\n' {
                let mut line = std::mem::take(&mut self.line);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.line_done(&String::from_utf8_lossy(&line), out);
            } else {
                self.line.push(byte);
            }
        }
    }

    /// The stream has ended: an event it did not close with a blank line
    /// still counts.
    pub fn finish(&mut self, out: &mut Vec<Event>) {
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.line_done(String::from_utf8_lossy(&line).trim_end_matches('\r'), out);
        }
        self.line_done("", out);
    }

    fn line_done(&mut self, line: &str, out: &mut Vec<Event>) {
        if line.is_empty() {
            if self.has_data {
                out.push(Event { name: std::mem::take(&mut self.name), data: std::mem::take(&mut self.data) });
            }
            self.name.clear();
            self.has_data = false;
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.name = value.to_string(),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(pieces: &[&[u8]]) -> Vec<Event> {
        let mut parser = Parser::new();
        let mut out = Vec::new();
        for piece in pieces {
            parser.feed(piece, &mut out);
        }
        parser.finish(&mut out);
        out
    }

    #[test]
    fn events_are_put_together_across_pieces() {
        let out = events(&[b"event: content_block_delta\r\nda", b"ta: {\"a\":", b"1}\r\n\r\n: ping\n\ndata: [DONE]\n\n"]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "content_block_delta");
        assert_eq!(out[0].data, "{\"a\":1}");
        assert_eq!(out[1], Event { name: String::new(), data: "[DONE]".into() });
    }

    /// A character of more than one byte split between two pieces survives.
    #[test]
    fn a_character_split_between_pieces_survives() {
        let text = "data: caf\u{e9} \u{2014} ok\n\n".as_bytes();
        let (a, b) = text.split_at(10);
        assert_eq!(events(&[a, b])[0].data, "caf\u{e9} \u{2014} ok");
    }

    #[test]
    fn several_data_lines_are_one_event_and_an_unclosed_one_still_counts() {
        let out = events(&[b"data: one\ndata: two\n\ndata: last"]);
        assert_eq!(out[0].data, "one\ntwo");
        assert_eq!(out[1].data, "last");
    }
}

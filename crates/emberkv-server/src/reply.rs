//! Replies to clients, encoded in RESP2.

/// One reply. Every command produces exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A short fixed status such as `OK` or `PONG`: `+OK\r\n`.
    Simple(&'static str),
    /// An error, starting with a code such as `ERR`: `-ERR message\r\n`.
    Error(String),
    /// A number, such as how many keys were deleted: `:1\r\n`.
    Integer(i64),
    /// Arbitrary bytes, such as a stored value: `$5\r\nvalue\r\n`.
    Bulk(Vec<u8>),
    /// "No value": `$-1\r\n`.
    Null,
}

impl Reply {
    /// Appends this reply's bytes to `out`.
    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Reply::Simple(text) => {
                out.push(b'+');
                out.extend_from_slice(text.as_bytes());
            }
            Reply::Error(text) => {
                out.push(b'-');
                // Error text can contain what a client sent, such as an unknown command name. A line
                // break in it would end the reply early and corrupt the stream, so replace them.
                out.extend(text.bytes().map(|byte| match byte {
                    b'\r' | b'\n' => b' ',
                    other => other,
                }));
            }
            Reply::Integer(number) => out.extend_from_slice(format!(":{number}").as_bytes()),
            Reply::Bulk(bytes) => {
                out.extend_from_slice(format!("${}\r\n", bytes.len()).as_bytes());
                out.extend_from_slice(bytes);
            }
            Reply::Null => out.extend_from_slice(b"$-1"),
        }
        out.extend_from_slice(b"\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(reply: &Reply) -> Vec<u8> {
        let mut out = Vec::new();
        reply.encode(&mut out);
        out
    }

    #[test]
    fn each_kind_of_reply_has_its_resp2_form() {
        assert_eq!(encoded(&Reply::Simple("OK")), b"+OK\r\n");
        assert_eq!(encoded(&Reply::Error("ERR no".into())), b"-ERR no\r\n");
        assert_eq!(encoded(&Reply::Integer(-3)), b":-3\r\n");
        assert_eq!(
            encoded(&Reply::Bulk(b"a\r\nb".to_vec())),
            b"$4\r\na\r\nb\r\n"
        );
        assert_eq!(encoded(&Reply::Bulk(Vec::new())), b"$0\r\n\r\n");
        assert_eq!(encoded(&Reply::Null), b"$-1\r\n");
    }

    #[test]
    fn line_breaks_in_error_text_cannot_end_the_reply_early() {
        let sneaky = Reply::Error("ERR unknown command 'x\r\n+OK'".into());
        assert_eq!(encoded(&sneaky), b"-ERR unknown command 'x  +OK'\r\n");
    }
}

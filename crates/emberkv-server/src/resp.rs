//! Parsing client requests in RESP2, the Redis protocol, so `redis-cli` can talk to EmberKV.
//!
//! A request is either an array of bulk strings, as real clients send:
//!
//! ```text
//! *2\r\n$3\r\nGET\r\n$3\r\nkey\r\n
//! ```
//!
//! or an **inline** command, one line of words separated by spaces, as typed into `nc` or `telnet`:
//!
//! ```text
//! GET key\r\n
//! ```
//!
//! Bytes arrive from the network in pieces, so [`parse`] works on whatever is buffered so far: it
//! returns a complete command and how many bytes it used, says more bytes are needed, or reports an
//! error. Every length a client sends is checked against [`ProtocolLimits`] before anything is
//! allocated for it.

use thiserror::Error;

const CRLF: &[u8] = b"\r\n";

/// The longest a line holding a count or a length may be: `*` or `$` plus up to 20 digits.
const MAX_NUMBER_LINE: usize = 21;

/// Size limits for one request. Each field is a maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolLimits {
    /// Most arguments in one command, the command name included.
    pub args: usize,
    /// Longest single argument, in bytes.
    pub arg_len: usize,
    /// Longest inline command line, in bytes.
    pub inline_len: usize,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        ProtocolLimits {
            args: 64,
            arg_len: 1024 * 1024 + 1024,
            inline_len: 64 * 1024,
        }
    }
}

/// The result of parsing the bytes buffered so far.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Not enough bytes yet for a whole command. Read more and try again.
    Incomplete,
    /// A whole command, which used the first `consumed` bytes of the buffer.
    Command { args: Vec<Vec<u8>>, consumed: usize },
}

/// Why a request was rejected. The connection should reply with an error and close, because the
/// bytes that follow can no longer be trusted to line up with command boundaries.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// A count or length was not a plain non-negative number.
    #[error("invalid number in request")]
    BadNumber,
    /// An array element did not start with `$`.
    #[error("expected a bulk string")]
    ExpectedBulkString,
    /// A bulk string was not followed by `\r\n`.
    #[error("missing line ending after a bulk string")]
    MissingLineEnd,
    /// More arguments than [`ProtocolLimits::args`].
    #[error("too many arguments")]
    TooManyArgs,
    /// An argument longer than [`ProtocolLimits::arg_len`].
    #[error("argument too long")]
    ArgTooLong,
    /// A line longer than allowed with no line ending yet.
    #[error("line too long")]
    LineTooLong,
}

/// Parses one command from the start of `buf`.
///
/// # Errors
///
/// A [`ProtocolError`] if the bytes are not valid RESP2 or exceed `limits`.
pub fn parse(buf: &[u8], limits: &ProtocolLimits) -> Result<Parsed, ProtocolError> {
    match buf.first() {
        None => Ok(Parsed::Incomplete),
        Some(b'*') => parse_array(buf, limits),
        Some(_) => parse_inline(buf, limits),
    }
}

fn parse_array(buf: &[u8], limits: &ProtocolLimits) -> Result<Parsed, ProtocolError> {
    let Some(line_end) = find_crlf(buf, 1, MAX_NUMBER_LINE)? else {
        return Ok(Parsed::Incomplete);
    };
    let count = parse_number(&buf[1..line_end])?;
    if count > limits.args {
        return Err(ProtocolError::TooManyArgs);
    }

    let mut args = Vec::with_capacity(count); // safe: count is already within the limit
    let mut pos = line_end + 2;
    for _ in 0..count {
        match buf.get(pos) {
            None => return Ok(Parsed::Incomplete),
            Some(b'$') => {}
            Some(_) => return Err(ProtocolError::ExpectedBulkString),
        }
        let Some(line_end) = find_crlf(buf, pos + 1, MAX_NUMBER_LINE)? else {
            return Ok(Parsed::Incomplete);
        };
        let len = parse_number(&buf[pos + 1..line_end])?;
        if len > limits.arg_len {
            return Err(ProtocolError::ArgTooLong);
        }
        let start = line_end + 2;
        let end = start + len;
        if buf.len() < end + 2 {
            return Ok(Parsed::Incomplete);
        }
        if &buf[end..end + 2] != CRLF {
            return Err(ProtocolError::MissingLineEnd);
        }
        args.push(buf[start..end].to_vec());
        pos = end + 2;
    }
    Ok(Parsed::Command {
        args,
        consumed: pos,
    })
}

/// An inline command: one line, words separated by spaces. A bare `\n` also ends the line, so `nc`
/// works as well as `telnet`.
fn parse_inline(buf: &[u8], limits: &ProtocolLimits) -> Result<Parsed, ProtocolError> {
    let Some(newline) = buf.iter().position(|&byte| byte == b'\n') else {
        if buf.len() > limits.inline_len {
            return Err(ProtocolError::LineTooLong);
        }
        return Ok(Parsed::Incomplete);
    };
    if newline > limits.inline_len {
        return Err(ProtocolError::LineTooLong);
    }
    let line = buf[..newline]
        .strip_suffix(b"\r")
        .unwrap_or(&buf[..newline]);
    let args: Vec<Vec<u8>> = line
        .split(u8::is_ascii_whitespace)
        .filter(|word| !word.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    if args.len() > limits.args {
        return Err(ProtocolError::TooManyArgs);
    }
    Ok(Parsed::Command {
        args,
        consumed: newline + 1,
    })
}

/// The position of the next `\r\n` at or after `start`, looking at most `max` bytes ahead.
fn find_crlf(buf: &[u8], start: usize, max: usize) -> Result<Option<usize>, ProtocolError> {
    let window = buf.get(start..).unwrap_or_default();
    let limit = window.len().min(max + 2);
    match window[..limit].windows(2).position(|pair| pair == CRLF) {
        Some(found) => Ok(Some(start + found)),
        None if window.len() >= max + 2 => Err(ProtocolError::LineTooLong),
        None => Ok(None),
    }
}

/// A non-negative decimal number with no sign, spaces or leading `+`.
fn parse_number(digits: &[u8]) -> Result<usize, ProtocolError> {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(ProtocolError::BadNumber);
    }
    std::str::from_utf8(digits)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(ProtocolError::BadNumber)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn limits() -> ProtocolLimits {
        ProtocolLimits::default()
    }

    fn command(args: &[&[u8]], consumed: usize) -> Parsed {
        Parsed::Command {
            args: args.iter().map(|arg| arg.to_vec()).collect(),
            consumed,
        }
    }

    const GET: &[u8] = b"*2\r\n$3\r\nGET\r\n$3\r\nkey\r\n";

    #[test]
    fn a_complete_array_command() {
        assert_eq!(
            parse(GET, &limits()),
            Ok(command(&[b"GET", b"key"], GET.len()))
        );
    }

    #[test]
    fn every_partial_prefix_asks_for_more_bytes() {
        for cut in 0..GET.len() {
            assert_eq!(
                parse(&GET[..cut], &limits()),
                Ok(Parsed::Incomplete),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn pipelined_commands_are_parsed_one_at_a_time() {
        let mut two = GET.to_vec();
        two.extend_from_slice(b"*1\r\n$4\r\nPING\r\n");
        assert_eq!(
            parse(&two, &limits()),
            Ok(command(&[b"GET", b"key"], GET.len()))
        );
        assert_eq!(
            parse(&two[GET.len()..], &limits()),
            Ok(command(&[b"PING"], 14))
        );
    }

    #[test]
    fn binary_values_and_empty_strings_are_kept_exactly() {
        let request = b"*3\r\n$3\r\nSET\r\n$0\r\n\r\n$4\r\n\r\n\x00\xff\r\n";
        assert_eq!(
            parse(request, &limits()),
            Ok(command(&[b"SET", b"", b"\r\n\x00\xff"], request.len()))
        );
    }

    #[test]
    fn inline_commands_from_telnet_and_nc() {
        assert_eq!(
            parse(b"SET  key value\r\n", &limits()),
            Ok(command(&[b"SET", b"key", b"value"], 16))
        );
        assert_eq!(parse(b"PING\n", &limits()), Ok(command(&[b"PING"], 5)));
        assert_eq!(parse(b"PING", &limits()), Ok(Parsed::Incomplete));
        assert_eq!(
            parse(b"\r\n", &limits()),
            Ok(command(&[], 2)),
            "an empty line is no command"
        );
    }

    #[test]
    fn malformed_requests_are_errors() {
        let cases: [(&[u8], ProtocolError); 5] = [
            (b"*x\r\n", ProtocolError::BadNumber),
            (b"*-1\r\n", ProtocolError::BadNumber),
            (b"*1\r\n:3\r\n", ProtocolError::ExpectedBulkString),
            (b"*1\r\n$3\r\nGETxx", ProtocolError::MissingLineEnd),
            (b"*1\r\n$+3\r\n", ProtocolError::BadNumber),
        ];
        for (request, expected) in cases {
            assert_eq!(parse(request, &limits()), Err(expected), "{request:?}");
        }
    }

    #[test]
    fn oversized_requests_are_refused_before_any_allocation() {
        let small = ProtocolLimits {
            args: 2,
            arg_len: 4,
            inline_len: 8,
        };
        assert_eq!(parse(b"*3\r\n", &small), Err(ProtocolError::TooManyArgs));
        assert_eq!(
            parse(b"*1\r\n$5\r\n", &small),
            Err(ProtocolError::ArgTooLong)
        );
        assert_eq!(
            parse(b"*1\r\n$99999999999999999999999\r\n", &small),
            Err(ProtocolError::LineTooLong)
        );
        assert_eq!(
            parse(b"aaaaaaaaaa", &small),
            Err(ProtocolError::LineTooLong)
        );
        assert_eq!(
            parse(b"*0000000000000000000000000", &small),
            Err(ProtocolError::LineTooLong)
        );
    }

    proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse(&bytes, &limits());
        }

        #[test]
        fn any_command_round_trips_through_the_array_form(
            args in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 1..8)
        ) {
            let mut request = format!("*{}\r\n", args.len()).into_bytes();
            for arg in &args {
                request.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
                request.extend_from_slice(arg);
                request.extend_from_slice(CRLF);
            }
            let consumed = request.len();
            prop_assert_eq!(parse(&request, &limits()), Ok(Parsed::Command { args, consumed }));
        }
    }
}

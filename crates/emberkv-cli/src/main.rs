//! The EmberKV command-line client.
//!
//! ```text
//! emberkv-cli [--port PORT] COMMAND [ARGUMENTS...]
//! emberkv-cli set greeting hello
//! emberkv-cli get greeting
//! ```
//!
//! It sends one command as a RESP2 array, prints the reply the way `redis-cli` does, and exits with
//! status 1 if the reply is an error. Plain blocking code: one request needs no async runtime.

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

/// Longest value the client accepts in a reply, so a broken server cannot make it allocate gigabytes.
const MAX_REPLY_LEN: usize = 64 * 1024 * 1024;

fn usage() -> &'static str {
    "usage: emberkv-cli [--port PORT] COMMAND [ARGUMENTS...]\n  e.g. emberkv-cli set name Henry"
}

/// One reply from the server.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Simple(String),
    Error(String),
    Integer(i64),
    Bulk(Vec<u8>),
    Null,
}

/// Encodes `args` as a RESP2 array of bulk strings, the form every real client sends.
fn encode_request(args: &[String]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        out.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        out.extend_from_slice(arg.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Reads one reply.
fn read_reply(reader: &mut impl BufRead) -> io::Result<Reply> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "server closed the connection",
        ));
    }
    let line = line.trim_end_matches(['\r', '\n']);
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected reply: {line}"),
        )
    };
    let (kind, rest) = line.split_at_checked(1).ok_or_else(invalid)?;
    match kind {
        "+" => Ok(Reply::Simple(rest.to_string())),
        "-" => Ok(Reply::Error(rest.to_string())),
        ":" => rest.parse().map(Reply::Integer).map_err(|_| invalid()),
        "$" if rest == "-1" => Ok(Reply::Null),
        "$" => {
            let len: usize = rest.parse().map_err(|_| invalid())?;
            if len > MAX_REPLY_LEN {
                return Err(invalid());
            }
            let mut value = vec![0; len + 2]; // the value and its \r\n
            reader.read_exact(&mut value)?;
            value.truncate(len);
            Ok(Reply::Bulk(value))
        }
        _ => Err(invalid()),
    }
}

/// How `redis-cli` shows each kind of reply.
fn show(reply: &Reply) -> String {
    match reply {
        Reply::Simple(text) => text.clone(),
        Reply::Error(text) => format!("(error) {text}"),
        Reply::Integer(number) => format!("(integer) {number}"),
        Reply::Bulk(bytes) => format!("\"{}\"", String::from_utf8_lossy(bytes)),
        Reply::Null => "(nil)".to_string(),
    }
}

/// Splits `--port PORT` off the front of the arguments.
fn parse_arguments(mut args: Vec<String>) -> Result<(u16, Vec<String>), String> {
    let mut port = 7379;
    if args.first().map(String::as_str) == Some("--port") {
        let value = args.get(1).ok_or("--port needs a value")?;
        port = value
            .parse()
            .map_err(|_| format!("invalid port: {value}"))?;
        args.drain(..2);
    }
    if args.is_empty() {
        return Err("no command given".to_string());
    }
    Ok((port, args))
}

fn run(port: u16, command: &[String]) -> io::Result<Reply> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(&encode_request(command))?;
    read_reply(&mut BufReader::new(stream))
}

fn main() -> ExitCode {
    let (port, command) = match parse_arguments(std::env::args().skip(1).collect()) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("emberkv-cli: {message}\n{}", usage());
            return ExitCode::FAILURE;
        }
    };
    match run(port, &command) {
        Ok(reply) => {
            println!("{}", show(&reply));
            if matches!(reply, Reply::Error(_)) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("emberkv-cli: cannot talk to 127.0.0.1:{port}: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    fn reply(bytes: &[u8]) -> io::Result<Reply> {
        read_reply(&mut Cursor::new(bytes))
    }

    #[test]
    fn usage_names_the_program() {
        assert!(usage().starts_with("usage: emberkv-cli"));
    }

    #[test]
    fn requests_are_resp2_arrays() {
        assert_eq!(
            encode_request(&strings(&["SET", "k", "hello"])),
            b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$5\r\nhello\r\n"
        );
    }

    #[test]
    fn every_kind_of_reply_is_read_and_shown_like_redis_cli() {
        assert_eq!(show(&reply(b"+OK\r\n").unwrap()), "OK");
        assert_eq!(show(&reply(b"-ERR no\r\n").unwrap()), "(error) ERR no");
        assert_eq!(show(&reply(b":2\r\n").unwrap()), "(integer) 2");
        assert_eq!(show(&reply(b"$5\r\nhello\r\n").unwrap()), "\"hello\"");
        assert_eq!(show(&reply(b"$-1\r\n").unwrap()), "(nil)");
    }

    #[test]
    fn broken_replies_are_errors_not_crashes() {
        for bytes in [
            &b""[..],
            b"?\r\n",
            b":x\r\n",
            b"$99999999999\r\n",
            b"$5\r\nhi",
        ] {
            assert!(reply(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn arguments() {
        assert_eq!(
            parse_arguments(strings(&["get", "k"])),
            Ok((7379, strings(&["get", "k"])))
        );
        assert_eq!(
            parse_arguments(strings(&["--port", "8000", "ping"])),
            Ok((8000, strings(&["ping"])))
        );
        assert!(parse_arguments(strings(&[])).is_err());
        assert!(parse_arguments(strings(&["--port", "x", "ping"])).is_err());
    }
}

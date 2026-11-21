//! The `emberkv` program. The server itself lives in the `emberkv_server` library (`lib.rs`).
//!
//! ```text
//! emberkv [--dir PATH] [--port PORT]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use emberkv_server::resp::ProtocolLimits;
use emberkv_server::server::serve;
use emberkv_storage::engine::Engine;
use emberkv_storage::log::LogConfig;
use tokio::net::TcpListener;

/// Settings from the command line.
#[derive(Debug, PartialEq, Eq)]
struct Options {
    dir: PathBuf,
    port: u16,
}

/// Reads `--dir` and `--port`; anything else is an error.
fn parse_options(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        dir: PathBuf::from("data"),
        port: 7379,
    };
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--dir" => options.dir = PathBuf::from(value),
            "--port" => {
                options.port = value
                    .parse()
                    .map_err(|_| format!("invalid port: {value}"))?;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(options)
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[tokio::main]
async fn main() -> ExitCode {
    let options = match parse_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("emberkv: {message}\nusage: emberkv [--dir PATH] [--port PORT]");
            return ExitCode::FAILURE;
        }
    };

    let (engine, report) = match Engine::open(&options.dir, LogConfig::default()) {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("emberkv: cannot open {}: {error}", options.dir.display());
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "emberkv {}: recovered {} records from {} segments, discarded {} bytes",
        version(),
        report.records,
        report.segments.len(),
        report.discarded_bytes
    );

    // Localhost only: EmberKV has no authentication, so it must not be reachable from other machines.
    let listener = match TcpListener::bind(("127.0.0.1", options.port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("emberkv: cannot listen on port {}: {error}", options.port);
            return ExitCode::FAILURE;
        }
    };
    eprintln!("emberkv: listening on 127.0.0.1:{}", options.port);

    tokio::select! {
        _ = serve(listener, Arc::new(engine), ProtocolLimits::default()) => {}
        _ = tokio::signal::ctrl_c() => eprintln!("emberkv: stopping"),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(words: &[&str]) -> Result<Options, String> {
        parse_options(words.iter().map(|word| (*word).to_string()))
    }

    #[test]
    fn version_has_three_parts() {
        assert_eq!(version().split('.').count(), 3);
    }

    #[test]
    fn command_line_options() {
        assert_eq!(
            options(&[]),
            Ok(Options {
                dir: PathBuf::from("data"),
                port: 7379
            })
        );
        assert_eq!(
            options(&["--port", "8000", "--dir", "/tmp/x"]),
            Ok(Options {
                dir: PathBuf::from("/tmp/x"),
                port: 8000
            })
        );
        assert!(options(&["--port"]).is_err());
        assert!(options(&["--port", "99999"]).is_err());
        assert!(options(&["--verbose", "yes"]).is_err());
    }
}

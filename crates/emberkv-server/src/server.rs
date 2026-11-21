//! The network side: accept connections and serve each one in its own task.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use emberkv_storage::engine::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::command::execute;
use crate::reply::Reply;
use crate::resp::{Parsed, ProtocolLimits, parse};

/// Accepts connections on `listener` until the task running it is cancelled, serving each connection
/// in its own task.
///
/// # Errors
///
/// Does not return errors from single connections; a failed `accept` is retried after a pause.
pub async fn serve(
    listener: TcpListener,
    engine: Arc<Engine>,
    limits: ProtocolLimits,
) -> io::Result<()> {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            // For example "too many open files". Giving up would stop serving everyone, and
            // retrying at once would spin, so wait briefly and try again.
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            // A failure on one connection, such as the client vanishing, only ends that connection.
            let _ = handle_connection(stream, engine, limits).await;
        });
    }
}

/// Reads requests from one client and writes a reply to each, in order, until the client hangs up or
/// sends something invalid.
async fn handle_connection(
    mut stream: TcpStream,
    engine: Arc<Engine>,
    limits: ProtocolLimits,
) -> io::Result<()> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let mut out = Vec::new();

    loop {
        // Answer every complete command already in the buffer; pipelined requests arrive together.
        loop {
            match parse(&buffer, &limits) {
                Ok(Parsed::Incomplete) => break,
                Ok(Parsed::Command { args, consumed }) => {
                    buffer.drain(..consumed);
                    if args.is_empty() {
                        continue; // an empty inline line
                    }
                    let engine = Arc::clone(&engine);
                    // The engine may wait for a disk flush, which must not block this async task's
                    // thread, so it runs on Tokio's pool of blocking threads.
                    let reply = tokio::task::spawn_blocking(move || execute(&engine, &args))
                        .await
                        .unwrap_or_else(|_| Reply::Error("ERR internal error".into()));
                    out.clear();
                    reply.encode(&mut out);
                    stream.write_all(&out).await?;
                }
                Err(error) => {
                    // After a malformed request the stream has lost its command boundaries, so
                    // the only safe move is to say why and close.
                    out.clear();
                    Reply::Error(format!("ERR Protocol error: {error}")).encode(&mut out);
                    stream.write_all(&out).await?;
                    return Ok(());
                }
            }
        }

        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(()); // the client closed the connection
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

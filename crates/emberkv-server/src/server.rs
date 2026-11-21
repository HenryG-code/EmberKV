//! The network side: accept connections, serve each one in its own task, enforce connection limits,
//! and shut down gracefully.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use emberkv_storage::engine::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

use crate::command::execute;
use crate::reply::Reply;
use crate::resp::{Parsed, ProtocolLimits, parse};

/// Limits for the server as a whole and for each connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerConfig {
    /// Limits on each request's arguments.
    pub protocol: ProtocolLimits,
    /// Most bytes a connection may have buffered while a request is still incomplete.
    pub max_request_bytes: usize,
    /// Most clients connected at once.
    pub max_connections: usize,
    /// A connection that sends nothing for this long is closed.
    pub idle_timeout: Duration,
    /// After shutdown starts, how long open connections get to finish before the server stops anyway.
    pub shutdown_grace: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            protocol: ProtocolLimits::default(),
            max_request_bytes: 4 * 1024 * 1024,
            max_connections: 1024,
            idle_timeout: Duration::from_secs(300),
            shutdown_grace: Duration::from_secs(10),
        }
    }
}

/// Serves connections on `listener` until `shutdown` completes. Then it stops accepting, lets each
/// connection finish the command it is running, closes them, and returns.
pub async fn serve(
    listener: TcpListener,
    engine: Arc<Engine>,
    config: ServerConfig,
    shutdown: impl Future<Output = ()>,
) {
    let slots = Arc::new(Semaphore::new(config.max_connections));
    let (stop_sender, stop) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => {
                let Ok((mut stream, _peer)) = accepted else {
                    // For example "too many open files". Giving up would stop serving everyone, and
                    // retrying at once would spin, so wait briefly and try again.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                    let mut out = Vec::new();
                    Reply::Error("ERR max number of clients reached".into()).encode(&mut out);
                    let _ = stream.write_all(&out).await;
                    continue; // dropping `stream` closes it
                };
                let engine = Arc::clone(&engine);
                let stop = stop.clone();
                connections.spawn(async move {
                    // A failure on one connection, such as the client vanishing, only ends that one.
                    let _ = handle_connection(stream, engine, config, stop).await;
                    drop(slot); // frees the place for another client
                });
            }
        }
        // Forget connections that have already finished, so the set does not grow forever.
        while connections.try_join_next().is_some() {}
    }

    // Shutdown: tell every connection to stop at its next pause, then wait for them, but not forever.
    let _ = stop_sender.send(true);
    let _ = tokio::time::timeout(config.shutdown_grace, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
}

/// Reads requests from one client and writes a reply to each, in order, until the client hangs up,
/// sends something invalid, goes quiet for too long, or the server shuts down.
async fn handle_connection(
    mut stream: TcpStream,
    engine: Arc<Engine>,
    config: ServerConfig,
    mut stop: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let mut out = Vec::new();

    loop {
        // Answer every complete command already in the buffer; pipelined requests arrive together.
        loop {
            match parse(&buffer, &config.protocol) {
                Ok(Parsed::Incomplete) => break,
                Ok(Parsed::Command { args, consumed }) => {
                    buffer.drain(..consumed);
                    if args.is_empty() {
                        continue; // an empty inline line
                    }
                    let engine = Arc::clone(&engine);
                    // The engine may wait for a disk flush, which must not block this async task's
                    // thread, so it runs on Tokio's pool of blocking threads. A command that has
                    // started always finishes and is answered, even during shutdown.
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
                    return reply_and_close(&mut stream, &format!("ERR Protocol error: {error}"))
                        .await;
                }
            }
        }
        if buffer.len() > config.max_request_bytes {
            return reply_and_close(&mut stream, "ERR Protocol error: request too large").await;
        }

        // Wait for more bytes, but give up if the client goes quiet or the server is stopping.
        let read = tokio::select! {
            read = tokio::time::timeout(config.idle_timeout, stream.read(&mut chunk)) => match read {
                Ok(read) => read?,
                Err(_elapsed) => return Ok(()),
            },
            _ = stop.wait_for(|&stopping| stopping) => return Ok(()),
        };
        if read == 0 {
            return Ok(()); // the client closed the connection
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

async fn reply_and_close(stream: &mut TcpStream, message: &str) -> std::io::Result<()> {
    let mut out = Vec::new();
    Reply::Error(message.to_string()).encode(&mut out);
    stream.write_all(&out).await
}

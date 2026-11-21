//! Integration tests over real sockets: a server on a random local port, and raw RESP2 bytes sent to it,
//! exactly as a client would.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use emberkv_server::resp::ProtocolLimits;
use emberkv_server::server::serve;
use emberkv_storage::engine::Engine;
use emberkv_storage::log::LogConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Starts a server on a free port and returns its address. The data folder lives as long as `dir`.
async fn start(dir: &tempfile::TempDir) -> std::net::SocketAddr {
    let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap(); // port 0: the OS picks a free one
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, Arc::new(engine), ProtocolLimits::default()));
    address
}

/// Sends `request` and reads exactly `expected.len()` bytes back.
async fn exchange(stream: &mut TcpStream, request: &[u8], expected: &[u8]) {
    stream.write_all(request).await.unwrap();
    let mut reply = vec![0; expected.len()];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&reply),
        String::from_utf8_lossy(expected),
        "reply to {:?}",
        String::from_utf8_lossy(request)
    );
}

#[tokio::test]
async fn ping_set_get_del_over_a_socket() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = TcpStream::connect(start(&dir).await).await.unwrap();

    exchange(&mut client, b"*1\r\n$4\r\nPING\r\n", b"+PONG\r\n").await;
    exchange(
        &mut client,
        b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$5\r\nhello\r\n",
        b"+OK\r\n",
    )
    .await;
    exchange(
        &mut client,
        b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n",
        b"$5\r\nhello\r\n",
    )
    .await;
    exchange(&mut client, b"*2\r\n$3\r\nDEL\r\n$1\r\nk\r\n", b":1\r\n").await;
    exchange(&mut client, b"*2\r\n$3\r\nGET\r\n$1\r\nk\r\n", b"$-1\r\n").await;
}

#[tokio::test]
async fn inline_commands_and_pipelining() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = TcpStream::connect(start(&dir).await).await.unwrap();

    exchange(
        &mut client,
        b"SET k v\r\nGET k\r\nPING\n",
        b"+OK\r\n$1\r\nv\r\n+PONG\r\n",
    )
    .await;
}

#[tokio::test]
async fn binary_values_survive_the_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = TcpStream::connect(start(&dir).await).await.unwrap();

    exchange(
        &mut client,
        b"*3\r\n$3\r\nSET\r\n$1\r\nb\r\n$4\r\n\r\n\x00\xff\r\n",
        b"+OK\r\n",
    )
    .await;
    exchange(
        &mut client,
        b"*2\r\n$3\r\nGET\r\n$1\r\nb\r\n",
        b"$4\r\n\r\n\x00\xff\r\n",
    )
    .await;
}

#[tokio::test]
async fn a_malformed_request_gets_an_error_and_the_connection_closes() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = TcpStream::connect(start(&dir).await).await.unwrap();

    client.write_all(b"*1\r\n:oops\r\n").await.unwrap();
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).await.unwrap(); // returns once the server closes
    assert_eq!(reply, b"-ERR Protocol error: expected a bulk string\r\n");
}

#[tokio::test]
async fn data_survives_a_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;
        engine.set(b"k".to_vec(), b"kept".to_vec()).unwrap();
    } // the engine is dropped here, releasing the folder's lock
    let mut client = TcpStream::connect(start(&dir).await).await.unwrap();
    exchange(&mut client, b"GET k\r\n", b"$4\r\nkept\r\n").await;
}

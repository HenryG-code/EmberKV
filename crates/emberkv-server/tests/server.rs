//! Integration tests over real sockets: a server on a random local port, and raw RESP2 bytes sent to it,
//! exactly as a client would.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use emberkv_server::server::{ServerConfig, serve};
use emberkv_storage::engine::Engine;
use emberkv_storage::log::LogConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// A running test server: its address, a switch to shut it down, and its task.
struct Running {
    address: SocketAddr,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

/// Starts a server on a free port with `config`. The data folder lives as long as `dir`.
async fn start_with(dir: &tempfile::TempDir, config: ServerConfig) -> Running {
    let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap(); // port 0: the OS picks a free one
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(serve(listener, Arc::new(engine), config, async {
        let _ = stopped.await;
    }));
    Running {
        address,
        stop,
        task,
    }
}

/// Starts a server that runs until the test ends.
async fn start(dir: &tempfile::TempDir) -> SocketAddr {
    let running = start_with(dir, ServerConfig::default()).await;
    std::mem::forget(running.stop); // never send the stop signal; the test runtime ends instead
    running.address
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

/// Everything the server sends until it closes the connection.
async fn read_until_closed(stream: &mut TcpStream) -> String {
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).await.unwrap();
    String::from_utf8_lossy(&reply).into_owned()
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
    assert_eq!(
        read_until_closed(&mut client).await,
        "-ERR Protocol error: expected a bulk string\r\n"
    );
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

#[tokio::test]
async fn a_web_page_cannot_smuggle_commands_through_a_browser() {
    let dir = tempfile::tempdir().unwrap();
    let address = start(&dir).await;

    // What a browser sends when a malicious page posts a form to localhost, with commands in the body.
    let mut browser = TcpStream::connect(address).await.unwrap();
    browser
        .write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nSET evil 1\r\n")
        .await
        .unwrap();
    assert_eq!(
        read_until_closed(&mut browser).await,
        "-ERR Protocol error: this is not an HTTP server\r\n"
    );

    let mut client = TcpStream::connect(address).await.unwrap();
    exchange(&mut client, b"GET evil\r\n", b"$-1\r\n").await;
}

#[tokio::test]
async fn an_oversized_request_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        max_request_bytes: 64,
        ..ServerConfig::default()
    };
    let running = start_with(&dir, config).await;
    let mut client = TcpStream::connect(running.address).await.unwrap();

    let mut request = b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1000\r\n".to_vec();
    request.extend_from_slice(&[b'x'; 100]); // never finished: the server must not wait for the rest
    client.write_all(&request).await.unwrap();
    assert_eq!(
        read_until_closed(&mut client).await,
        "-ERR Protocol error: request too large\r\n"
    );
}

#[tokio::test]
async fn clients_beyond_the_limit_are_turned_away() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        max_connections: 1,
        ..ServerConfig::default()
    };
    let running = start_with(&dir, config).await;

    let mut first = TcpStream::connect(running.address).await.unwrap();
    exchange(&mut first, b"PING\r\n", b"+PONG\r\n").await;

    let mut second = TcpStream::connect(running.address).await.unwrap();
    assert_eq!(
        read_until_closed(&mut second).await,
        "-ERR max number of clients reached\r\n"
    );

    drop(first); // frees the place
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut third = TcpStream::connect(running.address).await.unwrap();
    exchange(&mut third, b"PING\r\n", b"+PONG\r\n").await;
}

#[tokio::test]
async fn a_quiet_client_is_disconnected() {
    let dir = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        idle_timeout: Duration::from_millis(200),
        ..ServerConfig::default()
    };
    let running = start_with(&dir, config).await;
    let mut client = TcpStream::connect(running.address).await.unwrap();

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        read_until_closed(&mut client).await,
        "",
        "closed without a word"
    );
}

#[tokio::test]
async fn shutdown_finishes_cleanly_and_keeps_acknowledged_writes() {
    let dir = tempfile::tempdir().unwrap();
    let running = start_with(&dir, ServerConfig::default()).await;
    let mut client = TcpStream::connect(running.address).await.unwrap();
    exchange(&mut client, b"SET k before-shutdown\r\n", b"+OK\r\n").await;

    running.stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), running.task)
        .await
        .expect("the server stops promptly")
        .unwrap();
    assert_eq!(
        read_until_closed(&mut client).await,
        "",
        "open connections are closed"
    );

    // The engine was dropped with the server, so the folder can be opened again.
    let engine = Engine::open(dir.path(), LogConfig::default()).unwrap().0;
    assert_eq!(engine.get(b"k").unwrap(), Some(b"before-shutdown".to_vec()));
}

#[tokio::test]
async fn a_hundred_clients_at_once_each_read_back_their_own_writes() {
    let dir = tempfile::tempdir().unwrap();
    let address = start(&dir).await;

    let mut clients = tokio::task::JoinSet::new();
    for id in 0..100 {
        clients.spawn(async move {
            let mut client = TcpStream::connect(address).await.unwrap();
            let set = format!("SET key{id} value{id}\r\n");
            exchange(&mut client, set.as_bytes(), b"+OK\r\n").await;
            let value = format!("value{id}");
            let expected = format!("${}\r\n{value}\r\n", value.len());
            exchange(
                &mut client,
                format!("GET key{id}\r\n").as_bytes(),
                expected.as_bytes(),
            )
            .await;
        });
    }
    while let Some(result) = clients.join_next().await {
        result.unwrap(); // a failed assertion in any client fails the test
    }
}

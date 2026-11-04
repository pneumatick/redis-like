//! Option B: tests for buffered Connection and Tokio TCP integration.
//!
//! Install at server/src/connection_tokio_tests.rs and add to server/src/connection.rs:
//!     #[cfg(test)]
//!     #[path = "connection_tokio_tests.rs"]
//!     mod tokio_tests;
//!
//! These tests assume `Connection::new`, `Connection::read_request`,
//! `Connection::write_response` and:
//!     pub async fn handle_command(
//!         stream: tokio::net::TcpStream,
//!         db: &Arc<RwLock<Database>>,
//!     ) -> std::io::Result<()>
//!
//! They exercise the real async handler, RESP2 framing, and the shared database.
//! The listener below belongs only to the tests and uses an ephemeral port.

use super::Connection;
use crate::command::handle_command;
use crate::db::Database;
use common::resp::RespValue;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Barrier};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};

const IO_TIMEOUT: Duration = Duration::from_secs(4);

type SharedDb = Arc<RwLock<Database>>;

struct TestServer {
    addr: SocketAddr,
    db: SharedDb,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let db = Arc::new(RwLock::new(Database::new()));
        let listener_db = Arc::clone(&db);
        let (shutdown, mut shutdown_rx) = oneshot::channel();

        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, _)) => {
                            let db = Arc::clone(&listener_db);
                            clients.spawn(async move {
                                // A bad request should only end its own connection.
                                let _ = handle_command(stream, &db).await;
                            });
                        }
                        Err(_) => break,
                    },
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });

        Ok(Self { addr, db, shutdown, task })
    }

    async fn connect(&self) -> BufReader<TcpStream> {
        let stream = timeout(IO_TIMEOUT, TcpStream::connect(self.addr))
            .await.expect("connect timed out").expect("connect failed");
        BufReader::new(stream)
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        timeout(IO_TIMEOUT, self.task)
            .await.expect("server did not shut down")
            .expect("server task panicked");
    }
}

fn request(args: &[&[u8]]) -> Vec<u8> {
    let mut frame = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        frame.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        frame.extend_from_slice(arg);
        frame.extend_from_slice(b"\r\n");
    }
    frame
}

async fn send(client: &mut BufReader<TcpStream>, args: &[&[u8]]) {
    let frame = request(args);
    timeout(IO_TIMEOUT, client.get_mut().write_all(&frame))
        .await.expect("write timed out").expect("write failed");
}

// Minimal RESP2 response reader used by the network tests. Unlike the server's
// request decoder, it consumes RESP *replies* (simple strings, errors, integers,
// and optional bulk strings). The current command set doesn't return arrays.
async fn read_reply(reader: &mut BufReader<TcpStream>) -> io::Result<RespValue> {
    let mut line = Vec::new();
    if reader.read_until(b'\n', &mut line).await? == 0 {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "no reply"));
    }
    if !line.ends_with(b"\r\n") {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid reply header"));
    }
    let (prefix, body) = line[..line.len() - 2]
        .split_first()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "empty reply header"))?;

    match *prefix {
        b'+' => Ok(RespValue::SimpleString(
            String::from_utf8(body.to_vec())
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad UTF-8"))?,
        )),
        b'-' => Ok(RespValue::Error(
            String::from_utf8(body.to_vec())
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad UTF-8"))?,
        )),
        b':' => {
            let num = std::str::from_utf8(body)
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad integer"))?
                .parse::<i64>()
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad integer"))?;
            Ok(RespValue::Integer(num))
        }
        b'$' => {
            let len = std::str::from_utf8(body)
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad length"))?
                .parse::<i64>()
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad length"))?;
            if len == -1 {
                return Ok(RespValue::BulkString(None));
            }
            let len = usize::try_from(len)
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "negative length"))?;
            if len > 4 * 1024 * 1024 {
                return Err(io::Error::new(ErrorKind::InvalidData, "reply too large"));
            }
            let mut bytes = vec![0; len];
            reader.read_exact(&mut bytes).await?;
            let mut ending = [0u8; 2];
            reader.read_exact(&mut ending).await?;
            if ending != *b"\r\n" {
                return Err(io::Error::new(ErrorKind::InvalidData, "invalid bulk terminator"));
            }
            Ok(RespValue::BulkString(Some(bytes)))
        }
        _ => Err(io::Error::new(ErrorKind::InvalidData, "unsupported reply type")),
    }
}

async fn reply(client: &mut BufReader<TcpStream>) -> RespValue {
    timeout(IO_TIMEOUT, read_reply(client))
        .await.expect("read timed out").expect("invalid or missing reply")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_ping_round_trip() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    send(&mut client, &[b"PING"]).await;
    assert_eq!(reply(&mut client).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_multiple_requests_on_one_connection() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;

    send(&mut client, &[b"SET", b"foo", b"hello"]).await;
    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    send(&mut client, &[b"GET", b"foo"]).await;
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(b"hello".to_vec())));
    send(&mut client, &[b"DEL", b"foo"]).await;
    assert_eq!(reply(&mut client).await, RespValue::Integer(1));
    send(&mut client, &[b"GET", b"foo"]).await;
    assert_eq!(reply(&mut client).await, RespValue::BulkString(None));

    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_fragmented_request_is_reassembled() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    let frame = request(&[b"PING", b"fragmented"]);

    for byte in frame {
        client.get_mut().write_all(&[byte]).await.unwrap();
        tokio::task::yield_now().await;
    }
    assert_eq!(reply(&mut client).await,
               RespValue::BulkString(Some(b"fragmented".to_vec())));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_pipelined_requests_return_ordered_replies() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    let mut frames = request(&[b"SET", b"pipeline", b"42"]);
    frames.extend_from_slice(&request(&[b"GET", b"pipeline"]));
    frames.extend_from_slice(&request(&[b"INCR", b"pipeline"]));
    frames.extend_from_slice(&request(&[b"GET", b"pipeline"]));

    client.get_mut().write_all(&frames).await.unwrap();
    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(b"42".to_vec())));
    assert_eq!(reply(&mut client).await, RespValue::Integer(43));
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(b"43".to_vec())));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_idle_or_partial_client_does_not_block_another() {
    let server = TestServer::start().await.unwrap();
    let mut stalled = server.connect().await;
    // Leave a request unfinished and its connection open.
    stalled.get_mut().write_all(b"*2\r\n$4\r\nPING\r\n$").await.unwrap();

    let mut second = server.connect().await;
    send(&mut second, &[b"PING"]).await;
    assert_eq!(reply(&mut second).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_many_idle_connections_do_not_block_a_request() {
    let server = TestServer::start().await.unwrap();
    let mut idle = Vec::new();
    for _ in 0..32 {
        idle.push(server.connect().await);
    }
    let mut active = server.connect().await;
    send(&mut active, &[b"PING"]).await;
    assert_eq!(reply(&mut active).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_binary_safe_set_and_get() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    let key = b"\0\xff\r\nkey";
    let value = b"\xff\0hi\r\n";

    send(&mut client, &[b"SET", key, value]).await;
    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    send(&mut client, &[b"GET", key]).await;
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(value.to_vec())));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_bad_frame_closes_only_offending_connection() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    client.get_mut().write_all(b"*1\r\n$4\r\nPINGXX").await.unwrap();

    // The server's parser should reject malformed framing and close this socket.
    let read = timeout(IO_TIMEOUT, read_reply(&mut client)).await
        .expect("malformed connection not closed");
    assert!(read.is_err(), "malformed request unexpectedly produced a reply");

    let mut healthy = server.connect().await;
    send(&mut healthy, &[b"PING"]).await;
    assert_eq!(reply(&mut healthy).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_client_disconnection_does_not_terminate_listener() {
    let server = TestServer::start().await.unwrap();
    {
        let mut client = server.connect().await;
        send(&mut client, &[b"PING"]).await;
        assert_eq!(reply(&mut client).await, RespValue::SimpleString("PONG".into()));
        // client drops here, cleanly closing its socket.
    }
    let mut other = server.connect().await;
    send(&mut other, &[b"PING"]).await;
    assert_eq!(reply(&mut other).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_concurrent_incr_is_atomic() {
    const CLIENTS: usize = 8;
    const INCREMENTS: usize = 40;
    let server = TestServer::start().await.unwrap();
    let barrier = Arc::new(Barrier::new(CLIENTS));
    let mut workers = Vec::new();

    for _ in 0..CLIENTS {
        let addr = server.addr;
        let barrier = Arc::clone(&barrier);
        workers.push(tokio::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut client = BufReader::new(stream);
            barrier.wait().await;
            for _ in 0..INCREMENTS {
                send(&mut client, &[b"INCR", b"counter"]).await;
                assert!(matches!(reply(&mut client).await, RespValue::Integer(_)));
            }
        }));
    }

    for worker in workers {
        timeout(IO_TIMEOUT, worker).await.expect("worker timed out").unwrap();
    }

    let mut inspector = server.connect().await;
    send(&mut inspector, &[b"GET", b"counter"]).await;
    assert_eq!(reply(&mut inspector).await,
               RespValue::BulkString(Some((CLIENTS * INCREMENTS).to_string().into_bytes())));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_active_expiration_and_live_traffic_coexist() {
    let server = TestServer::start().await.unwrap();
    let cleanup_db = Arc::clone(&server.db);
    let cleanup = tokio::spawn(async move {
        for _ in 0..20 {
            sleep(Duration::from_millis(10)).await;
            // Drop the synchronous lock before any subsequent .await.
            {
                let mut db = cleanup_db.write().unwrap();
                db.remove_expired_sample(20);
            }
        }
    });

    let mut client = server.connect().await;
    send(&mut client, &[b"SET", b"short-lived", b"value", b"PX", b"30"]).await;
    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    for _ in 0..20 {
        send(&mut client, &[b"PING"]).await;
        assert_eq!(reply(&mut client).await, RespValue::SimpleString("PONG".into()));
        sleep(Duration::from_millis(5)).await;
    }
    cleanup.await.unwrap();
    // This checks physical cleanup, not just GET's lazy-expiration behavior.
    assert!(matches!(
        server.db.read().unwrap().ttl(b"short-lived").unwrap(),
        crate::db::TTL::Missing
    ));
    send(&mut client, &[b"GET", b"short-lived"]).await;
    assert_eq!(reply(&mut client).await, RespValue::BulkString(None));
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_truncated_request_does_not_terminate_listener() {
    let server = TestServer::start().await.unwrap();
    let mut bad = server.connect().await;
    bad.get_mut().write_all(b"*2\r\n$3\r\nGET\r\n$5\r\nab").await.unwrap();
    bad.get_mut().shutdown().await.unwrap();
    let response = timeout(IO_TIMEOUT, read_reply(&mut bad)).await
        .expect("truncated connection was not closed");
    assert!(response.is_err(), "truncated request unexpectedly returned a reply");

    let mut healthy = server.connect().await;
    send(&mut healthy, &[b"PING"]).await;
    assert_eq!(reply(&mut healthy).await, RespValue::SimpleString("PONG".into()));
    server.stop().await;
}

// Direct Connection tests. These specifically exercise the new Option B
// boundary (BytesMut input buffer + pure parse_request) rather than relying
// solely on command execution to test the framing.

async fn connection_pair() -> (Connection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (Connection::new(server), client)
}

#[tokio::test]
async fn connection_reassembles_one_byte_fragments() {
    let (mut connection, mut client) = connection_pair().await;
    let frame = request(&[b"SET", b"key", b"va\xfflue"]);
    let sender = tokio::spawn(async move {
        for b in frame {
            client.write_all(&[b]).await.unwrap();
            tokio::task::yield_now().await;
        }
        client.shutdown().await.unwrap();
    });
    let args = timeout(IO_TIMEOUT, connection.read_request())
        .await.expect("timed out").unwrap().unwrap();
    assert_eq!(args, vec![b"SET".to_vec(), b"key".to_vec(), b"va\xfflue".to_vec()]);
    assert_eq!(connection.read_request().await.unwrap(), None);
    sender.await.unwrap();
}

#[tokio::test]
async fn connection_does_not_lose_a_partial_second_pipelined_frame() {
    let (mut connection, mut client) = connection_pair().await;
    let first = request(&[b"PING"]);
    let second = request(&[b"GET", b"xyz"]);
    let split = second.len() - 2;
    let mut initial = first;
    initial.extend_from_slice(&second[..split]);
    client.write_all(&initial).await.unwrap();

    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request()).await.unwrap().unwrap(),
        Some(vec![b"PING".to_vec()])
    );

    client.write_all(&second[split..]).await.unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request()).await.unwrap().unwrap(),
        Some(vec![b"GET".to_vec(), b"xyz".to_vec()])
    );
    client.shutdown().await.unwrap();
    assert_eq!(connection.read_request().await.unwrap(), None);
}

#[tokio::test]
async fn connection_preserves_three_buffered_frames() {
    let (mut connection, mut client) = connection_pair().await;
    let mut bytes = request(&[b"PING"]);
    bytes.extend_from_slice(&request(&[b"INCR", b"counter"]));
    bytes.extend_from_slice(&request(&[b"GET", b"counter"]));
    client.write_all(&bytes).await.unwrap();
    client.shutdown().await.unwrap();

    assert_eq!(connection.read_request().await.unwrap(), Some(vec![b"PING".to_vec()]));
    assert_eq!(connection.read_request().await.unwrap(), Some(vec![b"INCR".to_vec(), b"counter".to_vec()]));
    assert_eq!(connection.read_request().await.unwrap(), Some(vec![b"GET".to_vec(), b"counter".to_vec()]));
    assert_eq!(connection.read_request().await.unwrap(), None);
}

#[tokio::test]
async fn connection_reports_clean_eof_between_frames() {
    let (mut connection, mut client) = connection_pair().await;
    client.shutdown().await.unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request()).await.unwrap().unwrap(),
        None
    );
}

#[tokio::test]
async fn connection_reports_truncated_frame_as_unexpected_eof() {
    let (mut connection, mut client) = connection_pair().await;
    client.write_all(b"*1\r\n$5\r\nabc").await.unwrap();
    client.shutdown().await.unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request())
            .await.unwrap().unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
}

#[tokio::test]
async fn connection_rejects_malformed_request() {
    let (mut connection, mut client) = connection_pair().await;
    client.write_all(b"*1\r\n$4\r\nPINGXX").await.unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request())
            .await.unwrap().unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[tokio::test]
async fn connection_rejects_oversized_bulk_header_without_payload() {
    let (mut connection, mut client) = connection_pair().await;
    client.write_all(b"*1\r\n$1048577\r\n").await.unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, connection.read_request())
            .await.unwrap().unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

#[tokio::test]
async fn connection_writes_resp2_replies_without_corruption() {
    let (mut connection, client) = connection_pair().await;
    let mut client = BufReader::new(client);
    connection.write_response(&RespValue::SimpleString("OK".into()))
        .await.unwrap();
    connection.write_response(&RespValue::BulkString(Some(b"\x00\xff\r\n".to_vec())))
        .await.unwrap();
    connection.write_response(&RespValue::Integer(17))
        .await.unwrap();

    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(b"\x00\xff\r\n".to_vec())));
    assert_eq!(reply(&mut client).await, RespValue::Integer(17));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokio_command_handler_handles_complete_then_fragmented_pipeline() {
    let server = TestServer::start().await.unwrap();
    let mut client = server.connect().await;
    let first = request(&[b"SET", b"a", b"1"]);
    let second = request(&[b"GET", b"a"]);
    let split = second.len() / 2;
    let mut initial = first;
    initial.extend_from_slice(&second[..split]);
    client.get_mut().write_all(&initial).await.unwrap();

    assert_eq!(reply(&mut client).await, RespValue::SimpleString("OK".into()));
    client.get_mut().write_all(&second[split..]).await.unwrap();
    assert_eq!(reply(&mut client).await, RespValue::BulkString(Some(b"1".to_vec())));
    server.stop().await;
}

//! Tests for the Option A asynchronous RESP2 request decoder.
//!
//! Install at common/src/resp_tokio_tests.rs and add to common/src/resp.rs:
//!     #[cfg(test)]
//!     #[path = "resp_tokio_tests.rs"]
//!     mod tokio_tests;
//!
//! Assumes `pub async fn read_request_async<R: tokio::io::AsyncBufRead + Unpin>(
//!     reader: &mut R,
//! ) -> std::io::Result<Option<Vec<Vec<u8>>>>` is implemented.

use super::*;
use std::io::{self, ErrorKind};
use tokio::io::{duplex, AsyncWriteExt, BufReader};
use tokio::time::{timeout, Duration};

fn request(args: &[&[u8]]) -> Vec<u8> {
    let mut frame = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        frame.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        frame.extend_from_slice(arg);
        frame.extend_from_slice(b"\r\n");
    }
    frame
}

async fn decode_bytes(bytes: &[u8]) -> io::Result<Option<Vec<Vec<u8>>>> {
    let (mut tx, rx) = duplex(1024);
    let bytes = bytes.to_vec();
    let sender = tokio::spawn(async move {
        tx.write_all(&bytes).await.expect("write test bytes");
        // Dropping tx provides EOF for truncated request tests.
    });

    let mut reader = BufReader::new(rx);
    let result = timeout(Duration::from_secs(3), read_request_async(&mut reader))
        .await
        .expect("RESP2 decoder hung");
    sender.await.expect("sender task panicked");
    result
}

#[tokio::test]
async fn async_parser_decodes_get() {
    let parsed = decode_bytes(&request(&[b"GET", b"foo"]))
        .await.unwrap().unwrap();
    assert_eq!(parsed, vec![b"GET".to_vec(), b"foo".to_vec()]);
}

#[tokio::test]
async fn async_parser_preserves_binary_arguments() {
    let key = b"\x00\xffkey";
    let value = b"\x00\r\n\xff";
    let parsed = decode_bytes(&request(&[b"SET", key, value]))
        .await.unwrap().unwrap();
    assert_eq!(parsed[1].as_slice(), &key[..]);
    assert_eq!(parsed[2].as_slice(), &value[..]);
}

#[tokio::test]
async fn async_parser_handles_one_byte_writes() {
    let frame = request(&[b"SET", b"my-key", b"some-value"]);
    let (mut tx, rx) = duplex(16);
    let sender = tokio::spawn(async move {
        for byte in frame {
            tx.write_all(&[byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    let mut reader = BufReader::new(rx);
    let parsed = timeout(Duration::from_secs(3), read_request_async(&mut reader))
        .await.expect("decoder timed out").unwrap().unwrap();
    assert_eq!(parsed, vec![b"SET".to_vec(), b"my-key".to_vec(), b"some-value".to_vec()]);
    sender.await.unwrap();
}

#[tokio::test]
async fn async_parser_decodes_pipelined_frames_without_losing_bytes() {
    let mut bytes = request(&[b"PING"]);
    bytes.extend_from_slice(&request(&[b"SET", b"x", b"1"]));
    bytes.extend_from_slice(&request(&[b"GET", b"x"]));
    let (mut tx, rx) = duplex(1024);
    tx.write_all(&bytes).await.unwrap();
    drop(tx);
    let mut reader = BufReader::new(rx);

    assert_eq!(read_request_async(&mut reader).await.unwrap(), Some(vec![b"PING".to_vec()]));
    assert_eq!(read_request_async(&mut reader).await.unwrap(),
               Some(vec![b"SET".to_vec(), b"x".to_vec(), b"1".to_vec()]));
    assert_eq!(read_request_async(&mut reader).await.unwrap(),
               Some(vec![b"GET".to_vec(), b"x".to_vec()]));
    assert_eq!(read_request_async(&mut reader).await.unwrap(), None);
}

#[tokio::test]
async fn async_parser_distinguishes_clean_eof() {
    assert_eq!(decode_bytes(b"").await.unwrap(), None);
}

#[tokio::test]
async fn async_parser_rejects_truncated_bulk_payload() {
    let error = decode_bytes(b"*2\r\n$3\r\nSET\r\n$5\r\nab")
        .await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn async_parser_rejects_wrong_prefix() {
    let error = decode_bytes(b"+PING\r\n").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_rejects_invalid_crlf() {
    let error = decode_bytes(b"*1\r\n$4\r\nPINGXX")
        .await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_rejects_negative_bulk_length() {
    let error = decode_bytes(b"*1\r\n$-1\r\n").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_rejects_oversized_bulk_before_allocating() {
    let error = decode_bytes(b"*1\r\n$1048577\r\n").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_rejects_too_many_arguments() {
    let error = decode_bytes(b"*1025\r\n").await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_rejects_oversized_numeric_header() {
    let error = decode_bytes(b"*11111111111111111111111111111111111111\r\n")
        .await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[tokio::test]
async fn async_parser_preserves_valid_first_frame_before_bad_second_frame() {
    let mut bytes = request(&[b"PING"]);
    bytes.extend_from_slice(b"*1\r\n$4\r\nPINGXX");
    let (mut tx, rx) = duplex(1024);
    tx.write_all(&bytes).await.unwrap();
    drop(tx);
    let mut reader = BufReader::new(rx);

    assert_eq!(read_request_async(&mut reader).await.unwrap(), Some(vec![b"PING".to_vec()]));
    assert_eq!(read_request_async(&mut reader).await.unwrap_err().kind(), ErrorKind::InvalidData);
}

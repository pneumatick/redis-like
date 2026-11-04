//! Option B: I/O-independent RESP2 parser tests.
//!
//! Install as `common/src/resp_parser_tests.rs`, then in `common/src/resp.rs`:
//!     #[cfg(test)]
//!     #[path = "resp_parser_tests.rs"]
//!     mod parser_tests;
//!
//! The tests target:
//!     fn parse_request(&[u8]) -> Result<Option<(Vec<Vec<u8>>, usize)>, ParseError>
//! They do not require Tokio or a running server.

use super::*;

fn frame(args: &[&[u8]]) -> Vec<u8> {
    let mut bytes = format!("*{}\r\n", args.len()).into_bytes();
    for arg in args {
        bytes.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        bytes.extend_from_slice(arg);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

fn parsed(input: &[u8]) -> (Vec<Vec<u8>>, usize) {
    parse_request(input)
        .expect("request should be valid")
        .expect("request should be complete")
}

#[test]
fn encodes_null_bulk_string() {
    let mut output = Vec::new();

    write_response(
        &mut output,
        &RespValue::BulkString(None)
    ).unwrap();

    assert_eq!(output, b"$-1\r\n");
}

#[test]
fn parses_get_request_and_exact_consumed_length() {
    let input = frame(&[b"GET", b"foo"]);
    let (args, consumed) = parsed(&input);
    assert_eq!(args, vec![b"GET".to_vec(), b"foo".to_vec()]);
    assert_eq!(consumed, input.len());
}

#[test]
fn preserves_binary_keys_and_values() {
    let key = b"\x00\xffkey\r\n";
    let value = b"\xff\x00\r\n\x80";
    let input = frame(&[b"SET", key, value]);
    let (args, consumed) = parsed(&input);
    assert_eq!(args[1].as_slice(), key);
    assert_eq!(args[2].as_slice(), value);
    assert_eq!(consumed, input.len());
}

#[test]
fn supports_empty_bulk_string_as_value() {
    let input = frame(&[b"SET", b"empty", b""]);
    let (args, consumed) = parsed(&input);
    assert_eq!(args[2].as_slice(), b"");
    assert_eq!(consumed, input.len());
}

#[test]
fn empty_input_is_incomplete_not_invalid() {
    assert!(parse_request(b"").unwrap().is_none());
}

#[test]
fn every_proper_prefix_of_valid_frame_is_incomplete() {
    let input = frame(&[b"SET", b"some-key", b"binary\x00\xff\r\nvalue"]);
    for i in 0..input.len() {
        assert!(
            parse_request(&input[..i]).unwrap().is_none(),
            "prefix of length {i} unexpectedly parsed as complete"
        );
    }
    assert_eq!(parsed(&input).1, input.len());
}

#[test]
fn incomplete_payload_does_not_consume_or_mutate_input() {
    let input = frame(&[b"SET", b"key", b"value"]);
    let truncated = &input[..input.len() - 3];
    let before = truncated.to_vec();
    assert!(parse_request(truncated).unwrap().is_none());
    assert_eq!(truncated, before.as_slice());
    assert!(parse_request(truncated).unwrap().is_none());
}

#[test]
fn parses_first_request_without_consuming_trailing_bytes() {
    let first = frame(&[b"PING"]);
    let second = frame(&[b"GET", b"foo"]);
    let mut input = first.clone();
    input.extend_from_slice(&second);
    let (args, consumed) = parsed(&input);
    assert_eq!(args, vec![b"PING".to_vec()]);
    assert_eq!(consumed, first.len());
    assert_eq!(&input[consumed..], second.as_slice());
}

#[test]
fn decodes_multiple_pipelined_frames_by_advancing_offset() {
    let commands: Vec<Vec<&[u8]>> = vec![
        vec![b"PING"],
        vec![b"SET", b"a", b"1"],
        vec![b"GET", b"a"],
        vec![b"DEL", b"a"],
    ];
    let input = commands
        .iter()
        .flat_map(|args| frame(args))
        .collect::<Vec<u8>>();

    let mut offset = 0;
    for command in commands {
        let (args, consumed) = parsed(&input[offset..]);
        let expected = command.iter().map(|v| v.to_vec()).collect::<Vec<_>>();
        assert_eq!(args, expected);
        assert!(consumed > 0);
        offset += consumed;
    }
    assert_eq!(offset, input.len());
}

#[test]
fn complete_first_and_partial_second_frame() {
    let first = frame(&[b"PING"]);
    let second = frame(&[b"GET", b"foo"]);
    let mut input = first.clone();
    input.extend_from_slice(&second[..second.len() - 1]);

    let (args, consumed) = parsed(&input);
    assert_eq!(args, vec![b"PING".to_vec()]);
    assert_eq!(consumed, first.len());
    assert!(parse_request(&input[consumed..]).unwrap().is_none());

    input.extend_from_slice(&second[second.len() - 1..]);
    let (args, second_consumed) = parsed(&input[consumed..]);
    assert_eq!(args, vec![b"GET".to_vec(), b"foo".to_vec()]);
    assert_eq!(consumed + second_consumed, input.len());
}

#[test]
fn repeated_parsing_is_stateless() {
    let input = frame(&[b"PING"]);
    assert_eq!(parsed(&input), parsed(&input));
}

#[test]
fn rejects_non_array_prefix() {
    assert!(matches!(parse_request(b"+PING\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_empty_command_array() {
    assert!(matches!(parse_request(b"*0\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_negative_array_count() {
    assert!(matches!(parse_request(b"*-1\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_excessive_array_count_before_reading_elements() {
    assert!(matches!(parse_request(b"*1025\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_non_bulk_array_element() {
    assert!(matches!(parse_request(b"*1\r\n+PING\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_null_bulk_argument() {
    assert!(matches!(parse_request(b"*1\r\n$-1\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_bulk_larger_than_limit_before_payload_arrives() {
    assert!(matches!(
        parse_request(b"*1\r\n$1048577\r\n"),
        Err(ParseError::TooLarge(_))
    ));
}

#[test]
fn rejects_sum_of_bulk_lengths_over_request_limit() {
    const MIB: usize = 1024 * 1024;
    let payload = vec![b'x'; MIB];
    let mut input = b"*5\r\n".to_vec();
    for _ in 0..4 {
        input.extend_from_slice(b"$1048576\r\n");
        input.extend_from_slice(&payload);
        input.extend_from_slice(b"\r\n");
    }
    // The parser must reject the total payload length before waiting for
    // the fifth argument's body.
    input.extend_from_slice(b"$1\r\n");
    assert!(matches!(parse_request(&input), Err(ParseError::TooLarge(_))));
}

#[test]
fn rejects_nonnumeric_length() {
    assert!(matches!(parse_request(b"*x\r\n"), Err(ParseError::Invalid(_))));
    assert!(matches!(parse_request(b"*1\r\n$nope\r\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_numeric_length_overflow() {
    assert!(matches!(
        parse_request(b"*999999999999999999999999999999\r\n"),
        Err(ParseError::Invalid(_))
    ));
}

#[test]
fn rejects_header_without_crlf() {
    assert!(matches!(parse_request(b"*1\n"), Err(ParseError::Invalid(_))));
    assert!(matches!(parse_request(b"*1\rX\n"), Err(ParseError::Invalid(_))));
}

#[test]
fn rejects_overlong_numeric_header() {
    assert!(matches!(
        parse_request(b"*111111111111111111111111111111111111111\r\n"),
        Err(ParseError::TooLarge(_))
    ));
}

#[test]
fn rejects_invalid_payload_terminator() {
    assert!(matches!(
        parse_request(b"*1\r\n$4\r\nPINGXX"),
        Err(ParseError::Invalid(_))
    ));
}

#[test]
fn incomplete_frame_is_not_a_protocol_error() {
    for partial in [
        &b"*"[..],
        &b"*2\r"[..],
        &b"*1\r\n$"[..],
        &b"*1\r\n$4\r\nPI"[..],
        &b"*1\r\n$4\r\nPING\r"[..],
    ] {
        assert!(parse_request(partial).unwrap().is_none());
    }
}

#[test]
fn response_encoding_is_unchanged_by_parser_refactor() {
    let cases: Vec<(RespValue, Vec<u8>)> = vec![
        (RespValue::SimpleString("OK".into()), b"+OK\r\n".to_vec()),
        (RespValue::Error("ERR example".into()), b"-ERR example\r\n".to_vec()),
        (RespValue::Integer(-2), b":-2\r\n".to_vec()),
        (RespValue::BulkString(None), b"$-1\r\n".to_vec()),
        (RespValue::BulkString(Some(vec![])), b"$0\r\n\r\n".to_vec()),
        (RespValue::BulkString(Some(b"hi\x00".to_vec())), b"$3\r\nhi\x00\r\n".to_vec()),
        (RespValue::Array(None), b"*-1\r\n".to_vec()),
        (RespValue::Array(Some(vec![RespValue::Integer(1)])), b"*1\r\n:1\r\n".to_vec()),
    ];
    for (response, expected) in cases {
        let mut encoded = Vec::new();
        write_response(&mut encoded, &response).unwrap();
        assert_eq!(encoded, expected);
    }
}

#[test]
fn response_encoding_rejects_newlines_in_simple_string() {
    let mut encoded = Vec::new();
    assert!(write_response(&mut encoded, &RespValue::SimpleString("bad\nline".into())).is_err());
}

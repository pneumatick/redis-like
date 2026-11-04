use std::io::{self, Write};
use std::ops::Range;

const MAX_ARGS: usize = 1024;
const MAX_BULK_SIZE: usize = 1024 * 1024;
const MAX_REQUEST_SIZE: usize = 4 * 1024 * 1024;
const MAX_HEADER_LEN: usize = 32;
pub const MAX_FRAME_BYTES: usize =
    MAX_REQUEST_SIZE + (MAX_ARGS + 1) * (MAX_HEADER_LEN + 3);


/// Represents the RESP2 data types 
#[derive(Debug, PartialEq, Eq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Option<Vec<u8>>),
    Array(Option<Vec<RespValue>>),
}

/// Compose an `io::Error` for malformed request responses.
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

// Compose a response according to the RESP2 protocol.
pub fn write_response<W: Write>(
    writer: &mut W,
    value: &RespValue,
) -> io::Result<()> {
    match value {
        RespValue::SimpleString(s) => {
            write_line(writer, b'+', s)
        }

        RespValue::Error(s) => {
            write_line(writer, b'-', s)
        }

        RespValue::Integer(n) => {
            write!(writer, ":{n}\r\n")
        }

        RespValue::BulkString(None) => {
            writer.write_all(b"$-1\r\n")
        }

        RespValue::BulkString(Some(bytes)) => {
            write!(writer, "${}\r\n", bytes.len())?;
            writer.write_all(bytes)?;
            writer.write_all(b"\r\n")
        }

        RespValue::Array(None) => {
            writer.write_all(b"*-1\r\n")
        }

        RespValue::Array(Some(items)) => {
            write!(writer, "*{}\r\n", items.len())?;

            for item in items {
                write_response(writer, item)?;
            }

            Ok(())
        }
    }
}

/// Write a single-line response for `RespValue::SimpleString` and 
/// `RespValue::Error`.
fn write_line<W: Write>(
    writer: &mut W,
    prefix: u8,
    text: &str,
) -> io::Result<()> {
    // RESP simple strings and errors cannot contain CR or LF.
    if text.bytes().any(|b| b == b'\r' || b == b'\n') {
        return Err(invalid("Invalid RESP line"));
    }

    writer.write_all(&[prefix])?;
    writer.write_all(text.as_bytes())?;
    writer.write_all(b"\r\n")
}

#[derive(Debug)]
pub enum ParseError {
    Invalid(&'static str),
    TooLarge(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) | Self::TooLarge(s) => {
                write!(f, "{s}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

fn parse_length(
    input: &[u8],
    start: usize,
) -> Result<Option<(usize, usize)>, ParseError> {
    let remaining = &input[start..];

    let newline = remaining
        .iter()
        .take(MAX_HEADER_LEN)
        .position(|&b| b == b'\n');

    let Some(newline) = newline else {
        return if remaining.len() >= MAX_HEADER_LEN {
            Err(ParseError::TooLarge("Header too long"))
        }
        else {
            Ok(None)
        };
    };

    if newline == 0 || remaining[newline - 1] != b'\r' {
        return Err(ParseError::Invalid("Expected CRLF"));
    }

    let digits = &remaining[..newline - 1];

    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::Invalid("Invalid length"));
    }

    let length = std::str::from_utf8(digits)
        .map_err(|_| ParseError::Invalid("Invalid length"))?
        .parse::<usize>()
        .map_err(|_| ParseError::Invalid("Length out of range"))?;

    Ok(Some((length, start + newline + 1)))
}

pub fn parse_request(
    input: &[u8],
) -> Result<Option<(Vec<Vec<u8>>, usize)>, ParseError> {
    if input.is_empty() {
        return Ok(None);
    }

    if input[0] != b'*' {
        return Err(ParseError::Invalid("Expected RESP array"));
    }

    // Get array element count from the command
    let Some((count, mut cursor)) = parse_length(input, 1)? else {
        return Ok(None);
    };

    if count == 0 || count > MAX_ARGS {
        return Err(ParseError::Invalid("Invalid argument count"));
    }

    let mut ranges: Vec<Range<usize>> = Vec::with_capacity(count);
    let mut total_size = 0usize;

    for _ in 0..count {
        let Some(&prefix) = input.get(cursor) else {
            return Ok(None);
        };

        if prefix != b'$' {
            return Err(ParseError::Invalid("Expected bulk string"));
        }

        // Get the payload (bulk string) size and location within input buffer
        let Some((size, start)) = parse_length(input, cursor + 1)?
        else {
            return Ok(None);
        };

        if size > MAX_BULK_SIZE {
            return Err(ParseError::TooLarge("Bulk string too large"));
        }

        total_size = total_size
            .checked_add(size)
            .ok_or(ParseError::TooLarge("Request too large"))?;

        if total_size > MAX_REQUEST_SIZE {
            return Err(ParseError::TooLarge("Request too large"));
        }

        let end = start
            .checked_add(size)
            .and_then(|n| n.checked_add(2))
            .ok_or(ParseError::TooLarge("Request too large"))?;

        // Check if all relevant data for the payload has been received
        if end > input.len() {
            return Ok(None);
        }

        let payload_end = start + size;

        if &input[payload_end..end] != b"\r\n" {
            return Err(ParseError::Invalid("Missing CRLF"));
        }

        ranges.push(start..payload_end);
        cursor = end;
    }

    // Only allocate argument payloads after the whole request has 
    // been validated.
    let args = ranges
        .into_iter()
        .map(|range| input[range].to_vec())
        .collect();

    Ok(Some((args, cursor)))
}


#[cfg(test)]
#[path = "resp_tests.rs"]
mod resp_tests;
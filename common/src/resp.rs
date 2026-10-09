use std::io::{self, BufRead, Read, Write};

const MAX_ARGS: usize = 1024;
const MAX_BULK_SIZE: usize = 1024 * 1024;
const MAX_REQUEST_SIZE: usize = 4 * 1024 * 1024;


#[derive(Debug, PartialEq, Eq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Option<Vec<u8>>),
    Array(Option<Vec<RespValue>>),
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

// Read a CRLF-terminated decimal number. Header length limited to prevent 
// unbounded reads.
fn read_number<R: BufRead>(reader: &mut R) -> io::Result<i64> {
    let mut line: Vec<u8> = Vec::with_capacity(16);

    for _ in 0..32 {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        line.push(byte[0]);

        if line.ends_with(b"\r\n") {
            let digits = &line[..line.len() - 2];

            let text = std::str::from_utf8(digits)
                .map_err(|_| invalid("Invalid numeric header"))?;

            return text.parse::<i64>()
                .map_err(|_| invalid("Invalid integer"));
        }
    }

    Err(invalid("Header too long"))
}

/// Read one RESP2 command.
/// 
/// Returns:
/// - Ok(Some(args)) for a complete command
/// - Ok(None) for a clean disconnect between commands
/// - Err(...) for malformed or truncated data
pub fn read_request<R: BufRead>(
    reader: &mut R,
) -> io::Result<Option<Vec<Vec<u8>>>> {
    let mut prefix = [0u8; 1];

    // Handle client disconnect (EOF)
    if reader.read(&mut prefix)? == 0 {
        return Ok(None);
    }

    if prefix[0] !=b'*' {
        return Err(invalid("Expected RESP array"));
    }

    let count = usize::try_from(read_number(reader)?)
        .map_err(|_| invalid("Invalid argument count"))?;

    if count == 0 || count > MAX_ARGS {
        return Err(invalid("Invalid argument count"));
    }

    let mut args = Vec::with_capacity(count);
    let mut total_size = 0usize;

    for _ in 0..count {
        reader.read_exact(&mut prefix)?;

        if prefix[0] != b'$' {
            return Err(invalid("Expected bulk string"));
        }

        let size = usize::try_from(read_number(reader)?)
            .map_err(|_| invalid("Invalid bulk length"))?;

        if size > MAX_BULK_SIZE {
            return Err(invalid("Bulk string too large"));
        }

        total_size = total_size.checked_add(size)
            .ok_or_else(|| invalid("Request too large"))?;

        if total_size > MAX_REQUEST_SIZE {
            return Err(invalid("Request too large"));
        }

        let mut data = vec![0u8; size];
        reader.read_exact(&mut data)?;

        let mut ending = [0u8; 2];
        reader.read_exact(&mut ending)?;

        if ending != *b"\r\n" {
            return Err(invalid("Missing CRLF"));
        }

        args.push(data);
    }

    Ok(Some(args))
}

// Write the response for the client
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

// RESP simple strings and error cannot contain CR or LF.
fn write_line<W: Write>(
    writer: &mut W,
    prefix: u8,
    text: &str,
) -> io::Result<()> {
    if text.bytes().any(|b| b == b'\r' || b == b'\n') {
        return Err(invalid("Invalid RESP line"));
    }

    writer.write_all(&[prefix])?;
    writer.write_all(text.as_bytes())?;
    writer.write_all(b"\r\n")
}
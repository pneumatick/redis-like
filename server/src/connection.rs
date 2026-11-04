use std::io;

use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use common::resp::{
    parse_request,
    write_response,
    RespValue,
};

// A conservative upper bound including payloads and framing.
const MAX_FRAME_BYTES: usize = common::resp::MAX_FRAME_BYTES;

pub struct Connection {
    stream: TcpStream,
    input: BytesMut,
    output: Vec<u8>,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            input: BytesMut::with_capacity(4096),
            output: Vec::with_capacity(4096),
        }
    }

    pub async fn read_request(
        &mut self,
    ) -> io::Result<Option<Vec<Vec<u8>>>> {
        loop {
            match parse_request(&self.input) {
                Ok(Some((args, consumed))) => {
                    self.input.advance(consumed);
                    return Ok(Some(args));
                }

                Ok(None) => {
                    if self.input.len() >= MAX_FRAME_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Request exceeds buffer limit",
                        ));
                    }
                }

                Err(error) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        error,
                    ));
                }
            }

            let bytes_read = self.stream
                .read_buf(&mut self.input)
                .await?;

            if bytes_read == 0 {
                return if self.input.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Incomplete RESP request",
                    ))
                };
            }
        }
    }

    pub async fn write_response(
        &mut self,
        response: &RespValue,
    ) -> io::Result<()> {
        self.output.clear();

        write_response(&mut self.output, response)?;

        self.stream.write_all(&self.output).await
    }
}

use std::io::Read;
use std::io::Write;
use std::net::{TcpListener, TcpStream};

use common::{Command};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

fn handle_client(mut stream: TcpStream) -> Result<()> {
    Ok(())
}

fn main() {
    let cmd = "0";
    let key = "apple";
    let val = "orange";

    // Convert header strings to bytes
    let command: u8 = cmd.parse().expect("Failed to parse cmd");
    let key_size: u64 = key.len().try_into().unwrap();
    let val_size: u64 = val.len().try_into().unwrap();

    // Convert key and values to bytes
    let key_bytes = key.as_bytes();
    let val_bytes = val.as_bytes();

    // Pack data into buffer
    let mut buffer: Vec<u8> = vec![command];
    buffer.extend(key_size.to_be_bytes());
    buffer.extend(val_size.to_be_bytes());
    buffer.extend(key_bytes);
    buffer.extend(val_bytes);

    println!("{:?}", buffer);

    // Establish connection with server
    if let Ok(mut stream) = TcpStream::connect("127.0.0.1:43210") {
        println!("Connected to the server!");
        stream.write_all(&buffer);
    }
    else {
        println!("Couldn't connect to server...");
    }
}

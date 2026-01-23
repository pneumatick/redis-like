use std::io::{Read, Write, stdin};
use std::net::TcpStream;

use common::{Command};

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

fn get_arg_string() -> Result<String> {
    let mut s = String::new();
    stdin()
        .read_line(&mut s)
        .expect("Failed to read user input");
    s = s.trim().to_string();

    Ok(s)
}

fn prepare_command() -> Result<Vec<u8>> {
    // Get the command and relevant data from the user
    println!("Enter the command type: ");
    let cmd = Command::from_string(get_arg_string()?)?;

    println!("\nEnter the key: ");
    let key = get_arg_string()?;

    let mut val = String::new();
    if cmd == Command::SET {
        println!("\nEnter the value: ");
        val = get_arg_string()?;
    }

    // Convert header data to bytes
    let command: u8 = Command::to_byte(cmd);
    let key_size: u64 = key.len().try_into().unwrap();
    let val_size: u64 = val.len().try_into().unwrap();

    // Convert key to bytes
    let key_bytes = key.as_bytes();

    // Pack data into buffer
    let mut buffer: Vec<u8> = vec![command];
    buffer.extend(key_size.to_be_bytes());
    buffer.extend(val_size.to_be_bytes());
    buffer.extend(key_bytes);
    // When applicable, convert value to bytes and append to buffer
    if cmd == Command::SET {
        let val_bytes = val.as_bytes();
        buffer.extend(val_bytes);
    }

    Ok(buffer)
}

fn main() {
    // Establish connection with server
    if let Ok(mut stream) = TcpStream::connect("127.0.0.1:43210") {
        println!("Connected to the server!");

        loop {
            // Prepare command data
            let buffer = match prepare_command() {
                Err(e) => {
                    eprintln!("{e}");
                    continue;
                },
                Ok(buff) => buff,
            };
            println!("\nMessage bytes:\n{:?}", buffer);

            // Send command to the server
            let _ = stream.write_all(&buffer);

            // Read and display the server's response
            let mut read_buff: Vec<u8> = vec![0; 1024];
            let _ = stream.read(&mut read_buff);
            println!("\nServer response:\n{}", String::from_utf8(read_buff).unwrap());
        }
    }
    else {
        println!("Couldn't connect to server...");
    }
}

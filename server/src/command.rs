use crate::db;

use std::io::Read;
use std::io::Write;
use std::io::ErrorKind;
use std::net::{TcpStream};

use common::{Command, Result, Error};

// Extract data from a TcpStream according to the specified byte size.
// NOTE: Trusting a declared buffer size is unsafe in real-world systems 
// due to out-of-bounds data extraction.
fn extract_data(stream: &mut TcpStream, size: u64) -> Result<Vec<u8>> {
    let size = size as usize;
    let mut buffer = vec![0u8; size];

    stream.read_exact(&mut buffer)?;

    Ok(buffer)
}

fn extract_k_or_v(stream: &mut TcpStream, buffer: &[u8]) -> Result<(Vec<u8>, u64)> {
    println!("Extracting key/value...");
    let size_bytes: [u8; 8] = buffer
        .try_into()
        .expect("Slice length mismatch");
    let size = u64::from_be_bytes(size_bytes);
    println!("Size of incoming data: {}", size);
    let bytes = extract_data(stream, size)?;

    Ok((bytes, size))
}

pub fn handle_command(mut stream: TcpStream) -> Result<()> {
   // Command Header: 1 byte; Key size: 8 bytes; Value size: 8 bytes
    let mut buffer: [u8; 17] = [0; 17];

    loop {
        // Read command header
        let bytes_read = stream.read(&mut buffer)?;

        // Handle client disconnect
        if bytes_read == 0 {
            break;
        }

        println!("Received {} bytes: {:?}", bytes_read, &buffer[..bytes_read]);

        // Get the command or restart loop if invalid
        let command = match Command::from_byte(buffer[0 as usize]) {
            Ok(c) => c,
            Err(e) => {
                // Print and send error message
                let message = format!("An error occurred: {}", e);
                eprintln!("{}", message);
                let _ = stream.write(message.as_bytes());

                // Clear any remaining data in the stream
                stream.set_nonblocking(true)?;
                let mut buff = [0; 1024];
                println!("Clearing remaining data in stream...");
                loop {
                    match stream.read(&mut buff) {
                        Ok(0) => break,
                        Ok(_n) => continue,
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => {
                            eprintln!("Error when clearing stream: {}", e);
                            break;
                        },
                    }
                }
                stream.set_nonblocking(false)?;
                println!("Stream cleared!");

                // Restart loop
                continue;
            }
        };

        // Determine the command type and execute it
        match command {
            Command::SET => { 
                // Get the key and value bytes
                let (key_bytes, k_size) = extract_k_or_v(&mut stream, &buffer[1..9])?;
                let (value_bytes, v_size) = extract_k_or_v(&mut stream, &buffer[9..17])?;
                println!("SET received: Key size = {}, Value size = {}", k_size, v_size);

                // Insert data into hash map (assuming string as value)
                db::set(key_bytes, (value_bytes, 0));

                // Announce insertion to the client
                let _ = stream.write("Insertion successful\n".as_bytes());

            }
            Command::GET => { 
                // Get the key bytes
                let (key_bytes, k_size) = extract_k_or_v(&mut stream, &buffer[1..9])?;
                println!("GET received: Key size = {}", k_size);

                // Get the data from the hash map
                let result = db::get(&key_bytes);

                if result.is_some() {
                    // Send value to the client (assume string for now)
                    let (value_bytes, _) = result.unwrap();
                    let _ = stream.write(&value_bytes);
                }
                else {
                    let message = format!("GET Error: Key not found");
                    let _ = stream.write(message.as_bytes());
                }
            }
            Command::DEL => { 
                // Get the key bytes
                let (key_bytes, k_size) = extract_k_or_v(&mut stream, &buffer[1..9])?;
                println!("DEL received: Key size = {}", k_size);

                // Delete the data from the hash map
                let result = db::del(&key_bytes);

                if result {
                    // Announce deletion to the client
                    let _ = stream.write("Deletion successful".as_bytes());
                }
                else {
                    let message = format!("DEL Error: Key not found");
                    let _ = stream.write(message.as_bytes());
                }
            }
        }

        // Print status of hash map for testing purposes
        // (assuming string as key and value)
        /*
        println!("\nMap state:");
        for (key, value) in &mut *map {
            let key_str = std::str::from_utf8(&key).expect("Invalid UTF-8 in key bytes");
            let val_str = std::str::from_utf8(&value.0).expect("Invalid UTF-8 in value bytes");
            println!("Key: {} : Value: {}", key_str, val_str);
        }
        println!("");
        */
    }

    Ok(())
}
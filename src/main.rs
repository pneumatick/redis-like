use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::collections::HashMap;

type Error = Box<dyn std::error::Error>; 
type Result<T> = std::result::Result<T, Error>; 

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    SET = 0,
    GET = 1,
    DEL = 2,
}

impl Command {
    pub fn from_byte(byte: u8) -> Result<Command> {
        match byte {
            0 => Ok(Command::SET),
            1 => Ok(Command::GET),
            2 => Ok(Command::DEL),
            _ => Err(Box::from("Unknown command")),
        }
    }
}

fn extract_data(stream: &mut TcpStream, size: u64) -> Result<Vec<u8>> {
    let size = size as usize;
    let mut buffer = vec![0u8; size];

    stream.read_exact(&mut buffer)?;

    Ok(buffer)
}

fn handle_client(mut stream: TcpStream, map: &mut HashMap<Vec<u8>, (Vec<u8>, usize)>) -> Result<()> {
    // Command: 1 byte; Key size: 8 bytes; Value size: 8 bytes
    let mut buffer = [0; 17];

    loop {
        let bytes_read = stream.read(&mut buffer)?;

        // Handle incoming data
        if bytes_read != 0 {
            println!("Received {} bytes: {:?}", bytes_read, &buffer[..bytes_read]);

            // Get the key size
            let k_size_bytes: [u8; 8] = buffer[1..9]
                .try_into()
                .expect("Slice length mismatch");
            let k_size = u64::from_be_bytes(k_size_bytes);

            // Determine the command
            // NOTE: Repeated debug code below; Encapsulate in function later
            match Command::from_byte(buffer[0])? {
                Command::SET => { 
                    println!("SET received"); 

                    // Get the value size
                    let v_size_bytes: [u8; 8] = buffer[9..17]
                        .try_into()
                        .expect("Slice length mismatch");
                    let v_size = u64::from_be_bytes(v_size_bytes);

                    println!("Key size: {}, Value size: {}", k_size, v_size);

                    // Get the key and value as bytes
                    let key_bytes = extract_data(&mut stream, k_size)?;
                    let value_bytes = extract_data(&mut stream, v_size)?;

                    // Display the data as bytes
                    println!("Key bytes: {:?}", &key_bytes[..k_size as usize]);
                    println!("Value bytes: {:?}", &value_bytes[..v_size as usize]);

                    // Insert data into hash map (assuming string as value)
                    map.insert(key_bytes, (value_bytes, 0));

                    // Print status of hash map for testing purposes
                    // (assuming string as key and value)
                    for (key, value) in &mut *map {
                        let key_str = std::str::from_utf8(&key).expect("Invalid UTF-8 in key bytes");
                        let val_str = std::str::from_utf8(&value.0).expect("Invalid UTF-8 in value bytes");
                        println!("{}: {}", key_str, val_str);
                    }
                }
                Command::GET => { 
                    println!("GET received"); 

                    println!("Key size: {}", k_size);

                    // Get the key as bytes
                    let key_bytes = extract_data(&mut stream, k_size)?;

                    // Display the data as bytes
                    println!("Key bytes: {:?}", &key_bytes[..k_size as usize]);

                    // Get the data from the hash map
                    let result = map.get(&key_bytes);

                    let key_str = std::str::from_utf8(&key_bytes).expect("Invalid UTF-8 in key bytes");
                    if result.is_some() {
                        let (value_bytes, _) = result.unwrap();
                        let val_str = std::str::from_utf8(&value_bytes).expect("Invalid UTF-8 in value bytes");
                         
                        println!("Key: {}\nValue: {}", key_str, val_str);
                    }
                    else {
                        println!("GET Error: No key matching {}", key_str);
                    }
                }
                Command::DEL => { 
                    println!("DEL received"); 

                    println!("Key size: {}", k_size);

                    // Get the key as bytes
                    let key_bytes = extract_data(&mut stream, k_size)?;

                    // Display the data as bytes
                    println!("Key bytes: {:?}", &key_bytes[..k_size as usize]);

                    // Delete the data from the hash map
                    let result = map.remove(&key_bytes);

                    let key_str = std::str::from_utf8(&key_bytes).expect("Invalid UTF-8 in key bytes");
                    if result.is_some() {
                        let (value_bytes, _) = result.unwrap();
                        let val_str = std::str::from_utf8(&value_bytes).expect("Invalid UTF-8 in value bytes");
                         
                        println!("Key: {}\nValue: {}", key_str, val_str);
                    }
                    else {
                        println!("DEL Error: No key matching {}", key_str);
                    }
                }
            }
        } 
        // Handle client disconnect
        else {
            break;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:43210")?;
    let mut map: HashMap<Vec<u8>, (Vec<u8>, usize)> = HashMap::new();

    for stream in listener.incoming() {
        let result = handle_client(stream?, &mut map);

        match result {
            Ok(_) => { println!{"Client connection closed"}; }
            Err(e) => { eprintln!{"An client error occurred: {}", e}; }
        }
    }

    Ok(())
}

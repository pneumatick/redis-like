use std::io::Read;
use std::net::{TcpListener, TcpStream};

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

fn handle_client(mut stream: TcpStream) -> Result<()> {
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
            match Command::from_byte(buffer[0])? {
                Command::SET => { 
                    println!("SET received"); 

                    // Get the value size
                    let v_size_bytes: [u8; 8] = buffer[9..17]
                        .try_into()
                        .expect("Slice length mismatch");
                    let v_size = u64::from_be_bytes(v_size_bytes);

                    println!("Key size: {}, Value size: {}", k_size, v_size);
                }
                Command::GET => { 
                    println!("GET received"); 

                    println!("Key size: {}", k_size);
                }
                Command::DEL => { 
                    println!("DEL received"); 

                    println!("Key size: {}", k_size);
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

    for stream in listener.incoming() {
        let result = handle_client(stream?);

        match result {
            Ok(_) => { println!{"Client connection closed"}; }
            Err(e) => { eprintln!{"An client error occurred: {}", e}; }
        }
    }

    Ok(())
}

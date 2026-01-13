use std::io::Read;
use std::net::{TcpListener, TcpStream};

type Error = Box<dyn std::error::Error>; 
type Result<T> = std::result::Result<T, Error>; 

fn handle_client(mut stream: TcpStream) -> Result<()> {
    let mut buffer = [0; 512];

    loop {
        let bytes_read = stream.read(&mut buffer)?;

        // Handle incoming data
        if bytes_read != 0 {
            println!("Received {} bytes: {:?}", bytes_read, &buffer[..bytes_read]);

            // If the data is expected to be a UTF-8 string, you can convert it.
            // Be cautious as TCP streams can contain arbitrary bytes.
            match std::str::from_utf8(&buffer[..bytes_read]) {
                Ok(message) => println!("Message: {}", message),
                Err(e) => eprintln!("Could not parse as utf8: {}", e),
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

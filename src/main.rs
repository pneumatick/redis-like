use std::net::{TcpListener, TcpStream, Shutdown};

type Error = Box<dyn std::error::Error>; 
type Result<T> = std::result::Result<T, Error>; 

fn handle_client(stream: TcpStream) -> Result<()> {
    let result = stream.shutdown(Shutdown::Both);

    Ok(result?)
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

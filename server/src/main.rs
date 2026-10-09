mod command;
mod db;

use std::io;
use std::net::{TcpListener, TcpStream};

fn handle_client(stream: TcpStream) -> io::Result<()> {
    command::handle_command(stream)
}

fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:43210")?;

    for stream in listener.incoming() {
        let result = handle_client(stream?);

        match result {
            Ok(_) => { println!{"Client connection closed"}; }
            Err(e) => { eprintln!{"A client error occurred: {}", e}; }
        }
    }

    Ok(())
}

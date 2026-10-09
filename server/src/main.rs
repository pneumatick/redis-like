mod command;
mod db;

use std::net::{TcpListener, TcpStream};

use common::{Result, Error};

fn handle_client(stream: TcpStream) -> Result<()> {
    command::handle_command(stream)
}

fn main() -> Result<()> {
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

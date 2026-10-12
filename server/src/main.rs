mod command;
mod db;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};

fn handle_client(stream: TcpStream, db: &Arc<RwLock<db::Database>>) -> io::Result<()> {
    command::handle_command(stream, db)
}

fn main() -> io::Result<()> {
    let db = Arc::new(RwLock::new(db::Database::new()));
    let listener = TcpListener::bind("127.0.0.1:43210")?;

    for stream in listener.incoming() {
        let stream = stream?;
        let db = Arc::clone(&db); 

        std::thread::spawn(move || {
            if let Err(error) = handle_client(stream, &db) {
                eprintln!("Client error: {error}");
            }
        });
    }

    Ok(())
}

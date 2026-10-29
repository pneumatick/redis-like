mod command;
mod db;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::db::DbError;

/// Handle the periodic cleanup cycle.
/// 
/// By default, the function is set to run roughly 10 times per second. It 
/// takes a random sample of key-value pairs with TTLs (20 by default) 
/// and removes any that are expired. If more than 25% were expired in that 
/// sample, it immediately repeats the process and continues to do so until 
/// 25% or less of the pairs in the sample were not expired.
fn cleanup(db: &Arc<RwLock<db::Database>>) {
    let Ok(mut guard) = db.write()
        .map_err(|_| DbError::LockPoisoned)
        else {
            eprintln!("Failed to acquire cleanup lock");
            return
        };

    loop {
        if guard.remove_expired_sample(20) <= 0.25 {
            return
        }
    }
}

/// Handle an incoming request from a client.
fn handle_client(stream: TcpStream, db: &Arc<RwLock<db::Database>>) -> io::Result<()> {
    command::handle_command(stream, db)
}

fn main() -> io::Result<()> {
    let db = Arc::new(RwLock::new(db::Database::new()));

    // Start periodic cleanup thread
    let clean_db = Arc::clone(&db);
    std::thread::spawn(move || {
        loop {
            cleanup(&clean_db);

            // Aim for 10 cleanup cycles per second
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    // Handle clients
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

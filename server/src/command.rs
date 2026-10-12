use crate::db::{Database, DbError};

use std::io::{self, BufReader};
use std::net::TcpStream;
use std::sync::{Arc, RwLock};

use common::resp::{
    read_request,
    write_response,
    RespValue,
};

pub fn handle_command(
    stream: TcpStream, 
    db: &Arc<RwLock<Database>>
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);

    loop {
        let args = match read_request(&mut reader)? {
            Some(args) => args,
            None => break
        };

        let response = execute(&args, db);

        write_response(reader.get_mut(), &response)?;
    }

    Ok(())
}

fn execute(request: &[Vec<u8>], db: &Arc<RwLock<Database>>) -> RespValue {
    let Some((command, args)) = request.split_first() else {
        return RespValue::Error(
            "ERR empty command".into()
        );
    };

    let wrong_arity = || RespValue::Error(
        "Err wrong number of arguments".into()
    );

    if command.eq_ignore_ascii_case(b"PING") {
        match args {
            [] => RespValue::SimpleString("PONG".into()),
            [message] => RespValue::BulkString(
                Some(message.clone())
            ),
            _ => wrong_arity(),
        }
    }
    else if command.eq_ignore_ascii_case(b"SET") {
        match args {
            [key, value] => {
                match execute_set(key, value, db) {
                    Ok(()) => { RespValue::SimpleString("OK".into()) }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }

                }
                
            }
            _ => wrong_arity(),
        }
    }
    else if command.eq_ignore_ascii_case(b"GET") {
        match args {
            [key] => {
                match execute_get(key, db) {
                    Ok(Some(value)) => { RespValue::BulkString(Some(value)) }
                    Ok(None) => { RespValue::BulkString(None) }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity(),
        }
    }
    else if command.eq_ignore_ascii_case(b"DEL") {
        if args.is_empty() {
            return wrong_arity();
        }

        let mut deleted = 0i64;

        for key in args {
            let success = match execute_del(key, db) {
                Ok(result) => { result }
                Err(error) => {
                    eprintln!("Database error: {error}");

                    return RespValue::Error("ERR internal server error".into())
                }
            };

            if success {
                deleted += 1;
            }
        }

        RespValue::Integer(deleted)
    } 
    else {
        RespValue::Error("ERR unknown command".into())
    }
}

fn execute_set(
    key: &Vec<u8>, 
    value: &Vec<u8>, 
    db: &Arc<RwLock<Database>>
) -> Result<(), DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(guard.set(key.clone(), value.clone()))
}

fn execute_get(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<Option<Vec<u8>>, DbError> {
    let guard = db.read()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(guard.get(key).map(|value| value.to_vec()))
}

fn execute_del(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<bool, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(guard.del(key))
}
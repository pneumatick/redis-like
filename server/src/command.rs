use crate::db::{Database, DbError, SetCondition, SetOptions, TTL, Value, StringValue};

use core::time;
//use std::fmt::Error;
use std::io::{self, BufReader};
use std::net::TcpStream;
use std::sync::{Arc, RwLock};
use std::time::Duration;

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
            [key, value, option_args @ ..] => {
                let options = match parse_set_options(option_args) {
                    Ok(options) => options,
                    Err(error) => {
                        return RespValue::Error(error.into());
                    }
                };

                match execute_set(key, value, options, db) {
                    Ok(true) => { RespValue::SimpleString("OK".into()) },
                    Ok(false) => { RespValue::BulkString(None) },
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
    else if command.eq_ignore_ascii_case(b"EXPIRE") || 
            command.eq_ignore_ascii_case(b"PEXPIRE") 
    {
        match args {
            [key, expire_in_str] => {
                // NOTE: Replace expect with proper error handling
                let time = match parse_i64(expire_in_str) {
                    Ok(time) => time,
                    Err(err) => return RespValue::Error(err.into()),
                };
                
                // Treat 0 and negative durations as DELs
                if time <= 0 {
                    return match execute_del(key, db) {
                        Ok(result) => RespValue::Integer(result.into()),
                        Err(error) => {
                            eprintln!("Database error: {error}");
                            RespValue::Error("ERR internal server error".into())
                        }
                    }
                }
                
                // <= 0 shouldn't make it here; 0 as default just in case for now
                let expire_in = if command.eq_ignore_ascii_case(b"EXPIRE") {
                    Duration::from_secs(
                        u64::try_from(time)
                        .unwrap_or(0)
                    )
                } 
                else {
                    Duration::from_millis(
                        u64::try_from(time)
                        .unwrap_or(0)
                    )
                };

                match execute_expire(key, expire_in, db) {
                    Ok(result) => { RespValue::Integer(result.into()) }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity()
        }
    }
    else if command.eq_ignore_ascii_case(b"TTL") {
        match args {
            [key] => {
                match execute_ttl(key, db) {
                    Ok(TTL::ExpiresIn(ttl)) => {
                        match i64::try_from(ttl.as_secs()) {
                            Ok(ttl_i64) => RespValue::Integer(ttl_i64),
                            Err(error) => {
                                eprintln!("Conversion error: {error}");
                                RespValue::Error("Err conversion error".into())
                            }
                        }
                    }
                    Ok(TTL::Persistent) => RespValue::Integer(-1),
                    Ok(TTL::Expired) | Ok(TTL::Missing) => RespValue::Integer(-2),
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity()
        }
    }
    else if command.eq_ignore_ascii_case(b"PERSIST") {
        match args {
            [key] => {
                match execute_persist(key, db) {
                    Ok(result) => {
                        if result {
                            RespValue::Integer(1)
                        }
                        else {
                            RespValue::Integer(0)
                        }
                    }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity()
        }
    }
    // Could remove INCR and DECR, replace with INCRBY where the former is 
    // add_integer(1), the latter add_integer(-1), INCRBY add_integer(n), DECRBY add_integer(-n)
    else if command.eq_ignore_ascii_case(b"INCR") {
        match args {
            [key] => {
                match execute_incr(key, db) {
                    Ok(i) => {
                        RespValue::Integer(i)
                    }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity()
        }
    }
    else if command.eq_ignore_ascii_case(b"DECR") {
        match args {
            [key] => {
                match execute_decr(key, db) {
                    Ok(i) => {
                        RespValue::Integer(i)
                    }
                    Err(error) => {
                        eprintln!("Database error: {error}");
                        RespValue::Error("Err internal server error".into())
                    }
                }
            }
            _ => wrong_arity()
        }
    }
    else {
        RespValue::Error("ERR unknown command".into())
    }
}

fn parse_set_options(
    args: &[Vec<u8>],
) -> Result<SetOptions, &'static str> {
    let mut options = SetOptions::default();
    let mut i = 0;

    while i < args.len() {
        let option = args[i].as_slice();

        if option.eq_ignore_ascii_case(b"NX") {
            if options.condition != SetCondition::Always {
                return Err("ERR syntax error");
            }
            
            options.condition = SetCondition::IfAbsent;
            i += 1;
        }
        else if option.eq_ignore_ascii_case(b"XX") {
            if options.condition != SetCondition::Always {
                return Err("ERR syntax error");
            }

            options.condition = SetCondition::IfPresent;
            i += 1;
        }
        else if option.eq_ignore_ascii_case(b"EX") 
             || option.eq_ignore_ascii_case(b"PX")
        {
            if options.expires_in.is_some() {
                return Err("ERR syntax error");
            }

            // Create the duration based on the specified option and time amount
            let amount = args.get(i + 1)
                .ok_or("ERR syntax error")?;

            let amount = parse_i64(amount)?;

            if amount <= 0 {
                return Err("ERR invalid expire time in 'set' command");
            }

            let amount = u64::try_from(amount)
                .map_err(|_| "ERR invalid expire time")?;

            options.expires_in = Some(
                if option.eq_ignore_ascii_case(b"EX") {
                    Duration::from_secs(amount)
                }
                else {
                    Duration::from_millis(amount)
                }
            );

            i += 2;
        }
        else {
            return Err("ERR syntax error");
        }
    }

    Ok(options)
}

// Helper function for converting byte arrays into i64s
fn parse_i64(bytes: &[u8]) -> Result<i64, &'static str> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or("ERR value is not an integer or out of range")
}

fn execute_set(
    key: &[u8], 
    value: &[u8], 
    options: SetOptions,
    db: &Arc<RwLock<Database>>
) -> Result<bool, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    guard.set(key.to_vec(), value.to_vec(), options)
}

fn execute_get(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<Option<Vec<u8>>, DbError> {
    // Check if the entry has expired
    match remove_if_expired(key, db) {
        Ok(TTL::Expired) => return Ok(None),
        Err(error) => return Err(error),
        _ => ()
    }

    // Execute the command
    let guard = db.read()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(
        match guard.get(key) {
            Some(Value::String(StringValue::Bytes(bytes))) => {
                Some(bytes.clone())
            }
            Some(Value::String(StringValue::Integer(n))) => {
                Some(n.to_string().into_bytes())
            }
            None => None,
        }
    )
}

fn execute_del(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<bool, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(guard.del(key))
}

fn execute_expire(
    key: &[u8],
    expire_in: Duration,
    db: &Arc<RwLock<Database>>
) -> Result<bool, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    guard.expire(key, expire_in)
}

fn execute_ttl(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<TTL, DbError> {
    // Get the TTL from remove_if_expired; no direct call necessary
    remove_if_expired(key, db)
}

fn execute_persist(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<bool, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;
    
    Ok(guard.persist(key))
}

fn execute_incr(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<i64, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    guard.incr(key)
}

fn execute_decr(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<i64, DbError> {
    let mut guard = db.write()
        .map_err(|_| DbError::LockPoisoned)?;

    guard.decr(key)
}

/// Checks the entry's TTL to determine if it is expired and removes it if needed.
/// Aquires a read lock at first, and only acquires a write lock if the 
/// entry is expired.
fn remove_if_expired(
    key: &[u8],
    db: &Arc<RwLock<Database>>
) -> Result<TTL, DbError> {
    let check_guard = db.read()
        .map_err(|_| DbError::LockPoisoned)?;
    match check_guard.ttl(key) {
        Ok(TTL::Expired) => {
            drop(check_guard);
            let mut expire_guard = db.write()
                .map_err(|_| DbError::LockPoisoned)?;
            expire_guard.remove_if_expired(key);
            Ok(TTL::Expired)
        }
        Ok(ttl) => Ok(ttl),
        Err(error) => return Err(error),
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod command_tests;

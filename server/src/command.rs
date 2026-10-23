use crate::db::{Database, DbError, SetCondition, SetOptions, TTL, Value, StringValue};

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
                let time = std::str::from_utf8(expire_in_str)
                    .expect("Invalid UTF-8 sequence")
                    .parse::<i64>()
                    .expect("Not a valid integer");
                
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

            let amount = std::str::from_utf8(amount)
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or("ERR value is not an integer or out of range")?;

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
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, RwLock};
    use std::thread;
    use std::time::Duration;

    fn new_db() -> Arc<RwLock<Database>> {
        Arc::new(RwLock::new(Database::new()))
    }

    // Execute commands using the same argument structure
    // produced by the RESP2 request parser.
    fn call(
        db: &Arc<RwLock<Database>>,
        args: &[&str],
    ) -> RespValue {
        let request: Vec<Vec<u8>> = args
            .iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect();

        execute(&request, db)
    }

    fn assert_ok(response: RespValue) {
        assert_eq!(
            response,
            RespValue::SimpleString("OK".into())
        );
    }

    fn assert_null(response: RespValue) {
        assert_eq!(
            response,
            RespValue::BulkString(None)
        );
    }

    fn assert_value(
        db: &Arc<RwLock<Database>>,
        key: &str,
        expected: &str,
    ) {
        assert_eq!(
            call(db, &["GET", key]),
            RespValue::BulkString(
                Some(expected.as_bytes().to_vec())
            )
        );
    }

    fn assert_error_contains(
        response: RespValue,
        expected: &str,
    ) {
        match response {
            RespValue::Error(message) => {
                assert!(
                    message.to_ascii_lowercase()
                        .contains(&expected.to_ascii_lowercase()),
                    "Expected error containing '{expected}', got '{message}'"
                );
            }
            other => panic!("Expected error, got {other:?}"),
        }
    }

    fn ttl(db: &Arc<RwLock<Database>>, key: &str) -> i64 {
        match call(db, &["TTL", key]) {
            RespValue::Integer(value) => value,
            other => panic!("Expected integer TTL, got {other:?}"),
        }
    }

    #[test]
    fn set_without_options() {
        let db = new_db();

        assert_ok(call(&db, &["SET", "foo", "bar"]));
        assert_value(&db, "foo", "bar");
        assert_eq!(ttl(&db, "foo"), -1);
    }

    #[test]
    fn set_nx_succeeds_if_missing() {
        let db = new_db();

        assert_ok(call(&db, &["SET", "foo", "first", "NX"]));
        assert_value(&db, "foo", "first");
    }

    #[test]
    fn set_nx_fails_if_present() {
        let db = new_db();

        assert_ok(call(&db, &["SET", "foo", "first"]));

        assert_null(call(
            &db,
            &["SET", "foo", "second", "NX"]
        ));

        assert_value(&db, "foo", "first");
    }

    #[test]
    fn set_xx_succeeds_if_present() {
        let db = new_db();

        assert_ok(call(&db, &["SET", "foo", "first"]));
        assert_ok(call(&db, &["SET", "foo", "second", "XX"]));

        assert_value(&db, "foo", "second");
    }

    #[test]
    fn set_xx_fails_if_missing() {
        let db = new_db();

        assert_null(call(
            &db,
            &["SET", "foo", "value", "XX"]
        ));

        assert_null(call(&db, &["GET", "foo"]));
    }

    #[test]
    fn set_ex_assigns_ttl() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "foo", "bar", "EX", "60"]
        ));

        assert_value(&db, "foo", "bar");

        let remaining = ttl(&db, "foo");
        assert!(
            (0..=60).contains(&remaining),
            "Expected TTL between 0 and 60, got {remaining}"
        );
    }

    #[test]
    fn set_px_expires_key() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "foo", "bar", "PX", "100"]
        ));

        thread::sleep(Duration::from_millis(150));

        assert_null(call(&db, &["GET", "foo"]));
        assert_eq!(ttl(&db, "foo"), -2);
    }

    #[test]
    fn set_accepts_options_in_either_order() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "first", "a", "NX", "EX", "60"]
        ));
        assert_ok(call(
            &db,
            &["SET", "second", "b", "EX", "60", "NX"]
        ));

        assert_value(&db, "first", "a");
        assert_value(&db, "second", "b");

        assert!((0..=60).contains(&ttl(&db, "first")));
        assert!((0..=60).contains(&ttl(&db, "second")));

        // Verify XX can also be combined with expiration.
        assert_ok(call(
            &db,
            &["SET", "first", "updated", "XX", "PX", "5000"]
        ));

        assert_value(&db, "first", "updated");

        assert!((0..=5).contains(&ttl(&db, "first")));
    }

    #[test]
    fn failed_nx_preserves_value_and_ttl() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "foo", "original", "EX", "10"]
        ));

        assert_null(call(
            &db,
            &["SET", "foo", "replacement", "NX", "EX", "60"]
        ));

        assert_value(&db, "foo", "original");

        // A failed NX must not extend the original TTL to 60.
        let remaining = ttl(&db, "foo");
        assert!(
            (0..=10).contains(&remaining),
            "Failed NX changed TTL to {remaining}"
        );
    }

    #[test]
    fn ordinary_set_clears_existing_ttl() {
        let db = new_db();

        assert_ok(call(&db, &["SET", "foo", "first"]));

        assert_eq!(
            call(&db, &["EXPIRE", "foo", "60"]),
            RespValue::Integer(1)
        );

        assert_ok(call(&db, &["SET", "foo", "second"]));

        assert_value(&db, "foo", "second");
        assert_eq!(ttl(&db, "foo"), -1);
    }

    #[test]
    fn set_nx_succeeds_on_expired_key() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "foo", "old", "PX", "30"]
        ));

        thread::sleep(Duration::from_millis(80));

        assert_ok(call(
            &db,
            &["SET", "foo", "new", "NX"]
        ));

        assert_value(&db, "foo", "new");
        assert_eq!(ttl(&db, "foo"), -1);
    }

    #[test]
    fn set_xx_fails_on_expired_key() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["SET", "foo", "old", "PX", "30"]
        ));

        thread::sleep(Duration::from_millis(80));

        assert_null(call(
            &db,
            &["SET", "foo", "new", "XX"]
        ));

        assert_null(call(&db, &["GET", "foo"]));
    }

    #[test]
    fn set_rejects_conflicting_and_duplicate_options() {
        let db = new_db();

        let invalid_cases: &[&[&str]] = &[
            &["SET", "foo", "bar", "NX", "XX"],
            &["SET", "foo", "bar", "EX", "10", "PX", "100"],
            &["SET", "foo", "bar", "NX", "NX"],
            &["SET", "foo", "bar", "XX", "XX"],
            &["SET", "foo", "bar", "EX", "10", "EX", "20"],
            &["SET", "foo", "bar", "PX", "10", "PX", "20"],
            &["SET", "foo", "bar", "UNKNOWN"],
        ];

        for args in invalid_cases {
            assert_error_contains(call(&db, args), "syntax");
        }

        // None of the invalid commands may change the DB.
        assert_null(call(&db, &["GET", "foo"]));
    }

    #[test]
    fn set_rejects_invalid_expiration_values() {
        let db = new_db();

        for args in [
            ["SET", "foo", "bar", "EX", "0"],
            ["SET", "foo", "bar", "EX", "-1"],
            ["SET", "foo", "bar", "PX", "0"],
        ] {
            assert_error_contains(
                call(&db, &args),
                "invalid expire time"
            );
        }

        for args in [
            ["SET", "foo", "bar", "EX", "abc"],
            ["SET", "foo", "bar", "PX", "1.5"],
            ["SET", "foo", "bar", "EX", "9223372036854775808"],
        ] {
            assert_error_contains(
                call(&db, &args),
                "integer"
            );
        }

        assert_error_contains(
            call(&db, &["SET", "foo", "bar", "EX"]),
            "syntax"
        );

        assert_null(call(&db, &["GET", "foo"]));
    }

    #[test]
    fn set_options_are_case_insensitive() {
        let db = new_db();

        assert_ok(call(
            &db,
            &["set", "foo", "bar", "nx", "ex", "60"]
        ));

        assert_value(&db, "foo", "bar");
        assert!((0..=60).contains(&ttl(&db, "foo")));
    }

    #[test]
    fn concurrent_set_nx_has_exactly_one_winner() {
        let db = new_db();

        const THREADS: usize = 16;
        let barrier = Arc::new(Barrier::new(THREADS + 1));

        let mut handles = Vec::new();

        for i in 0..THREADS {
            let db = Arc::clone(&db);
            let barrier = Arc::clone(&barrier);

            handles.push(thread::spawn(move || {
                let value = format!("value-{i}");

                // Synchronize the start of all commands.
                barrier.wait();

                call(
                    &db,
                    &["SET", "shared", &value, "NX"]
                )
            }));
        }

        barrier.wait();

        let mut successful = 0;
        let mut rejected = 0;

        for handle in handles {
            match handle.join().unwrap() {
                RespValue::SimpleString(s) if s == "OK" => {
                    successful += 1;
                }
                RespValue::BulkString(None) => {
                    rejected += 1;
                }
                other => panic!("Unexpected response: {other:?}"),
            }
        }

        assert_eq!(successful, 1);
        assert_eq!(rejected, THREADS - 1);

        // A successful SET must have stored one value.
        match call(&db, &["GET", "shared"]) {
            RespValue::BulkString(Some(value)) => {
                assert!(value.starts_with(b"value-"));
            }
            other => panic!("Expected stored value, got {other:?}"),
        }
    }
}

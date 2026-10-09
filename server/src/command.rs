use crate::db;

use std::io::{self, BufReader};
use std::net::TcpStream;

use common::resp::{
    read_request,
    write_response,
    RespValue,
};

pub fn handle_command(stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream);

    loop {
        let args = match read_request(&mut reader)? {
            Some(args) => args,
            None => break
        };

        let response = execute(&args);

        write_response(reader.get_mut(), &response)?;
    }

    Ok(())
}

fn execute(request: &[Vec<u8>]) -> RespValue {
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
                db::set(key.clone(), (value.clone(), 0));
                RespValue::SimpleString("OK".into())
            }
            _ => wrong_arity(),
        }
    }
    else if command.eq_ignore_ascii_case(b"GET") {
        match args {
            [key] => {
                match db::get(key) {
                    Some((value, _)) => {
                        RespValue::BulkString(Some(value))
                    }
                    None => RespValue::BulkString(None),
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
            if db::del(key) {
                deleted += 1;
            }
        }

        RespValue::Integer(deleted)
    } 
    else {
        RespValue::Error("ERR unknown command".into())
    }
}
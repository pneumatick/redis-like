//! Command behavior tests for the entire current command set.
//!
//! Install as server/src/command_tests.rs and replace the inline tests module in command.rs with:
//!
//! #[cfg(test)]
//! #[path = "command_tests.rs"]
//! mod tests;
//!
//! Tests call execute() directly; RESP framing is tested separately in common.
//! Each test uses its own Database instance. The thread tests verify atomicity.

use super::*;
use std::sync::{Arc, Barrier, RwLock};
use std::thread;
use std::time::Duration;

type SharedDb = Arc<RwLock<Database>>;

fn fresh_db() -> SharedDb {
    Arc::new(RwLock::new(Database::new()))
}

fn call(db: &SharedDb, args: &[&str]) -> RespValue {
    let args: Vec<Vec<u8>> = args.iter().map(|s| s.as_bytes().to_vec()).collect();
    execute(&args, db)
}

fn call_bytes(db: &SharedDb, args: &[&[u8]]) -> RespValue {
    let args: Vec<Vec<u8>> = args.iter().map(|s| s.to_vec()).collect();
    execute(&args, db)
}

fn assert_integer(reply: RespValue, expected: i64) {
    assert_eq!(reply, RespValue::Integer(expected));
}

fn assert_value(db: &SharedDb, key: &str, expected: &[u8]) {
    assert_eq!(
        call(db, &["GET", key]),
        RespValue::BulkString(Some(expected.to_vec()))
    );
}

fn assert_missing(db: &SharedDb, key: &str) {
    assert_eq!(call(db, &["GET", key]), RespValue::BulkString(None));
}

fn assert_ok(reply: RespValue) {
    assert_eq!(reply, RespValue::SimpleString("OK".into()));
}

fn assert_error(reply: RespValue) {
    assert!(matches!(&reply, RespValue::Error(_)), "expected RESP error, got {reply:?}");
}

fn ttl(db: &SharedDb, key: &str) -> i64 {
    match call(db, &["TTL", key]) {
        RespValue::Integer(ttl) => ttl,
        other => panic!("expected TTL integer, got {other:?}"),
    }
}


fn assert_null(reply: RespValue) {
    assert_eq!(reply, RespValue::BulkString(None));
}

fn assert_error_contains(reply: RespValue, expected: &str) {
    match reply {
        RespValue::Error(message) => assert!(
            message.to_ascii_lowercase().contains(&expected.to_ascii_lowercase()),
            "expected error containing {expected:?}, got {message:?}",
        ),
        other => panic!("expected error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SET: unconditional writes, NX/XX, EX/PX, validation, atomicity
// ---------------------------------------------------------------------------

#[test]
fn set_without_options() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "bar"]));
    assert_value(&db, "foo", b"bar");
    assert_integer(call(&db, &["TTL", "foo"]), -1);
}

#[test]
fn set_nx_succeeds_if_missing() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "first", "NX"]));
    assert_value(&db, "foo", b"first");
}

#[test]
fn set_nx_fails_if_present() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "first"]));
    assert_null(call(&db, &["SET", "foo", "second", "NX"]));
    assert_value(&db, "foo", b"first");
}

#[test]
fn set_xx_succeeds_if_present() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "first"]));
    assert_ok(call(&db, &["SET", "foo", "second", "XX"]));
    assert_value(&db, "foo", b"second");
}

#[test]
fn set_xx_fails_if_missing() {
    let db = fresh_db();
    assert_null(call(&db, &["SET", "foo", "value", "XX"]));
    assert_missing(&db, "foo");
}

#[test]
fn set_ex_assigns_ttl() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "bar", "EX", "60"]));
    assert_value(&db, "foo", b"bar");
    let remaining = ttl(&db, "foo");
    assert!(
        (0..=60).contains(&remaining),
        "expected TTL between 0 and 60, got {remaining}"
    );
}

#[test]
fn set_px_expires_key() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "bar", "PX", "100"]));
    thread::sleep(Duration::from_millis(150));
    assert_missing(&db, "foo");
    assert_integer(call(&db, &["TTL", "foo"]), -2);
}

#[test]
fn set_accepts_options_in_either_order() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "first", "a", "NX", "EX", "60"]));
    assert_ok(call(&db, &["SET", "second", "b", "EX", "60", "NX"]));
    assert_value(&db, "first", b"a");
    assert_value(&db, "second", b"b");
    assert!((0..=60).contains(&ttl(&db, "first")));
    assert!((0..=60).contains(&ttl(&db, "second")));

    assert_ok(call(&db, &["SET", "first", "updated", "XX", "PX", "5000"]));
    assert_value(&db, "first", b"updated");
    assert!((0..=5).contains(&ttl(&db, "first")));
}

#[test]
fn failed_nx_preserves_value_and_ttl() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "original", "EX", "10"]));
    assert_null(call(&db, &["SET", "foo", "replacement", "NX", "EX", "60"]));
    assert_value(&db, "foo", b"original");
    let remaining = ttl(&db, "foo");
    assert!(
        (0..=10).contains(&remaining),
        "failed NX changed TTL to {remaining}"
    );
}

#[test]
fn ordinary_set_clears_existing_ttl() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "first"]));
    assert_integer(call(&db, &["EXPIRE", "foo", "60"]), 1);
    assert_ok(call(&db, &["SET", "foo", "second"]));
    assert_value(&db, "foo", b"second");
    assert_eq!(ttl(&db, "foo"), -1);
}

#[test]
fn set_nx_succeeds_on_expired_key() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "old", "PX", "30"]));
    thread::sleep(Duration::from_millis(80));
    assert_ok(call(&db, &["SET", "foo", "new", "NX"]));
    assert_value(&db, "foo", b"new");
    assert_eq!(ttl(&db, "foo"), -1);
}

#[test]
fn set_xx_fails_on_expired_key() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "foo", "old", "PX", "30"]));
    thread::sleep(Duration::from_millis(80));
    assert_null(call(&db, &["SET", "foo", "new", "XX"]));
    assert_missing(&db, "foo");
}

#[test]
fn set_rejects_conflicting_and_duplicate_options() {
    let db = fresh_db();
    let invalid: &[&[&str]] = &[
        &["SET", "foo", "bar", "NX", "XX"],
        &["SET", "foo", "bar", "EX", "10", "PX", "100"],
        &["SET", "foo", "bar", "NX", "NX"],
        &["SET", "foo", "bar", "XX", "XX"],
        &["SET", "foo", "bar", "EX", "10", "EX", "20"],
        &["SET", "foo", "bar", "PX", "10", "PX", "20"],
        &["SET", "foo", "bar", "UNKNOWN"],
    ];
    for args in invalid {
        assert_error_contains(call(&db, args), "syntax");
    }
    assert_missing(&db, "foo");
}

#[test]
fn set_rejects_invalid_expiration_values() {
    let db = fresh_db();
    let nonpositive = [
        ["SET", "foo", "bar", "EX", "0"],
        ["SET", "foo", "bar", "EX", "-1"],
        ["SET", "foo", "bar", "PX", "0"],
    ];
    for args in nonpositive {
        assert_error_contains(call(&db, &args), "invalid expire time");
    }
    let invalid_integer = [
        ["SET", "foo", "bar", "EX", "abc"],
        ["SET", "foo", "bar", "PX", "1.5"],
        ["SET", "foo", "bar", "EX", "9223372036854775808"],
    ];
    for args in invalid_integer {
        assert_error_contains(call(&db, &args), "integer");
    }
    assert_error_contains(call(&db, &["SET", "foo", "bar", "EX"]), "syntax");
    assert_missing(&db, "foo");
}

#[test]
fn set_options_are_case_insensitive() {
    let db = fresh_db();
    assert_ok(call(&db, &["set", "foo", "bar", "nx", "ex", "60"]));
    assert_value(&db, "foo", b"bar");
    assert!((0..=60).contains(&ttl(&db, "foo")));
}

#[test]
fn concurrent_set_nx_has_exactly_one_winner() {
    const THREADS: usize = 16;
    let db = fresh_db();
    let barrier = Arc::new(Barrier::new(THREADS + 1));
    let mut handles = Vec::new();
    for i in 0..THREADS {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let value = format!("value-{i}");
            barrier.wait();
            call(&db, &["SET", "shared", &value, "NX"])
        }));
    }
    barrier.wait();
    let mut successful = 0;
    let mut rejected = 0;
    for handle in handles {
        match handle.join().unwrap() {
            RespValue::SimpleString(s) if s == "OK" => successful += 1,
            RespValue::BulkString(None) => rejected += 1,
            other => panic!("unexpected SET NX reply: {other:?}"),
        }
    }
    assert_eq!(successful, 1);
    assert_eq!(rejected, THREADS - 1);
    match call(&db, &["GET", "shared"]) {
        RespValue::BulkString(Some(value)) => assert!(value.starts_with(b"value-")),
        other => panic!("expected stored value, got {other:?}"),
    }
}

#[test]
fn ping_without_arguments_returns_pong() {
    let db = fresh_db();
    assert_eq!(
        call(&db, &["PING"]),
        RespValue::SimpleString("PONG".into())
    );
    assert_eq!(
        call(&db, &["ping"]),
        RespValue::SimpleString("PONG".into())
    );
}

#[test]
fn ping_with_message_echoes_bulk_string_including_binary() {
    let db = fresh_db();
    let message: &[u8] = &[0, 0xff, b'\r', b'\n', b'A'];
    assert_eq!(
        call_bytes(&db, &[b"PING", message]),
        RespValue::BulkString(Some(message.to_vec()))
    );
}

#[test]
fn get_present_and_missing_keys() {
    let db = fresh_db();
    assert_missing(&db, "missing");
    assert_ok(call(&db, &["SET", "key", "value"]));
    assert_value(&db, "key", b"value");
    assert_value(&db, "key", b"value");
    assert_missing(&db, "other");
}

#[test]
fn get_preserves_arbitrary_binary_data() {
    let db = fresh_db();
    let key: &[u8] = &[0, 0xff, b'k'];
    let value: &[u8] = &[0xff, 0, b'\r', b'\n', 0x80];
    assert_ok(call_bytes(&db, &[b"SET", key, value]));
    assert_eq!(
        call_bytes(&db, &[b"GET", key]),
        RespValue::BulkString(Some(value.to_vec()))
    );
}

#[test]
fn get_returns_null_for_expired_key() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "temporary", "data", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_missing(&db, "temporary");
}

#[test]
fn del_handles_existing_missing_and_duplicate_keys() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "a", "1"]));
    assert_ok(call(&db, &["SET", "b", "2"]));
    assert_integer(call(&db, &["DEL", "a", "missing", "b", "a"]), 2);
    assert_missing(&db, "a");
    assert_missing(&db, "b");
    assert_integer(call(&db, &["DEL", "a"]), 0);
}

#[test]
fn del_does_not_count_expired_key() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "old", "value", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["DEL", "old"]), 0);
    assert_missing(&db, "old");
}

#[test]
fn del_clears_expiration_metadata() {
    // Regression: the two maps must stay consistent after deletion.
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "key", "value", "EX", "60"]));
    assert_integer(call(&db, &["DEL", "key"]), 1);
    assert_integer(call(&db, &["PERSIST", "key"]), 0);
    assert_missing(&db, "key");
}

#[test]
fn expire_missing_key_returns_zero() {
    let db = fresh_db();
    assert_integer(call(&db, &["EXPIRE", "missing", "60"]), 0);
    assert_integer(call(&db, &["PEXPIRE", "missing", "60000"]), 0);
}

#[test]
fn expire_sets_and_replaces_ttl_in_seconds() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "key", "value"]));
    assert_integer(call(&db, &["EXPIRE", "key", "120"]), 1);
    assert!((0..=120).contains(&ttl(&db, "key")));
    assert_integer(call(&db, &["EXPIRE", "key", "5"]), 1);
    assert!((0..=5).contains(&ttl(&db, "key")));
    assert_value(&db, "key", b"value");
}

#[test]
fn expire_zero_and_negative_durations_delete_immediately() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "zero", "x"]));
    assert_integer(call(&db, &["EXPIRE", "zero", "0"]), 1);
    assert_missing(&db, "zero");
    assert_integer(call(&db, &["EXPIRE", "zero", "0"]), 0);

    assert_ok(call(&db, &["SET", "negative", "x"]));
    assert_integer(call(&db, &["EXPIRE", "negative", "-7"]), 1);
    assert_missing(&db, "negative");
}

#[test]
fn expire_cannot_revive_key_that_has_already_expired() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "old", "value", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["EXPIRE", "old", "60"]), 0);
    assert_missing(&db, "old");
}

#[test]
fn pexpire_uses_milliseconds() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "short", "x"]));
    assert_integer(call(&db, &["PEXPIRE", "short", "70"]), 1);
    assert_value(&db, "short", b"x");
    thread::sleep(Duration::from_millis(130));
    assert_missing(&db, "short");
    assert_integer(call(&db, &["TTL", "short"]), -2);
}

#[test]
fn pexpire_zero_and_negative_durations_delete_immediately() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "zero", "x"]));
    assert_integer(call(&db, &["PEXPIRE", "zero", "0"]), 1);
    assert_missing(&db, "zero");

    assert_ok(call(&db, &["SET", "negative", "x"]));
    assert_integer(call(&db, &["PEXPIRE", "negative", "-10"]), 1);
    assert_missing(&db, "negative");
}

#[test]
fn ttl_distinguishes_missing_persistent_and_expiring_keys() {
    let db = fresh_db();
    assert_integer(call(&db, &["TTL", "missing"]), -2);
    assert_ok(call(&db, &["SET", "permanent", "x"]));
    assert_integer(call(&db, &["TTL", "permanent"]), -1);

    assert_integer(call(&db, &["EXPIRE", "permanent", "60"]), 1);
    assert!((0..=60).contains(&ttl(&db, "permanent")));
}

#[test]
fn ttl_treats_elapsed_deadline_as_missing() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "old", "x", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["TTL", "old"]), -2);
}

#[test]
fn persist_removes_ttl_without_removing_value() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "key", "value"]));
    assert_integer(call(&db, &["EXPIRE", "key", "60"]), 1);
    assert_integer(call(&db, &["PERSIST", "key"]), 1);
    assert_integer(call(&db, &["TTL", "key"]), -1);
    assert_value(&db, "key", b"value");
    assert_integer(call(&db, &["PERSIST", "key"]), 0);
}

#[test]
fn persist_missing_or_expired_returns_zero_without_resurrection() {
    let db = fresh_db();
    assert_integer(call(&db, &["PERSIST", "missing"]), 0);
    assert_ok(call(&db, &["SET", "persistent", "x"]));
    assert_integer(call(&db, &["PERSIST", "persistent"]), 0);

    assert_ok(call(&db, &["SET", "expired", "x", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["PERSIST", "expired"]), 0);
    assert_missing(&db, "expired");
}

#[test]
fn incr_on_missing_key_creates_one() {
    let db = fresh_db();
    assert_integer(call(&db, &["INCR", "counter"]), 1);
    assert_value(&db, "counter", b"1");
    assert_integer(call(&db, &["TTL", "counter"]), -1);
}

#[test]
fn incr_parses_existing_string_and_increments_repeatedly() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "counter", "0012"]));
    assert_integer(call(&db, &["INCR", "counter"]), 13);
    assert_value(&db, "counter", b"13");
    assert_integer(call(&db, &["INCR", "counter"]), 14);
    assert_value(&db, "counter", b"14");
}

#[test]
fn incr_rejects_invalid_integer_without_modifying_value() {
    let db = fresh_db();
    for (key, value) in [("word", "hello"), ("decimal", "1.5"), ("large", "9223372036854775808")] {
        assert_ok(call(&db, &["SET", key, value]));
        assert_error(call(&db, &["INCR", key]));
        assert_value(&db, key, value.as_bytes());
    }
}

#[test]
fn incr_detects_overflow_without_modifying_value() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "max", "9223372036854775807"]));
    assert_error(call(&db, &["INCR", "max"]));
    assert_value(&db, "max", b"9223372036854775807");
}

#[test]
fn incr_preserves_ttl_and_treats_expired_key_as_missing() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "live", "5", "EX", "60"]));
    assert_integer(call(&db, &["INCR", "live"]), 6);
    assert!((0..=60).contains(&ttl(&db, "live")));

    assert_ok(call(&db, &["SET", "old", "99", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["INCR", "old"]), 1);
    assert_value(&db, "old", b"1");
    assert_integer(call(&db, &["TTL", "old"]), -1);
}

#[test]
fn decr_on_missing_key_creates_negative_one() {
    let db = fresh_db();
    assert_integer(call(&db, &["DECR", "counter"]), -1);
    assert_value(&db, "counter", b"-1");
    assert_integer(call(&db, &["TTL", "counter"]), -1);
}

#[test]
fn decr_handles_zero_negative_and_repeated_values() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "counter", "0"]));
    assert_integer(call(&db, &["DECR", "counter"]), -1);
    assert_integer(call(&db, &["DECR", "counter"]), -2);
    assert_value(&db, "counter", b"-2");
}

#[test]
fn decr_rejects_non_integer_without_modifying_value() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "counter", "nope"]));
    assert_error(call(&db, &["DECR", "counter"]));
    assert_value(&db, "counter", b"nope");
}

#[test]
fn decr_detects_underflow_without_modifying_value() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "min", "-9223372036854775808"]));
    assert_error(call(&db, &["DECR", "min"]));
    assert_value(&db, "min", b"-9223372036854775808");
}

#[test]
fn decr_preserves_ttl_and_treats_expired_key_as_missing() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "live", "5", "EX", "60"]));
    assert_integer(call(&db, &["DECR", "live"]), 4);
    assert!((0..=60).contains(&ttl(&db, "live")));

    assert_ok(call(&db, &["SET", "old", "99", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));
    assert_integer(call(&db, &["DECR", "old"]), -1);
    assert_value(&db, "old", b"-1");
    assert_integer(call(&db, &["TTL", "old"]), -1);
}

#[test]
fn command_names_are_case_insensitive() {
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "n", "0"]));
    assert_integer(call(&db, &["incr", "n"]), 1);
    assert_value(&db, "n", b"1");
    assert_integer(call(&db, &["decr", "n"]), 0);
    assert_integer(call(&db, &["eXpIrE", "n", "60"]), 1);
    assert!((0..=60).contains(&ttl(&db, "n")));
    assert_integer(call(&db, &["persist", "n"]), 1);
    assert_integer(call(&db, &["del", "n"]), 1);
}

#[test]
fn every_command_rejects_wrong_arity() {
    let db = fresh_db();
    let invalid: &[&[&str]] = &[
        &["PING", "one", "two"],
        &["SET"], &["SET", "key"],
        &["GET"], &["GET", "a", "b"],
        &["DEL"],
        &["EXPIRE", "a"], &["EXPIRE", "a", "1", "extra"],
        &["PEXPIRE", "a"], &["PEXPIRE", "a", "1", "extra"],
        &["TTL"], &["TTL", "a", "b"],
        &["PERSIST"], &["PERSIST", "a", "b"],
        &["INCR"], &["INCR", "a", "b"],
        &["DECR"], &["DECR", "a", "b"],
    ];

    for args in invalid {
        assert_error(call(&db, args));
    }
}

#[test]
fn unknown_and_empty_commands_return_errors() {
    let db = fresh_db();
    assert_error(call(&db, &["NOT_A_COMMAND", "a"]));
    assert_error(execute(&[], &db));
}

#[test]
fn expire_and_pexpire_reject_invalid_integer_arguments() {
    // As of the reviewed main branch, this test exposes a known panic:
    // EXPIRE/PEXPIRE use expect() instead of sending a RESP error.
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "key", "value"]));

    for cmd in ["EXPIRE", "PEXPIRE"] {
        for invalid in ["garbage", "1.25", "9223372036854775808"] {
            assert_error(call(&db, &[cmd, "key", invalid]));
            assert_value(&db, "key", b"value");
        }
    }
}

#[test]
fn concurrent_incr_operations_do_not_lose_updates() {
    const THREADS: usize = 8;
    const OPERATIONS: usize = 75;

    let db = fresh_db();
    assert_ok(call(&db, &["SET", "counter", "0"]));
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::new();

    for _ in 0..THREADS {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..OPERATIONS {
                match call(&db, &["INCR", "counter"]) {
                    RespValue::Integer(_) => {}
                    other => panic!("unexpected INCR reply: {other:?}"),
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    let expected = (THREADS * OPERATIONS).to_string();
    assert_value(&db, "counter", expected.as_bytes());
}

#[test]
fn concurrent_decr_operations_do_not_lose_updates() {
    const THREADS: usize = 8;
    const OPERATIONS: usize = 75;

    let db = fresh_db();
    assert_ok(call(&db, &["SET", "counter", "0"]));
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = Vec::new();

    for _ in 0..THREADS {
        let db = Arc::clone(&db);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..OPERATIONS {
                match call(&db, &["DECR", "counter"]) {
                    RespValue::Integer(_) => {}
                    other => panic!("unexpected DECR reply: {other:?}"),
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    let expected = format!("-{}", THREADS * OPERATIONS);
    assert_value(&db, "counter", expected.as_bytes());
}

#[test]
fn active_expiration_sampling_removes_expired_keys() {
    // Direct database test: verifies active cleanup, which has no RESP command.
    let db = fresh_db();
    assert_ok(call(&db, &["SET", "expired", "x", "PX", "30"]));
    thread::sleep(Duration::from_millis(90));

    let mut guard = db.write().unwrap();
    guard.remove_expired_sample(20);
    // If the sample removed it, lazy deletion should have nothing left to do.
    assert!(!guard.remove_if_expired(b"expired"));
    drop(guard);

    assert_missing(&db, "expired");
}

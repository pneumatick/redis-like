use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, SystemTimeError};
use indexmap::IndexMap;
use rand::RngExt;

pub struct Entry {
    value: Value,
    // String representation? Otherwise probably remove this...
}

pub struct Database {
    entries: HashMap<Vec<u8>, Entry>,
    expirations: IndexMap<Vec<u8>, SystemTime>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            expirations: IndexMap::new(),
        }
    }

    /// Adds or overwrites a key-value pair to the database.
    /// 
    /// Uses optional arguments to set TTLs or to change behavior depending on 
    /// whether or not an entry with the given key already exists.
    /// 
    /// Returns a `DbResult<bool>` which indicates whether or not the 
    /// key-value pair was successfully added to the database.
    pub fn set(
        &mut self, 
        key: Vec<u8>, 
        value: Vec<u8>,
        options: SetOptions
    ) -> DbResult<bool> {
        // Remove the key if it's expired
        self.remove_if_expired(&key);

        // Check that the key exists
        let exists = self.entries.contains_key(&key);

        match options.condition {
            SetCondition::IfAbsent if exists => { return Ok(false); }
            SetCondition::IfPresent if !exists => { return Ok(false); }
            _ => {}
        }

        let now = SystemTime::now();

        let expires_at = options.expires_in
            .map(|duration| {
                now.checked_add(duration)
                    .ok_or(DbError::InvalidExpiration)
            })
            .transpose()?;

        // Insert the KV pair into the hashmaps
        self.entries.insert(
            key.clone(), 
            Entry {
                value: Value::String(StringValue::Bytes(value)),
        });
        match expires_at {
            Some(expiration) => {
                self.expirations.insert(
                    key,
                    expiration
                );
            }
            // Clear existing TTL if one exists
            None => {
                self.expirations.swap_remove(&key);
            }
        }

        Ok(true)
    }

    /// Get the value associated with the given key if it exists.
    /// 
    /// Expired keys are treated as non-existent.
    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        let entry = self.entries.get(key)?;

        if self.expirations
            .get(key)
            .is_some_and(|&expiration| expiration <= SystemTime::now()) {
                return None;
            }
        
        Some(&entry.value)
    }

    /// Remove a key-value pair from the database. 
    pub fn del(&mut self, key: &[u8]) -> bool {
        // Check for expiration first, remove if necessary and treat 
        // the key as though it didn't exist
        if self.remove_if_expired(key) {
            return false
        }

        self.expirations.swap_remove(key);
        self.entries.remove(key).is_some()
    }

    /// Assign or modify a TTL for a key-value pair that exists in the 
    /// database.
    /// 
    /// Returns `DbResult<bool>` indicating whether or not the assignment or 
    /// modification was successful.
    pub fn expire(&mut self, key: &[u8], expire_in: Duration) -> DbResult<bool> {
        match self.entries.get(key) {
            Some(_) => {
                let now = SystemTime::now();

                // Make sure the entry hasn't already expired
                match self.expirations.get(key) {
                    Some(&expiration) => {
                        if expiration > now {
                            self.expirations.insert(
                                key.to_vec(), 
                                now
                                    .checked_add(expire_in)
                                    .ok_or(DbError::InvalidExpiration)?
                            );
                            Ok(true)
                        }
                        else { 
                            self.remove_if_expired(key);
                            Ok(false) 
                        }
                        
                    }
                    _ => {
                        self.expirations.insert(
                            key.to_vec(),
                            now
                                .checked_add(expire_in)
                                .ok_or(DbError::InvalidExpiration)?
                        );
                        Ok(true)
                    }
                }
            }
            None => Ok(false)
        }
    }

    /// Remove an key-value pair and all associated data from the database 
    /// if it is expired.
    /// 
    /// Returns a `bool` which is true if the key was expired and removed, 
    /// and false either if the key is not expired or if it doesn't exist.
    pub fn remove_if_expired(
        &mut self,
        key: &[u8],
    ) -> bool {
        let expired = match self.expirations.get(key) {
            Some(&expiration) => expiration <= SystemTime::now(),
            _ => false
        };

        if expired {
            self.entries.remove(key);
            self.expirations.swap_remove(key);
        }

        expired
    }

    /// Sample a number of keys with TTLs and remove those that are expired.
    /// 
    /// This is called as a part of the persistent cleanup process which is 
    /// done roughly 10 times per second by default.
    /// 
    /// Returns an `f64` ranging between 0.0 and 1.0 which describes the 
    /// proportion of samples that were expired and removed. 
    pub fn remove_expired_sample(&mut self, count: u32) -> f64 {
        let mut rng = rand::rng();
        let now = SystemTime::now();
        let mut removed = 0;

        for _ in 0..count {
            if self.expirations.is_empty() {
                break;
            }

            let idx = rng.random_range(0..self.expirations.len());

            let expired_key = {
                let (key, expiration) = 
                    self.expirations.get_index(idx).unwrap();
                
                if *expiration <= now {
                    Some(key.clone())
                }
                else {
                    None
                }
            };

            if let Some(key) = expired_key {
                if self.remove_if_expired(&key) {
                    removed += 1
                }
            }
        }

        f64::from(removed) / f64::from(count)
    }

    /// Get the TTL for a given key-value pair.
    pub fn ttl(&self, key: &[u8]) -> DbResult<TTL> {
        match self.entries.get(key) {
            Some(_) => {
                match self.expirations.get(key) {
                    Some(&expiration) => {
                        // Make sure the entry hasn't already expired
                        let now = SystemTime::now();
                        if expiration > now {
                            let seconds = expiration.duration_since(now)
                                .map_err(DbError::ClockError)?;

                            Ok(TTL::ExpiresIn(seconds))
                        }
                        else {
                            Ok(TTL::Expired)
                        }
                    }
                    _ => Ok(TTL::Persistent)
                }
            }
            _ => {
                Ok(TTL::Missing)
            }
        }
    }

    /// Remove a TTL from a given key-value pair if one exists.
    /// 
    /// Returns a `bool` which is true if a TTL was present and removed, 
    /// and false if the key either had no TTL or doesn't exist.
    pub fn persist(&mut self, key: &[u8]) -> bool {
        // Check for expiration first, remove if necessary and treat 
        // the key as though it didn't exist
        if self.remove_if_expired(key) {
            return false
        }

        self.expirations.swap_remove(key).is_some()
    }

    /// Increment an integer value by one for the given key.
    /// 
    /// If the key does not exist, it will be added to the database 
    /// with a default value of 1.
    /// 
    /// Returns a `DbResult<i64>` containing the incremented value.
    pub fn incr(&mut self, key: &[u8]) -> DbResult<i64> {
        // Remove expired entry if necessary
        self.remove_if_expired(key);

        match self.entries.get_mut(key) {
            Some(entry) => entry.value.add_integer(1),
            None => {
                self.set(key.to_vec(), vec!['0' as u8], SetOptions::default())?;
                // SET is only false with options, so always true here
                self.incr(key)
            }
        }
    }

    /// Decrement an integer value by one for the given key.
    /// 
    /// If the key does not exist, it will be added to the database 
    /// with a default value of -1.
    /// 
    /// Returns a `DbResult<i64>` containing the decremented value.
    pub fn decr(&mut self, key: &[u8]) -> DbResult<i64> {
        // Remove expired entry if necessary
        self.remove_if_expired(key);

        match self.entries.get_mut(key) {
            Some(entry) => entry.value.sub_integer(1),
            None => {
                self.set(key.to_vec(), vec!['0' as u8], SetOptions::default())?;
                // SET is only false with options, so always true here
                self.decr(key)
            }
        }
    }
}


#[derive(Debug, PartialEq, Eq)]
pub enum Value {
    String(StringValue),
    /*
    List(Vec<Vec<u8>>),
    Set(HashSet<Vec<u8>>),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
    */
}

impl Value {
    /// Attempt to add an integer to the value.
    /// 
    /// Returns a `Result<i64, DbError>` containing the new integer, 
    /// or an error in the case of value incompatibility or overflow.
    pub fn add_integer(&mut self, delta: i64) -> DbResult<i64> {
        let Value::String(string) = self else {
            return Err(DbError::WrongType);
        };

        let current = match string {
            StringValue::Integer(i) => *i,
            StringValue::Bytes(bytes) => {
                std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .ok_or(DbError::NotInteger)?
            }
        };

        let result = current
            .checked_add(delta)
            .ok_or(DbError::Overflow)?;

        *string = StringValue::Integer(result);

        Ok(result)
    }

    /// Attempt to subtract an integer to the value.
    /// 
    /// Returns a `Result<i64, DbError>` containing the new integer, 
    /// or an error in the case of value incompatibility or underflow.
    pub fn sub_integer(&mut self, delta: i64) -> DbResult<i64> {
        let Value::String(string) = self else {
            return Err(DbError::WrongType);
        };

        let current = match string {
            StringValue::Integer(i) => *i,
            StringValue::Bytes(bytes) => {
                std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .ok_or(DbError::NotInteger)?
            }
        };

        let result = current
            .checked_sub(delta)
            .ok_or(DbError::Underflow)?;

        *string = StringValue::Integer(result);

        Ok(result)
    }

    /// Create a new copy of a `Value`.
    pub fn copy(&self) -> Value {
        match self {
            Value::String(s_value) => Value::String(s_value.copy_value())
        }
    }
}

/// Holds the actual type that a `Value::String` represents.
/// 
/// Since RESP2 encodes values such as integers as strings, a separate 
/// `StringValue` type was created to simplify certain operations, such as 
/// addition and subtraction for integers, so that decoding the string 
/// representation every time such an operation is required is not necessary.
#[derive(Debug, PartialEq, Eq)]
pub enum StringValue {
    Bytes(Vec<u8>),
    Integer(i64),
}

impl StringValue {
    // Create a new copy of `StringValue`.
    pub fn copy_value(&self) -> StringValue {
        match self {
            StringValue::Bytes(v) => StringValue::Bytes(v.clone()),
            StringValue::Integer(i) => StringValue::Integer(*i)
        }
    }
}

#[derive(Debug)]
pub enum DbError {
    LockPoisoned,
    InvalidExpiration,
    ClockError(SystemTimeError),
    WrongType,
    NotInteger,
    Overflow,
    Underflow,
}

/// Represent the TTL state for keys.
pub enum TTL {
    Missing,
    Persistent,
    Expired,
    ExpiresIn(Duration),
}

/// Represent optional conditional arguments that SET commands can handle.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum SetCondition {
    #[default]
    /// Corresponds to the default where no optional argument is given in the 
    /// command.
    Always,
    /// Corresponds to the `NX` argument.
    IfAbsent, 
    /// Corresponds to the `XX` argument.
    IfPresent,
}

/// Represent all optional arguments that SET commands can handle.
#[derive(Debug, Default)]
pub struct SetOptions {
    /// Conditional arguments such as `NX` and `XX`.
    pub condition: SetCondition,
    /// The TTL for the key-value pair if one was specified.
    pub expires_in: Option<Duration>,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockPoisoned => {
                write!(f, "Database lock poisoned")
            }
            Self::InvalidExpiration => {
                write!(f, "Expiration value invalid")
            }
            Self::ClockError(err) => {
                write!(f, "System clock error: {}", err)
            }
            Self::WrongType => {
                write!(f, "Wrong Redis type")
            }
            Self::NotInteger => {
                write!(f, "Not an integer or out of range")
            }
            Self::Overflow => {
                write!(f, "Integer overflow")
            }
            Self::Underflow => {
                write!(f, "Integer underflow")
            }
        }
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;
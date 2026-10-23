use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime, SystemTimeError};

pub struct Entry {
    value: Value,
    // String representation? Otherwise probably remove this...
}

pub struct Database {
    entries: HashMap<Vec<u8>, Entry>,
    expirations: HashMap<Vec<u8>, SystemTime>,
}

impl Database {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            expirations: HashMap::new(),
        }
    }

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
                self.expirations.remove(&key);
            }
        }

        Ok(true)
    }

    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        let entry = self.entries.get(key)?;

        if self.expirations
            .get(key)
            .is_some_and(|&expiration| expiration <= SystemTime::now()) {
                return None;
            }
        
        Some(&entry.value)
    }

    pub fn del(&mut self, key: &[u8]) -> bool {
        // Check for expiration first, remove if necessary and treat 
        // the key as though it didn't exist
        if self.remove_if_expired(key) {
            return false
        }

        self.entries.remove(key).is_some()
    }

    // NOTE: Returns Result<> because an actual error (overflow) can occur
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
            self.expirations.remove(key);
        }

        expired
    }

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

    pub fn persist(&mut self, key: &[u8]) -> bool {
        // Check for expiration first, remove if necessary and treat 
        // the key as though it didn't exist
        if self.remove_if_expired(key) {
            return false
        }

        self.expirations.remove(key).is_some()
    }

    pub fn incr(&mut self, key: &[u8]) -> DbResult<i64> {
        // Remove expired entry if necessary
        self.remove_if_expired(key);

        match self.entries.get_mut(key) {
            Some(entry) => {
                match entry.value.add_integer(1) {
                    Ok(i) => Ok(i),
                    Err(err) => Err(err),
                }
            }
            None => {
                let attempt = self.set(key.to_vec(), vec!['0' as u8], SetOptions::default());
                match attempt {
                    Ok(_) => {
                        // Attempt is only false with options. Always true here
                        Ok(1)
                    }
                    Err(err) => Err(err)
                }
            }
        }
    }

    pub fn decr(&mut self, key: &[u8]) -> DbResult<i64> {
        // Remove expired entry if necessary
        self.remove_if_expired(key);

        match self.entries.get_mut(key) {
            Some(entry) => {
                match entry.value.sub_integer(1) {
                    Ok(i) => Ok(i),
                    Err(err) => Err(err),
                }
            }
            None => {
                let attempt = self.set(key.to_vec(), vec!['0' as u8], SetOptions::default());
                match attempt {
                    Ok(_) => {
                        // Attempt is only false with options. Always true here
                        Ok(0)
                    }
                    Err(err) => Err(err)
                }
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
    pub fn add_integer(&mut self, delta: i64) -> Result<i64, DbError> {
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

    pub fn sub_integer(&mut self, delta: i64) -> Result<i64, DbError> {
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

    pub fn copy(&self) -> Value {
        match self {
            Value::String(s_value) => Value::String(s_value.copy_value())
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StringValue {
    Bytes(Vec<u8>),
    Integer(i64),
}

impl StringValue {
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

pub enum TTL {
    Missing,
    Persistent,
    Expired,
    ExpiresIn(Duration),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum SetCondition {
    #[default]
    Always,
    IfAbsent,  // NX
    IfPresent, // XX
}

#[derive(Debug, Default)]
pub struct SetOptions {
    pub condition: SetCondition,
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
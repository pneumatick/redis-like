use std::collections::HashMap;
use std::time::{Duration, SystemTime, SystemTimeError};

pub struct Entry {
    value: Value,
    expires_at: Option<SystemTime>,
}

impl Entry {
    fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|expiration| expiration <= now)
    }
}

pub struct Database {
    entries: HashMap<Vec<u8>, Entry>
}

impl Database {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
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
        let exists = self.entries.contains_key(key.as_slice());

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

        self.entries.insert(
            key, 
            Entry {
                value: Value::Bytes(value),
                expires_at,
        });

        Ok(true)
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries
            .get(key)
            .filter(|entry| {
                entry.expires_at
                    .is_none_or(|expires_at| expires_at > SystemTime::now())
            })
            .map(|entry| entry.value.as_slice())?
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
        match self.entries.get_mut(key) {
            Some(entry) => {
                let now = SystemTime::now();

                // Make sure the entry hasn't already expired
                match entry.expires_at {
                    Some(expiration) => {
                        if expiration > now {
                            entry.expires_at = Some(
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
                    None => {
                        entry.expires_at = Some(
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
        let now = SystemTime::now();
        let expired = self.entries
            .get(key)
            .is_some_and(|entry| entry.is_expired(now));

        if expired {
            self.entries.remove(key);
        }

        expired
    }

    pub fn ttl(&self, key: &[u8]) -> DbResult<TTL> {
        match self.entries.get(key) {
            Some(entry) => {
                match entry.expires_at {
                    Some(expiration) => {
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

        match self.entries.get_mut(key) {
            Some(entry) => {
                match entry.expires_at {
                    Some(_ttl) => { 
                        entry.expires_at = None;
                        true
                    }
                    _ => false
                }
            },
            _ => false
        }
    }
}


#[derive(Debug, PartialEq, Eq)]
enum Value {
    Bytes(Vec<u8>),
    Integer(i64),
}

impl Value {
    fn as_slice(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(v) => Some(v.as_slice()),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum DbError {
    LockPoisoned,
    InvalidExpiration,
    ClockError(SystemTimeError),
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
        }
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;
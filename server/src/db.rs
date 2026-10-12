use std::collections::HashMap;
use std::time::{Duration, SystemTime};

pub struct Entry {
    value: Vec<u8>,
    expires_at: Option<SystemTime>,
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

    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.entries.insert(
            key, 
            Entry {
            value,
            expires_at: None,
        });
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries
            .get(key)
            .filter(|entry| {
                entry.expires_at
                    .is_none_or(|expires_at| expires_at > SystemTime::now())
            })
            .map(|entry| entry.value.as_slice())
    }

    pub fn del(&mut self, key: &[u8]) -> bool {
        self.entries.remove(key).is_some()
    }

    // NOTE: Returns Result<> because an actual error (overflow) can occur
    pub fn expire(&mut self, key: &[u8], expire_in: u64) -> DbResult<bool> {
        if expire_in == 0 {

        }

        match self.entries.get_mut(key) {
            Some(entry) => {
                let now = SystemTime::now();

                // Make sure the entry hasn't already expired
                match entry.expires_at {
                    Some(expiration) => {
                        if expiration > now {
                            entry.expires_at = Some(
                                now
                                .checked_add(Duration::from_secs(expire_in))
                                .ok_or(DbError::InvalidExpiration)?
                            );
                            Ok(true)
                        }
                        else { Ok(false) }
                        
                    }
                    None => {
                        entry.expires_at = Some(now + Duration::from_secs(expire_in));
                        Ok(true)
                    }
                }
            }
            None => Ok(false)
        }
    }
}


#[derive(Debug)]
pub enum DbError {
    LockPoisoned,
    InvalidExpiration,
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
        }
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;
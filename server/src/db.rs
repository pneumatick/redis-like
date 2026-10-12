use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static MAP: OnceLock<RwLock<HashMap<Vec<u8>, Vec<u8>>>> = OnceLock::new();
pub struct Database {
    entries: HashMap<Vec<u8>, Vec<u8>>
}

impl Database {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.entries.insert(key, value);
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    pub fn del(&mut self, key: &[u8]) -> bool {
        self.entries.remove(key).is_some()
    }
}


#[derive(Debug)]
pub enum DbError {
    LockPoisoned,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockPoisoned => {
                write!(f, "Database lock poisoned")
            }
        }
    }
}

impl std::error::Error for DbError {}

pub type DbResult<T> = Result<T, DbError>;
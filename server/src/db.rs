use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static MAP: OnceLock<RwLock<HashMap<Vec<u8>, (Vec<u8>, usize)>>> = OnceLock::new();


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


fn get_map() -> &'static RwLock<HashMap<Vec<u8>, (Vec<u8>, usize)>> {
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

/// (Redis: SET) Insert or update a key-value pair
pub fn set(key: Vec<u8>, value: (Vec<u8>, usize)) -> DbResult<()> {
    let mut map = get_map()
        .write()
        .map_err(|_| DbError::LockPoisoned)?;

    map.insert(key, value);

    Ok(())
}

/// (Redis: GET) Retrieve a value by its key
pub fn get(key: &Vec<u8>) -> DbResult<Option<(Vec<u8>, usize)>> {
    let map = get_map()
        .read()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(map.get(key).cloned())
}

/// (Redis: DEL) Delete a key-value pair
pub fn del(key: &Vec<u8>) -> DbResult<bool> {
    let mut map = get_map()
        .write()
        .map_err(|_| DbError::LockPoisoned)?;

    Ok(map.remove(key).is_some())
}
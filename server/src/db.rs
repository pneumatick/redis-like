use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static MAP: OnceLock<RwLock<HashMap<Vec<u8>, (Vec<u8>, usize)>>> = OnceLock::new();

fn get_map() -> &'static RwLock<HashMap<Vec<u8>, (Vec<u8>, usize)>> {
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

/// (Redis: SET) Insert or update a key-value pair
pub fn set(key: Vec<u8>, value: (Vec<u8>, usize)) {
    let map = get_map();
    if let Ok(mut guard) = map.write() {
        guard.insert(key, value);
    }
}

/// (Redis: GET) Retrieve a value by its key
pub fn get(key: &Vec<u8>) -> Option<(Vec<u8>, usize)> {
    let map = get_map();
    if let Ok(guard) = map.read() {
        guard.get(key).cloned()
    }
    else {
        None
    }
}

/// (Redis: DEL) Delete a key-value pair
pub fn del(key: &Vec<u8>) -> bool {
    let map = get_map();
    if let Ok(mut guard) = map.write() {
        guard.remove(key).is_some()
    }
    else {
        false
    }
}

use std::collections::HashMap;

pub trait StateMachine {
    type Error;

    fn apply(&mut self, command: &str) -> Result<(), Self::Error>;
    fn get_state(&self) -> Result<String, Self::Error>;
}
pub struct KeyValueStore {
    store: HashMap<String, String>,
}

impl KeyValueStore {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    pub fn get_key(&self, key: &str) -> Option<&String> {
        self.store.get(key)
    }

}

impl StateMachine for KeyValueStore {
    type Error = String;

    fn apply(&mut self, command: &str) -> Result<(), Self::Error> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.len() < 2 {
            return Err("Invalid command".to_string());
        }

        match parts[0] {
            "SET" => {
                if parts.len() != 3 {
                    return Err("SET requires key and value".to_string());
                }
                self.store.insert(parts[1].to_string(), parts[2].to_string());
                Ok(())
            }
            "GET" => {
                if parts.len() != 2 {
                    return Err("GET requires key".to_string());
                }
                
                Ok(())
            }
            _ => Err("Unknown command".to_string()),
        }
    }

    fn get_state(&self) -> Result<String, Self::Error> {
        Ok(format!("KeyValueStore with {} entries", self.store.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_value_store() {
        let mut store = KeyValueStore::new();
        store.apply("SET key1 value1").unwrap();

    }

    #[test]
    fn test_key_value_store_get() {
        let mut store = KeyValueStore::new();
        store.apply("SET key1 value1").unwrap();
        let value = store.get_key("key1").unwrap();
        assert_eq!(value, "value1");
    }

    #[test]
    fn test_multiple_commands() {
        let mut store = KeyValueStore::new();
        store.apply("SET key1 value1").unwrap();
        store.apply("SET key2 value2").unwrap();
        let value = store.get_key("key1").unwrap();
        assert_eq!(value, "value1");
        let value = store.get_key("key2").unwrap();
        assert_eq!(value, "value2");
    }

    #[test]
    fn update_key_value() {
        let mut store = KeyValueStore::new();
        store.apply("SET key1 value1").unwrap();
        let value = store.get_key("key1").unwrap();
        assert_eq!(value, "value1");
        store.apply("SET key1 value2").unwrap();
        let value = store.get_key("key1").unwrap();
        assert_eq!(value, "value2");
    }
    
}

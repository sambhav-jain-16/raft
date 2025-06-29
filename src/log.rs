use crate::types::{LogIndex, Term};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub term: Term,
    pub index: LogIndex,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    pub entries: Vec<LogEntry>,
    pub last_index: LogIndex,
}

impl Log {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_index: 0,
        }
    }

    pub fn append(&mut self, entry: LogEntry) -> Result<(), String> {
        if entry.index != self.last_index + 1 {
            return Err(format!("Invalid log index: {}", entry.index));
        }

        self.last_index = entry.index;
        self.entries.push(entry);
        
        Ok(())
    }

    pub fn get(&self, index: LogIndex) -> Option<&LogEntry> {
        if index > self.last_index || index < 1 {
            return None;
        }
        self.entries.get((index - 1) as usize)
    }

    pub fn last_index(&self) -> LogIndex {
        self.last_index
    }

    pub fn truncate(&mut self, index: LogIndex) {
        if index > self.last_index || index < 1 {
            return;
        }
        self.entries.truncate((index - 1) as usize);
        self.last_index = index-1;
    }

    pub fn get_term(&self, index: LogIndex) -> Option<Term> {
        let entry = self.get(index)?;
        Some(entry.term)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

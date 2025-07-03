use crate::log::LogEntry;
use crate::types::{Term, NodeId};


pub enum AppEvent {
    EntryCommitted(LogEntry),
    LeaderChanged(Option<NodeId>),
    TermChanged(Term),
}
use std::collections::{HashMap, HashSet};
use crate::types::{NodeId, Term, LogIndex, NodeState};
use crate::log::Log;
use crate::state_machine::StateMachine;
use crate::network::Network;
use crate::timer::Timer;
use crate::messages::RaftMessage;
use std::time::Instant;

pub struct RaftNode {
    // Identity
    node_id: NodeId,
    term: Term,
    state: NodeState,
    voted_for: Option<NodeId>,

    // Components
    log: Log,
    state_machine: Box<dyn StateMachine>,
    network: Network,
    election_timer: Timer,
    heartbeat_timer: Timer,

    // leader state (only for leader)
    next_index: HashMap<NodeId, LogIndex>,
    match_index: HashMap<NodeId, LogIndex>,

    // Log state
    commit_index: LogIndex,
    last_applied: LogIndex,

    // Minimal additions
    cluster_nodes: Vec<NodeId>,           // All nodes
    votes_received: HashSet<NodeId>,      // Election votes
    current_leader: Option<NodeId>,       // Known leader
}

impl RaftNode {
    pub fn new(
        node_id: NodeId,
        log: Log,
        state_machine: Box<dyn StateMachine>,
        network: Network,
        cluster_nodes: Vec<NodeId>,
    ) -> Self {
        let election_timer = Timer::new_election_timer();
        let heartbeat_timer = Timer::new_heartbeat_timer();

        let mut next_index = HashMap::new();
        let mut match_index = HashMap::new();

        let last_log_index = log.last_index();

        for &node in &cluster_nodes {
            next_index.insert(node, last_log_index + 1);
            match_index.insert(node, 0);
        }

        let mut node =Self {
            node_id,
            term: 0,
            state: NodeState::Follower,
            voted_for: None,
            log,
            state_machine,
            network,
            election_timer,
            heartbeat_timer,
            next_index,
            match_index,
            commit_index: 0,
            last_applied: 0,
            cluster_nodes,
            votes_received: HashSet::new(),
            current_leader: None,
        };

        node.election_timer.start();
        node
    }

    pub async fn start(&mut self) -> Result<(), String> {
        self.run_event_loop().await
    }

    async fn run_event_loop(&mut self) -> Result<(), String> {
        loop {
            // Placeholder for now
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn become_follower(&mut self, new_term: Term) {
        if new_term > self.term {
            self.term = new_term;
            self.voted_for = None;
        }
        self.state = NodeState::Follower;
        self.election_timer.reset();
        self.votes_received.clear();
        self.current_leader = None;
        self.heartbeat_timer.stop();
    }

    fn become_candidate(&mut self) {
        self.term += 1;
        self.state = NodeState::Candidate;
        self.voted_for = Some(self.node_id);  // Vote for self
        self.current_leader = None;
        self.votes_received.clear();
        self.votes_received.insert(self.node_id);  // Count own vote
        
        // Reset timers
        self.election_timer.reset();
        self.heartbeat_timer.stop();
    }

    fn become_leader(&mut self) {
        self.state = NodeState::Leader;
        self.current_leader = Some(self.node_id);
        
        // Initialize leader state
        let last_log_index = self.log.last_index();
        for &node in &self.cluster_nodes {
            self.next_index.insert(node, last_log_index + 1);
            self.match_index.insert(node, 0);
        }
        
        // Start heartbeat timer
        self.heartbeat_timer.start();
        self.election_timer.stop();
    }
}
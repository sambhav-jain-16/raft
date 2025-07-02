use std::collections::{HashMap, HashSet};
use crate::types::{NodeId, Term, LogIndex, NodeState};
use crate::log::Log;
use crate::state_machine::StateMachine;
use crate::network::Network;
use crate::timer::{Timer, TimerType};
use crate::messages::{AppendEntries, AppendEntriesResponse, RaftMessage, RequestVote, RequestVoteResponse};
use std::time::Instant;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// Events for the Raft event loop
pub enum RaftEvent {
    TimerExpired(TimerType),
    Message(RaftMessage),
    ClientCommand(String),
}

pub struct RaftNode {
    // Identity
    node_id: NodeId,
    term: Arc<Mutex<Term>>,
    state: Arc<Mutex<NodeState>>,
    voted_for: Arc<Mutex<Option<NodeId>>>,

    // Components
    log: Arc<Mutex<Log>>,
    state_machine: Arc<Mutex<Box<dyn StateMachine<Error = String> + Send>>>,
    network: Arc<Mutex<Network>>,
    election_timer: Arc<Mutex<Timer>>,
    heartbeat_timer: Arc<Mutex<Timer>>,

    // leader state (only for leader)
    next_index: Arc<Mutex<HashMap<NodeId, LogIndex>>>,
    match_index: Arc<Mutex<HashMap<NodeId, LogIndex>>>,

    // Log state
    commit_index: Arc<Mutex<LogIndex>>,
    last_applied: Arc<Mutex<LogIndex>>,

    // Minimal additions
    cluster_nodes: Vec<NodeId>,           // All nodes
    votes_received: Arc<Mutex<HashSet<NodeId>>>,      // Election votes
    current_leader: Arc<Mutex<Option<NodeId>>>,       // Known leader

    // Channel for event-driven communication
    event_sender: mpsc::Sender<RaftEvent>,
    event_receiver: Arc<Mutex<mpsc::Receiver<RaftEvent>>>,
}

impl RaftNode {
    pub fn new(
        node_id: NodeId,
        log: Log,
        state_machine: Box<dyn StateMachine<Error = String> + Send>,
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

        let (event_sender, event_receiver) = mpsc::channel(100);

        let mut node = Self {
            node_id,
            term: Arc::new(Mutex::new(0)),
            state: Arc::new(Mutex::new(NodeState::Follower)),
            voted_for: Arc::new(Mutex::new(None)),
            log: Arc::new(Mutex::new(log)),
            state_machine: Arc::new(Mutex::new(state_machine)),
            network: Arc::new(Mutex::new(network)),
            election_timer: Arc::new(Mutex::new(election_timer)),
            heartbeat_timer: Arc::new(Mutex::new(heartbeat_timer)),
            next_index: Arc::new(Mutex::new(next_index)),
            match_index: Arc::new(Mutex::new(match_index)),
            commit_index: Arc::new(Mutex::new(0)),
            last_applied: Arc::new(Mutex::new(0)),
            cluster_nodes,
            votes_received: Arc::new(Mutex::new(HashSet::new())),
            current_leader: Arc::new(Mutex::new(None)),
            event_sender,
            event_receiver: Arc::new(Mutex::new(event_receiver)),
        };
        node.election_timer.lock().unwrap().start();
        node
    }

    pub fn event_sender(&self) -> mpsc::Sender<RaftEvent> {
        self.event_sender.clone()
    }

    pub async fn start(&mut self) -> Result<(), String> {
        self.run_event_loop().await
    }

    async fn run_event_loop(&mut self) -> Result<(), String> {
        loop {
            let event = {
                let mut receiver = self.event_receiver.lock().unwrap();
                receiver.recv().await   
            };
            if let Some(event) = event {
                match event {
                    RaftEvent::TimerExpired(timer_type) => {
                        // Handle timer expiration
                        self.handle_timer_expired(timer_type).await?;
                    }
                    RaftEvent::Message(msg) => {
                        // Handle Raft protocol message
                        self.handle_message(msg).await?;
                    }
                    RaftEvent::ClientCommand(cmd) => {
                        // Handle client command
                        return Ok(());
                    }
                }
            }
        }
    }

    fn become_follower(&mut self, new_term: Term) {
        if new_term > *self.term.lock().unwrap() {
            *self.term.lock().unwrap() = new_term;
            *self.voted_for.lock().unwrap() = None;
        }
        *self.state.lock().unwrap() = NodeState::Follower;
        self.election_timer.lock().unwrap().reset();
        self.votes_received.lock().unwrap().clear();
        *self.current_leader.lock().unwrap() = None;
        self.heartbeat_timer.lock().unwrap().stop();
    }

    fn become_candidate(&mut self) {
        *self.term.lock().unwrap() = *self.term.lock().unwrap() + 1;
        *self.state.lock().unwrap() = NodeState::Candidate;
        *self.voted_for.lock().unwrap() = Some(self.node_id);  // Vote for self
        *self.current_leader.lock().unwrap() = None;
        self.votes_received.lock().unwrap().clear();
        self.votes_received.lock().unwrap().insert(self.node_id);  // Count own vote
        
        // Reset timers
        self.election_timer.lock().unwrap().reset();
        self.heartbeat_timer.lock().unwrap().stop();
    }

    fn become_leader(&mut self) {
        *self.state.lock().unwrap() = NodeState::Leader;
        *self.current_leader.lock().unwrap() = Some(self.node_id);
        
        // Initialize leader state
        let last_log_index = self.log.lock().unwrap().last_index();
        for &node in &self.cluster_nodes {
            self.next_index.lock().unwrap().insert(node, last_log_index + 1);
            self.match_index.lock().unwrap().insert(node, 0);
        }
        
        // Start heartbeat timer
        self.heartbeat_timer.lock().unwrap().start();
        self.election_timer.lock().unwrap().stop();
    }

    async fn handle_timer_expired(&mut self, timer_type: TimerType) -> Result<(), String> {

        match timer_type {
            TimerType::Election => {
                self.start_election().await?;
            }
            TimerType::Heartbeat => {
                self.send_heartbeat().await?;
            }
        }
        Ok(())
    }

    async fn start_election(&mut self) -> Result<(), String> {
        self.become_candidate();
        self.request_votes().await?;
        Ok(())
    }

    async fn send_heartbeat(&mut self) -> Result<(), String> {
         
        let state = self.state.lock().unwrap().clone();
        if state != NodeState::Leader {
            return Ok(());
        }

        let term = *self.term.lock().unwrap();
        let message = RaftMessage::AppendEntries(AppendEntries{
            term,
            leader_id: self.node_id,
            prev_log_index: 0,
            prev_log_term: 0,
            entries: vec![],
            leader_commit: *self.commit_index.lock().unwrap(),
        });

        let results = self.network.lock().unwrap().broadcast(message).await;
        for result in results {
            if let Err(e) = result {
                eprintln!("Failed to send heartbeat to {}", e);
            }
        }

        self.heartbeat_timer.lock().unwrap().reset();
        Ok(())
    }


    async fn request_votes(&mut self) -> Result<(), String> {
        let term = *self.term.lock().unwrap();
        let last_log_index = self.log.lock().unwrap().last_index();
        let last_log_term = self.log.lock().unwrap().get_term(last_log_index).unwrap_or(0);

        let message = RaftMessage::RequestVote(RequestVote{
            term,
            candidate_id: self.node_id,
            last_log_index,
            last_log_term,
        });

        for &node in &self.cluster_nodes {
            if node != self.node_id {
                self.network.lock().unwrap().send_message(node, message.clone()).await?
                    
            }
        }
        Ok(())
    }

    async fn handle_message(&mut self, msg: RaftMessage) -> Result<(), String> {
        match msg {
            RaftMessage::RequestVote(rv) => {
                self.handle_request_vote(rv).await
            }
            RaftMessage::RequestVoteResponse(rvr) => {
                self.handle_request_vote_response(rvr).await
            }
            RaftMessage::AppendEntries(ae) => {
                self.handle_append_entries(ae).await
                // Ok(())
            }
            RaftMessage::AppendEntriesResponse(aer) => {
                // self.handle_append_entries_response(aer).await
                Ok(())
            }
            RaftMessage::ClientCommand(cmd) => {
                // You can call handle_client_command here if you want
                // self.handle_client_command(cmd.command).await
                Ok(())
            }
        }
    }


    async fn handle_request_vote(&mut self, rv: RequestVote) -> Result<(), String> {
        // Get all values we need first
        let current_term = *self.term.lock().unwrap();
        let current_voted_for = *self.voted_for.lock().unwrap();
        let last_log_index = self.log.lock().unwrap().last_index();
        let last_log_term = self.log.lock().unwrap().get_term(last_log_index).unwrap_or(0);
    
        let mut vote_granted = false;
        let mut new_term = current_term;
        let mut new_voted_for = current_voted_for;
    
        if rv.term >= current_term {
            new_term = rv.term;
            new_voted_for = None;
            self.become_follower(rv.term);
        }
    
        if new_voted_for.is_none() || new_voted_for == Some(rv.candidate_id) {
            let log_ok = (rv.last_log_term > last_log_term) || (rv.last_log_term == last_log_term && rv.last_log_index >= last_log_index);
            if log_ok {
                new_voted_for = Some(rv.candidate_id);
                vote_granted = true;
                self.election_timer.lock().unwrap().reset();
            }
        }
    
        // Update state if needed
        if new_term != current_term {
            *self.term.lock().unwrap() = new_term;
        }
        if new_voted_for != current_voted_for {
            *self.voted_for.lock().unwrap() = new_voted_for;
        }
    
        let response = RequestVoteResponse{
            term: new_term,
            vote_granted,
            voter_id: self.node_id,
        };
    
        self.network.lock().unwrap().send_message(rv.candidate_id, RaftMessage::RequestVoteResponse(response)).await?;
        Ok(())
    }

    async fn handle_request_vote_response(&mut self, rvr: RequestVoteResponse) -> Result<(), String> {
        let current_state = self.state.lock().unwrap().clone();
        if current_state != NodeState::Candidate {
            return Ok(());
        }

        let current_term = self.term.lock().unwrap().clone();
        if rvr.term > current_term {
            self.become_follower(rvr.term);
            return Ok(());
        }

        if rvr.term < current_term {
            return Ok(());
        }
        if rvr.vote_granted {
            self.votes_received.lock().unwrap().insert(rvr.voter_id);
        }

        let votes_count = self.votes_received.lock().unwrap().len();
        let majority = (self.cluster_nodes.len() / 2) + 1;
        if votes_count >= majority {
            self.become_leader();
        }

        Ok(())
    }

    async fn handle_append_entries(&mut self, ae: AppendEntries) -> Result<(), String> {
        let current_term = self.term.lock().unwrap().clone();

        // If the term is lower than the current term, ignore the message
        if ae.term < current_term {
            return Ok(());
        }

        // If the term is higher than the current term, become a follower
        if ae.term > current_term {
            self.become_follower(ae.term);
            return Ok(());
        }

        self.election_timer.lock().unwrap().reset();

        let mut log = self.log.lock().unwrap();
        let last_log_index = log.last_index();
        if last_log_index < ae.prev_log_index {
            return Ok(());
        }

        if ae.prev_log_index > 0 {
            let prev_log_term = log.get_term(ae.prev_log_index).unwrap_or(0);
            if prev_log_term != ae.prev_log_term {
                self.network.lock().unwrap().send_message(ae.leader_id, RaftMessage::AppendEntriesResponse(AppendEntriesResponse{
                    term: current_term,
                    success: false,
                })).await?;
                return Ok(());
            }
        }
        for entry in ae.entries {
            log.append(entry).unwrap();
        }

        let new_commit_index = std::cmp::min(ae.leader_commit, log.last_index());
        *self.commit_index.lock().unwrap() = new_commit_index;

        self.network.lock().unwrap().send_message(ae.leader_id, RaftMessage::AppendEntriesResponse(AppendEntriesResponse{
            term: current_term,
            success: true,
        })).await?;

        Ok(())

    }

}
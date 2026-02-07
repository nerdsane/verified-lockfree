```rust
/// Raft Consensus - Lock-free implementation using atomics and crossbeam-epoch
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicBool, Ordering};
use std::sync::Arc;
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared, Guard};
use std::collections::HashMap;

pub type NodeId = u64;
pub type Term = u64;
pub type LogIndex = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeState {
    Follower = 0,
    Candidate = 1,
    Leader = 2,
}

impl From<u8> for NodeState {
    fn from(v: u8) -> Self {
        match v {
            0 => NodeState::Follower,
            1 => NodeState::Candidate,
            2 => NodeState::Leader,
            _ => NodeState::Follower,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RaftError {
    NotLeader,
    NotCandidate,
    StaleTerm,
    NodeNotFound,
    LogInconsistency,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub index: LogIndex,
    pub term: Term,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Message {
    RequestVote {
        term: Term,
        candidate_id: NodeId,
        last_log_index: LogIndex,
        last_log_term: Term,
    },
    RequestVoteResponse {
        term: Term,
        voter_id: NodeId,
        vote_granted: bool,
    },
    AppendEntries {
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    },
    AppendEntriesResponse {
        term: Term,
        follower_id: NodeId,
        success: bool,
        match_index: LogIndex,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Ready {
    pub messages: Vec<(NodeId, Message)>,
    pub committed_entries: Vec<LogEntry>,
    pub should_persist: bool,
}

#[derive(Clone)]
struct LogState {
    entries: Vec<LogEntry>,
    commit_index: LogIndex,
    last_applied: LogIndex,
}

#[derive(Clone)]
struct LeaderState {
    next_index: HashMap<NodeId, LogIndex>,
    match_index: HashMap<NodeId, LogIndex>,
}

#[derive(Clone)]
struct ElectionState {
    votes_received: Vec<NodeId>,
    voted_for: Option<NodeId>,
}

#[derive(Clone)]
struct NodeData {
    log_state: LogState,
    leader_state: LeaderState,
    election_state: ElectionState,
    pending_messages: Vec<(NodeId, Message)>,
    pending_committed: Vec<LogEntry>,
}

pub struct RaftNode {
    id: NodeId,
    peers: Vec<NodeId>,
    state: AtomicU8,
    current_term: AtomicU64,
    election_elapsed: AtomicU64,
    heartbeat_elapsed: AtomicU64,
    election_timeout: u64,
    heartbeat_interval: u64,
    data: Atomic<NodeData>,
}

impl RaftNode {
    pub fn new(id: NodeId, peers: Vec<NodeId>) -> Self {
        debug_assert!(!peers.is_empty(), "Must have at least one peer");
        let cluster_size = peers.len() + 1;
        debug_assert!(
            cluster_size % 2 == 1,
            "Cluster size should be odd for clean majority"
        );

        let initial_data = NodeData {
            log_state: LogState {
                entries: Vec::new(),
                commit_index: 0,
                last_applied: 0,
            },
            leader_state: LeaderState {
                next_index: HashMap::new(),
                match_index: HashMap::new(),
            },
            election_state: ElectionState {
                votes_received: Vec::new(),
                voted_for: None,
            },
            pending_messages: Vec::new(),
            pending_committed: Vec::new(),
        };

        RaftNode {
            id,
            peers,
            state: AtomicU8::new(NodeState::Follower as u8),
            current_term: AtomicU64::new(0),
            election_elapsed: AtomicU64::new(0),
            heartbeat_elapsed: AtomicU64::new(0),
            election_timeout: 10,
            heartbeat_interval: 3,
            data: Atomic::new(initial_data),
        }
    }

    pub fn id(&self) -> NodeId {
        self.id
    }

    pub fn state(&self) -> NodeState {
        NodeState::from(self.state.load(Ordering::Acquire))
    }

    pub fn term(&self) -> Term {
        self.current_term.load(Ordering::Acquire)
    }

    pub fn commit_index(&self) -> LogIndex {
        let guard = &epoch::pin();
        let data_ref = self.data.load(Ordering::Acquire, guard);
        if let Some(data) = unsafe { data_ref.as_ref() } {
            data.log_state.commit_index
        } else {
            0
        }
    }

    pub fn log_len(&self) -> u64 {
        let guard = &epoch::pin();
        let data_ref = self.data.load(Ordering::Acquire, guard);
        if let Some(data) = unsafe { data_ref.as_ref() } {
            data.log_state.entries.len() as u64
        } else {
            0
        }
    }

    fn quorum_size(cluster_size: usize) -> usize {
        cluster_size / 2 + 1
    }

    pub fn tick(&self) {
        let guard = &epoch::pin();
        let state = self.state();

        match state {
            NodeState::Leader => {
                let elapsed = self.heartbeat_elapsed.fetch_add(1, Ordering::AcqRel) + 1;
                if elapsed >= self.heartbeat_interval {
                    self.heartbeat_elapsed.store(0, Ordering::Release);
                    self.send_heartbeats(guard);
                }
            }
            NodeState::Follower | NodeState::Candidate => {
                let elapsed = self.election_elapsed.fetch_add(1, Ordering::AcqRel) + 1;
                if elapsed >= self.election_timeout {
                    self.start_election(guard);
                }
            }
        }
    }

    pub fn propose(&self, data: Vec<u8>) -> Result<u64, RaftError> {
        if self.state() != NodeState::Leader {
            return Err(RaftError::NotLeader);
        }

        let guard = &epoch::pin();
        let term = self.term();
        
        loop {
            let data_ref = self.data.load(Ordering::Acquire, guard);
            let Some(current_data) = unsafe { data_ref.as_ref() } else {
                return Err(RaftError::NotLeader);
            };

            let mut new_data = current_data.clone();
            let index = new_data.log_state.entries.len() as u64 + 1;
            let entry = LogEntry {
                index,
                term,
                data: data.clone(),
            };
            new_data.log_state.entries.push(entry);
            new_data.leader_state.match_index.insert(self.id, index);

            let owned = Owned::new(new_data);
            match self.data.compare_exchange(data_ref, owned, Ordering::AcqRel, Ordering::Acquire, guard) {
                Ok(_) => {
                    self.send_append_entries(guard);
                    return Ok(index);
                }
                Err(_) => continue,
            }
        }
    }

    pub fn step(&self, message: Message) -> Result<(), RaftError> {
        let guard = &epoch::pin();
        
        match message {
            Message::RequestVote { term, candidate_id, last_log_index, last_log_term } => {
                self.handle_request_vote(guard, term, candidate_id, last_log_index, last_log_term);
            }
            Message::RequestVoteResponse { term, voter_id, vote_granted } => {
                self.handle_vote_response(guard, term, voter_id, vote_granted);
            }
            Message::AppendEntries { term, leader_id, prev_log_index, prev_log_term, entries, leader_commit } => {
                self.handle_append_entries(guard, term, leader_id, prev_log_index, prev_log_term, entries, leader_commit);
            }
            Message::AppendEntriesResponse { term, follower_id, success, match_index } => {
                self.handle_append_entries_response(guard, term, follower_id, success, match_index);
            }
        }

        Ok(())
    }

    pub fn ready(&self) -> Ready {
        let guard = &epoch::pin();
        let data_ref = self.data.load(Ordering::Acquire, guard);
        
        if let Some(data) = unsafe { data_ref.as_ref() } {
            Ready {
                messages: data.pending_messages.clone(),
                committed_entries: data.pending_committed.clone(),
                should_persist: !data.pending_messages.is_empty() || !data.pending_committed.is_empty(),
            }
        } else {
            Ready::default()
        }
    }

    pub fn advance(&self, _ready: Ready) {
        let guard = &epoch::pin();
        
        loop {
            let data_ref = self.data.load(Ordering::Acquire, guard);
            let Some(current_data) = unsafe { data_ref.as_ref() } else {
                return;
            };

            let mut new_data = current_data.clone();
            new_data.pending_messages.clear();
            new_data.pending_committed.clear();

            let owned = Owned::new(new_data);
            match self.data.compare_exchange(data_ref, owned, Ordering::AcqRel, Ordering::Acquire, guard) {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }

    fn last_log_info(&self, data: &NodeData) -> (LogIndex, Term) {
        data.log_state.entries.last()
            .map(|e| (e.index, e.term))
            .unwrap_or((0, 0))
    }

    fn start_election(&self, guard: &Guard) {
        let new_term = self.current_term.fetch_add(1, Ordering::AcqRel) + 1;
        self.state.store(NodeState::Candidate as u8, Ordering::Release);
        self.election_elapsed.store(0, Ordering::Release);

        loop {
            let data_ref = self.data.load(Ordering::Acquire, guard);
            let Some(current_data) = unsafe { data_ref.as_ref() } else {
                return;
            };

            let mut new_data = current_data.clone();
            new_data.election_state.voted_for = Some(self.id);
            new_data.election_state.votes_received = vec![self.id];
            
            let (last_log_index, last_log_term) = self.last_log_info(&new_data);

            for &peer in &self.peers {
                new_data.pending_messages.push((
                    peer,
                    Message::RequestVote {
                        term: new_term,
                        candidate_id: self.id,
                        last_log_index,
                        last_log_term,
                    },
                ));
            }

            let owned = Owned::new(new_data);
            match self.data.compare_exchange(data_ref, owned, Ordering::AcqRel, Ordering::Acquire, guard) {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }

    fn send_heartbeats(&self, guard: &Guard) {
        self.send_append_entries(guard);
    }

    fn send_append_entries(&self, guard: &Guard) {
        let term = self.term();
        
        loop {
            let data_ref = self.data.load(Ordering::Acquire, guard);
            let Some(current_data) = unsafe { data_ref.as_ref() } else {
                return;
            };

            let mut new_data = current_data.clone();

            for &peer in &self.peers {
                let next_idx = new_data.leader_state.next_index.get(&peer).copied().unwrap_or(1);
                let prev_log_index = next_idx.saturating_sub(1);
                let prev_log_term = if prev_log_index == 0 {
                    0
                } else {
                    new_data.log_state.entries
                        .get((prev_log_index - 1) as usize)
                        .map(|e| e.term)
                        .unwrap_or(0)
                };

                let entries: Vec<LogEntry> = new_data.log_state.entries
                    .iter()
                    .filter(|e| e.index >= next_idx)
                    .cloned()
                    .collect();

                new_data.pending_messages.push((
                    peer,
                    Message::AppendEntries {
                        term,
                        leader_id: self.id,
                        prev_log_index,
                        prev_log_term,
                        entries,
                        leader_commit: new_data.log_state.commit_index,
                    },
                ));
            }

            let owned = Owned::new(new_data);
            match self.data.compare_exchange(data_ref, owned, Ordering::AcqRel, Ordering::Acquire, guard) {
                Ok(_) => return,
                Err(_) => continue,
            }
        }
    }

    fn handle_request_vote(&self, guard: &Guard, term: Term, candidate_id: NodeId, last_log_index: LogIndex, last_log_term: Term) {
        let current_term = self.term();
        
        if term > current_term {
            self.current_term.store(term, Ordering::Release);
            self.state.store(NodeState::Follower as u8, Ordering::Release);
        }

        loop {
            let data_ref = self.data.load(Ordering::Acquire, guard);
            let Some(current_data) = unsafe { data_ref.as_ref() } else {
                return;
            };

            let mut new_data = current_data.clone();
            
            if term > current_term {
                new_data.election_state.voted_for = None;
                new_data.election_state.votes_received.clear();
            }

            let (my_last_index, my_last_term) = self.last_log_info(&new_data);
            let log_ok = last_log_term > my_last_term || 
                (last_log_term == my_last_term && last_log_index >= my_last_index);

            let vote_granted = term >= self.term() &&
                (new_data.election_state.voted_for.is_none() || new_data.election_state.voted_for == Some(candidate_id)) &&
                log_ok;

            if vote_granted {
                new_data.election_state.voted_for = Some(candidate_id);
                self.
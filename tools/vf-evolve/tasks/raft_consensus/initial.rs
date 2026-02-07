/// Raft Consensus - Initial Seed (Mutex-based with log replication)
/// ShinkaEvolve will evolve election + log replication + commit safety.
use std::collections::HashMap;
use std::sync::Mutex;

pub type NodeId = u64;
pub type Term = u64;
pub type LogIndex = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Follower,
    Candidate,
    Leader,
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

struct NodeInner {
    id: NodeId,
    peers: Vec<NodeId>,
    state: NodeState,
    current_term: Term,
    voted_for: Option<NodeId>,
    log: Vec<LogEntry>,
    commit_index: LogIndex,
    last_applied: LogIndex,
    // Leader state
    next_index: HashMap<NodeId, LogIndex>,
    match_index: HashMap<NodeId, LogIndex>,
    // Election state
    votes_received: Vec<NodeId>,
    // Timers (tick counts)
    election_elapsed: u64,
    heartbeat_elapsed: u64,
    election_timeout: u64,
    heartbeat_interval: u64,
    // Output buffer
    pending_messages: Vec<(NodeId, Message)>,
    pending_committed: Vec<LogEntry>,
}

pub struct RaftNode {
    inner: Mutex<NodeInner>,
}

impl RaftNode {
    /// Create a new Raft node.
    pub fn new(id: NodeId, peers: Vec<NodeId>) -> Self {
        debug_assert!(!peers.is_empty(), "Must have at least one peer");
        let cluster_size = peers.len() + 1;
        debug_assert!(
            cluster_size % 2 == 1,
            "Cluster size should be odd for clean majority"
        );

        RaftNode {
            inner: Mutex::new(NodeInner {
                id,
                peers,
                state: NodeState::Follower,
                current_term: 0,
                voted_for: None,
                log: Vec::new(),
                commit_index: 0,
                last_applied: 0,
                next_index: HashMap::new(),
                match_index: HashMap::new(),
                votes_received: Vec::new(),
                election_elapsed: 0,
                heartbeat_elapsed: 0,
                election_timeout: 10, // ticks
                heartbeat_interval: 3,
                pending_messages: Vec::new(),
                pending_committed: Vec::new(),
            }),
        }
    }

    pub fn id(&self) -> NodeId {
        self.inner.lock().unwrap().id
    }

    pub fn state(&self) -> NodeState {
        self.inner.lock().unwrap().state
    }

    pub fn term(&self) -> Term {
        self.inner.lock().unwrap().current_term
    }

    pub fn commit_index(&self) -> LogIndex {
        self.inner.lock().unwrap().commit_index
    }

    pub fn log_len(&self) -> u64 {
        self.inner.lock().unwrap().log.len() as u64
    }

    fn quorum_size(cluster_size: usize) -> usize {
        cluster_size / 2 + 1
    }

    /// Advance logical clock by one tick.
    pub fn tick(&self) {
        let mut inner = self.inner.lock().unwrap();

        match inner.state {
            NodeState::Leader => {
                inner.heartbeat_elapsed += 1;
                if inner.heartbeat_elapsed >= inner.heartbeat_interval {
                    inner.heartbeat_elapsed = 0;
                    Self::send_heartbeats(&mut inner);
                }
            }
            NodeState::Follower | NodeState::Candidate => {
                inner.election_elapsed += 1;
                if inner.election_elapsed >= inner.election_timeout {
                    Self::start_election(&mut inner);
                }
            }
        }
    }

    /// Propose a new entry (leader only).
    pub fn propose(&self, data: Vec<u8>) -> Result<u64, RaftError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.state != NodeState::Leader {
            return Err(RaftError::NotLeader);
        }

        let index = inner.log.len() as u64 + 1;
        let entry = LogEntry {
            index,
            term: inner.current_term,
            data,
        };
        inner.log.push(entry);

        // Update own match_index
        inner.match_index.insert(inner.id, index);

        // Send AppendEntries to all peers
        Self::send_append_entries(&mut inner);

        Ok(index)
    }

    /// Process an incoming message.
    pub fn step(&self, message: Message) -> Result<(), RaftError> {
        let mut inner = self.inner.lock().unwrap();

        match message {
            Message::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                Self::handle_request_vote(
                    &mut inner,
                    term,
                    candidate_id,
                    last_log_index,
                    last_log_term,
                );
            }
            Message::RequestVoteResponse {
                term,
                voter_id,
                vote_granted,
            } => {
                Self::handle_vote_response(&mut inner, term, voter_id, vote_granted);
            }
            Message::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                Self::handle_append_entries(
                    &mut inner,
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                );
            }
            Message::AppendEntriesResponse {
                term,
                follower_id,
                success,
                match_index,
            } => {
                Self::handle_append_entries_response(
                    &mut inner, term, follower_id, success, match_index,
                );
            }
        }

        Ok(())
    }

    /// Collect ready output.
    pub fn ready(&self) -> Ready {
        let inner = self.inner.lock().unwrap();
        Ready {
            messages: inner.pending_messages.clone(),
            committed_entries: inner.pending_committed.clone(),
            should_persist: !inner.pending_messages.is_empty()
                || !inner.pending_committed.is_empty(),
        }
    }

    /// Acknowledge ready output processed.
    pub fn advance(&self, _ready: Ready) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending_messages.clear();
        inner.pending_committed.clear();
    }

    // -- Internal helpers --

    fn last_log_info(inner: &NodeInner) -> (LogIndex, Term) {
        inner
            .log
            .last()
            .map(|e| (e.index, e.term))
            .unwrap_or((0, 0))
    }

    fn start_election(inner: &mut NodeInner) {
        inner.current_term += 1;
        inner.state = NodeState::Candidate;
        inner.voted_for = Some(inner.id);
        inner.votes_received = vec![inner.id];
        inner.election_elapsed = 0;

        let (last_log_index, last_log_term) = Self::last_log_info(inner);

        for &peer in &inner.peers.clone() {
            inner.pending_messages.push((
                peer,
                Message::RequestVote {
                    term: inner.current_term,
                    candidate_id: inner.id,
                    last_log_index,
                    last_log_term,
                },
            ));
        }
    }

    fn send_heartbeats(inner: &mut NodeInner) {
        for &peer in &inner.peers.clone() {
            let next_idx = inner.next_index.get(&peer).copied().unwrap_or(1);
            let prev_log_index = next_idx.saturating_sub(1);
            let prev_log_term = if prev_log_index == 0 {
                0
            } else {
                inner
                    .log
                    .get((prev_log_index - 1) as usize)
                    .map(|e| e.term)
                    .unwrap_or(0)
            };

            let entries: Vec<LogEntry> = inner
                .log
                .iter()
                .filter(|e| e.index >= next_idx)
                .cloned()
                .collect();

            inner.pending_messages.push((
                peer,
                Message::AppendEntries {
                    term: inner.current_term,
                    leader_id: inner.id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit: inner.commit_index,
                },
            ));
        }
    }

    fn send_append_entries(inner: &mut NodeInner) {
        Self::send_heartbeats(inner);
    }

    fn handle_request_vote(
        inner: &mut NodeInner,
        term: Term,
        candidate_id: NodeId,
        last_log_index: LogIndex,
        last_log_term: Term,
    ) {
        if term > inner.current_term {
            inner.current_term = term;
            inner.state = NodeState::Follower;
            inner.voted_for = None;
            inner.votes_received.clear();
        }

        let (my_last_index, my_last_term) = Self::last_log_info(inner);
        let log_ok = last_log_term > my_last_term
            || (last_log_term == my_last_term && last_log_index >= my_last_index);

        let vote_granted = term >= inner.current_term
            && (inner.voted_for.is_none() || inner.voted_for == Some(candidate_id))
            && log_ok;

        if vote_granted {
            inner.voted_for = Some(candidate_id);
            inner.election_elapsed = 0; // Reset election timer
        }

        inner.pending_messages.push((
            candidate_id,
            Message::RequestVoteResponse {
                term: inner.current_term,
                voter_id: inner.id,
                vote_granted,
            },
        ));
    }

    fn handle_vote_response(
        inner: &mut NodeInner,
        term: Term,
        voter_id: NodeId,
        vote_granted: bool,
    ) {
        if term > inner.current_term {
            inner.current_term = term;
            inner.state = NodeState::Follower;
            inner.voted_for = None;
            inner.votes_received.clear();
            return;
        }

        if inner.state != NodeState::Candidate || term != inner.current_term {
            return;
        }

        if vote_granted && !inner.votes_received.contains(&voter_id) {
            inner.votes_received.push(voter_id);
        }

        let cluster_size = inner.peers.len() + 1;
        if inner.votes_received.len() >= Self::quorum_size(cluster_size) {
            Self::become_leader(inner);
        }
    }

    fn become_leader(inner: &mut NodeInner) {
        inner.state = NodeState::Leader;
        inner.heartbeat_elapsed = 0;

        let last_log_idx = inner.log.len() as u64 + 1;
        for &peer in &inner.peers.clone() {
            inner.next_index.insert(peer, last_log_idx);
            inner.match_index.insert(peer, 0);
        }
        inner.match_index.insert(inner.id, inner.log.len() as u64);

        // Send initial heartbeats
        Self::send_heartbeats(inner);
    }

    fn handle_append_entries(
        inner: &mut NodeInner,
        term: Term,
        leader_id: NodeId,
        prev_log_index: LogIndex,
        prev_log_term: Term,
        entries: Vec<LogEntry>,
        leader_commit: LogIndex,
    ) {
        if term > inner.current_term {
            inner.current_term = term;
            inner.voted_for = None;
            inner.votes_received.clear();
        }

        if term < inner.current_term {
            inner.pending_messages.push((
                leader_id,
                Message::AppendEntriesResponse {
                    term: inner.current_term,
                    follower_id: inner.id,
                    success: false,
                    match_index: 0,
                },
            ));
            return;
        }

        inner.state = NodeState::Follower;
        inner.election_elapsed = 0;

        // Check log consistency
        if prev_log_index > 0 {
            let has_entry = inner
                .log
                .get((prev_log_index - 1) as usize)
                .map(|e| e.term == prev_log_term)
                .unwrap_or(false);
            if !has_entry {
                inner.pending_messages.push((
                    leader_id,
                    Message::AppendEntriesResponse {
                        term: inner.current_term,
                        follower_id: inner.id,
                        success: false,
                        match_index: 0,
                    },
                ));
                return;
            }
        }

        // Append entries (overwrite conflicts)
        for entry in &entries {
            let idx = (entry.index - 1) as usize;
            if idx < inner.log.len() {
                if inner.log[idx].term != entry.term {
                    inner.log.truncate(idx);
                    inner.log.push(entry.clone());
                }
            } else {
                inner.log.push(entry.clone());
            }
        }

        // Update commit index
        if leader_commit > inner.commit_index {
            let last_new_idx = entries.last().map(|e| e.index).unwrap_or(inner.log.len() as u64);
            inner.commit_index = leader_commit.min(last_new_idx);
            Self::apply_committed(inner);
        }

        let match_idx = inner.log.len() as u64;
        inner.pending_messages.push((
            leader_id,
            Message::AppendEntriesResponse {
                term: inner.current_term,
                follower_id: inner.id,
                success: true,
                match_index: match_idx,
            },
        ));
    }

    fn handle_append_entries_response(
        inner: &mut NodeInner,
        term: Term,
        follower_id: NodeId,
        success: bool,
        match_index: LogIndex,
    ) {
        if term > inner.current_term {
            inner.current_term = term;
            inner.state = NodeState::Follower;
            inner.voted_for = None;
            inner.votes_received.clear();
            return;
        }

        if inner.state != NodeState::Leader || term != inner.current_term {
            return;
        }

        if success {
            inner.next_index.insert(follower_id, match_index + 1);
            inner.match_index.insert(follower_id, match_index);
            Self::try_advance_commit(inner);
        } else {
            // Decrement next_index and retry
            let next = inner.next_index.get(&follower_id).copied().unwrap_or(1);
            inner.next_index.insert(follower_id, next.saturating_sub(1).max(1));
        }
    }

    fn try_advance_commit(inner: &mut NodeInner) {
        let cluster_size = inner.peers.len() + 1;
        let quorum = Self::quorum_size(cluster_size);

        // Find the highest index replicated on a quorum
        for n in (inner.commit_index + 1..=inner.log.len() as u64).rev() {
            let replicated = inner
                .match_index
                .values()
                .filter(|&&mi| mi >= n)
                .count();

            if replicated >= quorum {
                // Only commit entries from current term (Raft safety)
                if let Some(entry) = inner.log.get((n - 1) as usize) {
                    if entry.term == inner.current_term {
                        inner.commit_index = n;
                        Self::apply_committed(inner);
                        break;
                    }
                }
            }
        }
    }

    fn apply_committed(inner: &mut NodeInner) {
        while inner.last_applied < inner.commit_index {
            inner.last_applied += 1;
            if let Some(entry) = inner.log.get((inner.last_applied - 1) as usize) {
                inner.pending_committed.push(entry.clone());
            }
        }
    }
}

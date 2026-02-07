/// Raft Leader Election - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub type ServerId = u64;
pub type Term = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RaftError {
    NotCandidate,
    AlreadyVoted,
    StaleTerm,
    ClusterTooSmall,
    ServerNotFound,
}

#[derive(Debug, Clone)]
pub struct VoteRequest {
    pub term: Term,
    pub candidate_id: ServerId,
}

#[derive(Debug, Clone)]
pub struct VoteResponse {
    pub term: Term,
    pub vote_granted: bool,
    pub voter_id: ServerId,
}

#[derive(Debug, Clone)]
pub struct Heartbeat {
    pub term: Term,
    pub leader_id: ServerId,
}

struct ServerInner {
    current_term: Term,
    voted_for: Option<ServerId>,
    state: ServerState,
    votes_received: Vec<ServerId>,
}

pub struct RaftElection {
    servers: HashMap<ServerId, Mutex<ServerInner>>,
    cluster_size: usize,
    next_id: AtomicU64,
}

impl RaftElection {
    pub fn new(server_ids: &[ServerId]) -> Self {
        debug_assert!(!server_ids.is_empty(), "Cluster must have at least one server");
        debug_assert!(
            server_ids.len() % 2 == 1,
            "Cluster size should be odd for clean majority"
        );

        let mut servers = HashMap::new();
        for &id in server_ids {
            servers.insert(
                id,
                Mutex::new(ServerInner {
                    current_term: 0,
                    voted_for: None,
                    state: ServerState::Follower,
                    votes_received: Vec::new(),
                }),
            );
        }

        RaftElection {
            cluster_size: servers.len(),
            servers,
            next_id: AtomicU64::new(server_ids.iter().max().copied().unwrap_or(0) + 1),
        }
    }

    pub fn cluster_size(&self) -> usize {
        self.cluster_size
    }

    pub fn quorum_size(&self) -> usize {
        self.cluster_size / 2 + 1
    }

    pub fn get_state(&self, server_id: ServerId) -> Option<ServerState> {
        self.servers
            .get(&server_id)
            .map(|s| s.lock().unwrap().state)
    }

    pub fn get_term(&self, server_id: ServerId) -> Option<Term> {
        self.servers
            .get(&server_id)
            .map(|s| s.lock().unwrap().current_term)
    }

    pub fn get_leader(&self) -> Option<ServerId> {
        for (&id, server) in &self.servers {
            let guard = server.lock().unwrap();
            if guard.state == ServerState::Leader {
                return Some(id);
            }
        }
        None
    }

    /// Simulate a timeout on a server, causing it to start an election.
    /// Returns the VoteRequest that should be broadcast to other servers.
    pub fn timeout(&self, server_id: ServerId) -> Result<VoteRequest, RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        let mut guard = server.lock().unwrap();

        debug_assert!(
            guard.state != ServerState::Leader,
            "Leaders should not time out in normal operation"
        );

        // Increment term and become candidate
        guard.current_term += 1;
        guard.state = ServerState::Candidate;
        guard.voted_for = Some(server_id); // Vote for self
        guard.votes_received = vec![server_id]; // Self-vote

        Ok(VoteRequest {
            term: guard.current_term,
            candidate_id: server_id,
        })
    }

    /// Handle an incoming VoteRequest on a server.
    pub fn handle_vote_request(
        &self,
        server_id: ServerId,
        request: &VoteRequest,
    ) -> Result<VoteResponse, RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        let mut guard = server.lock().unwrap();

        // If the request has a higher term, step down
        if request.term > guard.current_term {
            guard.current_term = request.term;
            guard.state = ServerState::Follower;
            guard.voted_for = None;
            guard.votes_received.clear();
        }

        // Grant vote if: term matches, and we haven't voted or voted for this candidate
        let vote_granted = request.term >= guard.current_term
            && (guard.voted_for.is_none() || guard.voted_for == Some(request.candidate_id));

        if vote_granted {
            guard.voted_for = Some(request.candidate_id);
        }

        Ok(VoteResponse {
            term: guard.current_term,
            vote_granted,
            voter_id: server_id,
        })
    }

    /// Handle a VoteResponse received by a candidate.
    /// Returns Some(true) if the candidate has won the election.
    pub fn handle_vote_response(
        &self,
        candidate_id: ServerId,
        response: &VoteResponse,
    ) -> Result<bool, RaftError> {
        let server = self
            .servers
            .get(&candidate_id)
            .ok_or(RaftError::ServerNotFound)?;

        let mut guard = server.lock().unwrap();

        // If response has higher term, step down
        if response.term > guard.current_term {
            guard.current_term = response.term;
            guard.state = ServerState::Follower;
            guard.voted_for = None;
            guard.votes_received.clear();
            return Ok(false);
        }

        // Only process if still a candidate in the same term
        if guard.state != ServerState::Candidate || response.term != guard.current_term {
            return Ok(false);
        }

        if response.vote_granted {
            if !guard.votes_received.contains(&response.voter_id) {
                guard.votes_received.push(response.voter_id);
            }
        }

        // Check if we have a quorum
        let won = guard.votes_received.len() >= self.quorum_size();
        if won {
            guard.state = ServerState::Leader;
        }

        Ok(won)
    }

    /// Handle a heartbeat received by a server.
    pub fn handle_heartbeat(
        &self,
        server_id: ServerId,
        heartbeat: &Heartbeat,
    ) -> Result<(), RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        let mut guard = server.lock().unwrap();

        if heartbeat.term >= guard.current_term {
            guard.current_term = heartbeat.term;
            guard.state = ServerState::Follower;
            if heartbeat.term > guard.current_term {
                guard.voted_for = None;
            }
            guard.votes_received.clear();
        }

        Ok(())
    }

    /// Create a heartbeat from the leader.
    pub fn create_heartbeat(&self, leader_id: ServerId) -> Result<Heartbeat, RaftError> {
        let server = self
            .servers
            .get(&leader_id)
            .ok_or(RaftError::ServerNotFound)?;

        let guard = server.lock().unwrap();

        if guard.state != ServerState::Leader {
            return Err(RaftError::NotCandidate);
        }

        Ok(Heartbeat {
            term: guard.current_term,
            leader_id,
        })
    }

    /// Run a complete election: server times out, requests votes from all others,
    /// collects responses, and returns whether it became leader.
    pub fn run_election(&self, candidate_id: ServerId) -> Result<bool, RaftError> {
        let vote_request = self.timeout(candidate_id)?;

        let other_servers: Vec<ServerId> = self
            .servers
            .keys()
            .filter(|&&id| id != candidate_id)
            .copied()
            .collect();

        for &other_id in &other_servers {
            let response = self.handle_vote_request(other_id, &vote_request)?;
            let won = self.handle_vote_response(candidate_id, &response)?;
            if won {
                return Ok(true);
            }
        }

        // Check final state
        Ok(self.get_state(candidate_id) == Some(ServerState::Leader))
    }
}

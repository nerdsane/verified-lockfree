/// Raft Leader Election - Lock-Free Implementation
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicPtr, Ordering};
use std::ptr;
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};

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

#[derive(Debug)]
struct ServerInner {
    current_term: AtomicU64,
    voted_for: AtomicU64, // 0 means None, otherwise ServerId
    state: AtomicU64, // 0=Follower, 1=Candidate, 2=Leader
    votes_received: Atomic<Vec<ServerId>>,
}

impl ServerInner {
    fn new() -> Self {
        Self {
            current_term: AtomicU64::new(0),
            voted_for: AtomicU64::new(0),
            state: AtomicU64::new(0),
            votes_received: Atomic::new(Vec::new()),
        }
    }

    fn get_state(&self) -> ServerState {
        match self.state.load(Ordering::Acquire) {
            0 => ServerState::Follower,
            1 => ServerState::Candidate,
            2 => ServerState::Leader,
            _ => ServerState::Follower,
        }
    }

    fn set_state(&self, state: ServerState) {
        let val = match state {
            ServerState::Follower => 0,
            ServerState::Candidate => 1,
            ServerState::Leader => 2,
        };
        self.state.store(val, Ordering::Release);
    }

    fn get_voted_for(&self) -> Option<ServerId> {
        let val = self.voted_for.load(Ordering::Acquire);
        if val == 0 { None } else { Some(val) }
    }

    fn set_voted_for(&self, server_id: Option<ServerId>) {
        let val = server_id.unwrap_or(0);
        self.voted_for.store(val, Ordering::Release);
    }
}

pub struct RaftElection {
    servers: HashMap<ServerId, ServerInner>,
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
            servers.insert(id, ServerInner::new());
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
        self.servers.get(&server_id).map(|s| s.get_state())
    }

    pub fn get_term(&self, server_id: ServerId) -> Option<Term> {
        self.servers
            .get(&server_id)
            .map(|s| s.current_term.load(Ordering::Acquire))
    }

    pub fn get_leader(&self) -> Option<ServerId> {
        for (&id, server) in &self.servers {
            if server.get_state() == ServerState::Leader {
                return Some(id);
            }
        }
        None
    }

    pub fn timeout(&self, server_id: ServerId) -> Result<VoteRequest, RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        loop {
            let current_term = server.current_term.load(Ordering::Acquire);
            let current_state = server.get_state();

            if current_state == ServerState::Leader {
                return Err(RaftError::NotCandidate);
            }

            let new_term = current_term + 1;
            
            // Try to increment term atomically
            if server.current_term.compare_exchange_weak(
                current_term,
                new_term,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).is_ok() {
                // Successfully incremented term, now update state
                server.set_state(ServerState::Candidate);
                server.set_voted_for(Some(server_id));
                
                // Reset votes received
                let guard = epoch::pin();
                let new_votes = Owned::new(vec![server_id]);
                server.votes_received.store(new_votes, Ordering::Release);
                
                return Ok(VoteRequest {
                    term: new_term,
                    candidate_id: server_id,
                });
            }
        }
    }

    pub fn handle_vote_request(
        &self,
        server_id: ServerId,
        request: &VoteRequest,
    ) -> Result<VoteResponse, RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        loop {
            let current_term = server.current_term.load(Ordering::Acquire);
            let voted_for = server.get_voted_for();

            // If request has higher term, update our term
            if request.term > current_term {
                if server.current_term.compare_exchange_weak(
                    current_term,
                    request.term,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ).is_ok() {
                    server.set_state(ServerState::Follower);
                    server.set_voted_for(None);
                    
                    let guard = epoch::pin();
                    let new_votes = Owned::new(Vec::new());
                    server.votes_received.store(new_votes, Ordering::Release);
                    continue;
                }
                continue;
            }

            let final_term = server.current_term.load(Ordering::Acquire);
            let vote_granted = request.term >= final_term
                && (voted_for.is_none() || voted_for == Some(request.candidate_id));

            if vote_granted && request.term == final_term {
                server.set_voted_for(Some(request.candidate_id));
            }

            return Ok(VoteResponse {
                term: final_term,
                vote_granted,
                voter_id: server_id,
            });
        }
    }

    pub fn handle_vote_response(
        &self,
        candidate_id: ServerId,
        response: &VoteResponse,
    ) -> Result<bool, RaftError> {
        let server = self
            .servers
            .get(&candidate_id)
            .ok_or(RaftError::ServerNotFound)?;

        let current_term = server.current_term.load(Ordering::Acquire);

        // If response has higher term, step down
        if response.term > current_term {
            if server.current_term.compare_exchange_weak(
                current_term,
                response.term,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).is_ok() {
                server.set_state(ServerState::Follower);
                server.set_voted_for(None);
                
                let guard = epoch::pin();
                let new_votes = Owned::new(Vec::new());
                server.votes_received.store(new_votes, Ordering::Release);
            }
            return Ok(false);
        }

        // Only process if still a candidate in the same term
        if server.get_state() != ServerState::Candidate || response.term != current_term {
            return Ok(false);
        }

        if response.vote_granted {
            let guard = epoch::pin();
            loop {
                let votes_ptr = server.votes_received.load(Ordering::Acquire, &guard);
                let votes = unsafe { votes_ptr.as_ref() }.unwrap();
                
                if votes.contains(&response.voter_id) {
                    break;
                }

                let mut new_votes = votes.clone();
                new_votes.push(response.voter_id);
                let new_votes_owned = Owned::new(new_votes);
                
                if server.votes_received.compare_exchange_weak(
                    votes_ptr,
                    new_votes_owned,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                    &guard,
                ).is_ok() {
                    unsafe {
                        guard.defer_destroy(votes_ptr);
                    }
                    break;
                }
            }
        }

        // Check if we have a quorum
        let guard = epoch::pin();
        let votes_ptr = server.votes_received.load(Ordering::Acquire, &guard);
        let votes = unsafe { votes_ptr.as_ref() }.unwrap();
        let won = votes.len() >= self.quorum_size();
        
        if won {
            server.set_state(ServerState::Leader);
        }

        Ok(won)
    }

    pub fn handle_heartbeat(
        &self,
        server_id: ServerId,
        heartbeat: &Heartbeat,
    ) -> Result<(), RaftError> {
        let server = self
            .servers
            .get(&server_id)
            .ok_or(RaftError::ServerNotFound)?;

        let current_term = server.current_term.load(Ordering::Acquire);

        if heartbeat.term >= current_term {
            if heartbeat.term > current_term {
                server.current_term.store(heartbeat.term, Ordering::Release);
                server.set_voted_for(None);
            }
            server.set_state(ServerState::Follower);
            
            let guard = epoch::pin();
            let new_votes = Owned::new(Vec::new());
            server.votes_received.store(new_votes, Ordering::Release);
        }

        Ok(())
    }

    pub fn create_heartbeat(&self, leader_id: ServerId) -> Result<Heartbeat, RaftError> {
        let server = self
            .servers
            .get(&leader_id)
            .ok_or(RaftError::ServerNotFound)?;

        if server.get_state() != ServerState::Leader {
            return Err(RaftError::NotCandidate);
        }

        Ok(Heartbeat {
            term: server.current_term.load(Ordering::Acquire),
            leader_id,
        })
    }

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

        Ok(self.get_state(candidate_id) == Some(ServerState::Leader))
    }
}
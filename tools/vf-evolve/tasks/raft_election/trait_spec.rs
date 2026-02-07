/// Trait specification tests for RaftElection
///
/// The evolved code must provide:
///   pub type ServerId = u64;
///   pub type Term = u64;
///   pub enum ServerState { Follower, Candidate, Leader }
///   pub enum RaftError { NotCandidate, AlreadyVoted, StaleTerm, ClusterTooSmall, ServerNotFound }
///   pub struct VoteRequest { pub term: Term, pub candidate_id: ServerId }
///   pub struct VoteResponse { pub term: Term, pub vote_granted: bool, pub voter_id: ServerId }
///   pub struct Heartbeat { pub term: Term, pub leader_id: ServerId }
///   pub struct RaftElection { ... }
///   impl RaftElection {
///       pub fn new(server_ids: &[ServerId]) -> Self;
///       pub fn cluster_size(&self) -> usize;
///       pub fn quorum_size(&self) -> usize;
///       pub fn get_state(&self, server_id: ServerId) -> Option<ServerState>;
///       pub fn get_term(&self, server_id: ServerId) -> Option<Term>;
///       pub fn get_leader(&self) -> Option<ServerId>;
///       pub fn timeout(&self, server_id: ServerId) -> Result<VoteRequest, RaftError>;
///       pub fn handle_vote_request(&self, server_id: ServerId, request: &VoteRequest) -> Result<VoteResponse, RaftError>;
///       pub fn handle_vote_response(&self, candidate_id: ServerId, response: &VoteResponse) -> Result<bool, RaftError>;
///       pub fn handle_heartbeat(&self, server_id: ServerId, heartbeat: &Heartbeat) -> Result<(), RaftError>;
///       pub fn create_heartbeat(&self, leader_id: ServerId) -> Result<Heartbeat, RaftError>;
///       pub fn run_election(&self, candidate_id: ServerId) -> Result<bool, RaftError>;
///   }
///
/// All operations must be safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_initial_state_all_followers() {
        let cluster = RaftElection::new(&[1, 2, 3, 4, 5]);
        for id in 1..=5 {
            assert_eq!(
                cluster.get_state(id),
                Some(ServerState::Follower),
                "All servers must start as followers"
            );
            assert_eq!(
                cluster.get_term(id),
                Some(0),
                "All servers must start at term 0"
            );
        }
        assert_eq!(cluster.get_leader(), None, "No leader initially");
    }

    #[test]
    fn test_quorum_size() {
        assert_eq!(RaftElection::new(&[1, 2, 3]).quorum_size(), 2);
        assert_eq!(RaftElection::new(&[1, 2, 3, 4, 5]).quorum_size(), 3);
        assert_eq!(RaftElection::new(&[1, 2, 3, 4, 5, 6, 7]).quorum_size(), 4);
    }

    #[test]
    fn test_timeout_becomes_candidate() {
        let cluster = RaftElection::new(&[1, 2, 3]);
        let request = cluster.timeout(1).unwrap();

        assert_eq!(
            cluster.get_state(1),
            Some(ServerState::Candidate),
            "Timeout must transition to Candidate"
        );
        assert_eq!(
            cluster.get_term(1),
            Some(1),
            "Timeout must increment term"
        );
        assert_eq!(request.candidate_id, 1);
        assert_eq!(request.term, 1);
    }

    #[test]
    fn test_single_server_election() {
        // A single server should be able to elect itself (quorum of 1)
        let cluster = RaftElection::new(&[1]);
        let won = cluster.run_election(1).unwrap();
        assert!(won, "Single server must win its own election");
        assert_eq!(cluster.get_state(1), Some(ServerState::Leader));
    }

    #[test]
    fn test_three_server_election() {
        let cluster = RaftElection::new(&[1, 2, 3]);
        let won = cluster.run_election(1).unwrap();
        assert!(won, "Server 1 should win in 3-server cluster");
        assert_eq!(cluster.get_state(1), Some(ServerState::Leader));
    }

    #[test]
    fn test_election_safety_one_leader_per_term() {
        let cluster = RaftElection::new(&[1, 2, 3, 4, 5]);

        // Server 1 wins election in term 1
        let won = cluster.run_election(1).unwrap();
        assert!(won);

        let leader_term = cluster.get_term(1).unwrap();

        // Count leaders in same term
        let mut leader_count = 0;
        for id in 1..=5 {
            if cluster.get_state(id) == Some(ServerState::Leader) {
                assert_eq!(
                    cluster.get_term(id).unwrap(),
                    leader_term,
                    "Leader must be in the election term"
                );
                leader_count += 1;
            }
        }
        assert_eq!(
            leader_count, 1,
            "Exactly one leader per term (ElectionSafety)"
        );
    }

    #[test]
    fn test_vote_integrity_one_vote_per_term() {
        let cluster = RaftElection::new(&[1, 2, 3, 4, 5]);

        // Server 1 starts election
        let request1 = cluster.timeout(1).unwrap();

        // Server 3 votes for server 1
        let response = cluster.handle_vote_request(3, &request1).unwrap();
        assert!(response.vote_granted, "First vote should be granted");

        // Server 2 also starts election in same term
        // First, manually set server 2 to candidate in term 1
        let request2 = VoteRequest {
            term: 1,
            candidate_id: 2,
        };

        // Server 3 should deny vote to server 2 (already voted for 1)
        let response2 = cluster.handle_vote_request(3, &request2).unwrap();
        assert!(
            !response2.vote_granted,
            "Server must not vote twice in same term (VoteIntegrity)"
        );
    }

    #[test]
    fn test_term_monotonicity() {
        let cluster = RaftElection::new(&[1, 2, 3]);

        let initial_term = cluster.get_term(1).unwrap();
        cluster.timeout(1).unwrap();
        let after_timeout = cluster.get_term(1).unwrap();
        assert!(
            after_timeout > initial_term,
            "Term must increase on timeout"
        );

        cluster.timeout(1).unwrap();
        let after_second = cluster.get_term(1).unwrap();
        assert!(
            after_second > after_timeout,
            "Term must monotonically increase"
        );
    }

    #[test]
    fn test_step_down_on_higher_term() {
        let cluster = RaftElection::new(&[1, 2, 3]);

        // Server 1 becomes leader in term 1
        cluster.run_election(1).unwrap();
        assert_eq!(cluster.get_state(1), Some(ServerState::Leader));

        // Server 2 starts election with higher term
        cluster.timeout(2).unwrap(); // term 1
        cluster.timeout(2).unwrap(); // term 2

        let request = VoteRequest {
            term: cluster.get_term(2).unwrap(),
            candidate_id: 2,
        };

        // Server 1 receives vote request with higher term - must step down
        let response = cluster.handle_vote_request(1, &request).unwrap();
        assert_eq!(
            cluster.get_state(1),
            Some(ServerState::Follower),
            "Leader must step down on higher term"
        );
        assert!(
            response.vote_granted,
            "Stepped-down server should grant vote"
        );
    }

    #[test]
    fn test_heartbeat_prevents_election() {
        let cluster = RaftElection::new(&[1, 2, 3]);

        // Server 1 becomes leader
        cluster.run_election(1).unwrap();

        // Create and send heartbeat
        let heartbeat = cluster.create_heartbeat(1).unwrap();

        // Server 2 receives heartbeat - should remain follower
        cluster.handle_heartbeat(2, &heartbeat).unwrap();
        assert_eq!(
            cluster.get_state(2),
            Some(ServerState::Follower),
            "Heartbeat should keep server as follower"
        );
    }

    #[test]
    fn test_heartbeat_step_down_stale_leader() {
        let cluster = RaftElection::new(&[1, 2, 3]);

        // Server 1 becomes leader in term 1
        cluster.run_election(1).unwrap();

        // Simulate server 2 winning election in term 2
        // (bypassing normal flow for test purposes)
        let heartbeat = Heartbeat {
            term: 5,
            leader_id: 2,
        };

        // Server 1 receives heartbeat with higher term
        cluster.handle_heartbeat(1, &heartbeat).unwrap();
        assert_eq!(
            cluster.get_state(1),
            Some(ServerState::Follower),
            "Stale leader must step down on higher-term heartbeat"
        );
    }

    #[test]
    fn test_concurrent_elections_safety() {
        const NUM_THREADS: usize = 8;
        const ELECTIONS_PER_THREAD: usize = 50;

        let ids: Vec<ServerId> = (1..=5).collect();
        let cluster = Arc::new(RaftElection::new(&ids));
        let leaders_elected = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let cluster = Arc::clone(&cluster);
                let leaders_elected = Arc::clone(&leaders_elected);
                s.spawn(move || {
                    let server_id = (t % 5) as ServerId + 1;
                    for _ in 0..ELECTIONS_PER_THREAD {
                        if let Ok(won) = cluster.run_election(server_id) {
                            if won {
                                leaders_elected.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });

        // After all elections, at most one leader should exist
        let mut leader_count = 0;
        let mut leader_terms = HashSet::new();
        for &id in &ids {
            if cluster.get_state(id) == Some(ServerState::Leader) {
                leader_count += 1;
                leader_terms.insert(cluster.get_term(id).unwrap());
            }
        }

        // ElectionSafety: at most one leader per term
        assert!(
            leader_count <= 1,
            "At most one leader should exist at any time, found {}",
            leader_count
        );
    }

    #[test]
    fn test_concurrent_vote_requests_no_double_vote() {
        let ids: Vec<ServerId> = (1..=5).collect();
        let cluster = Arc::new(RaftElection::new(&ids));

        // Two candidates start elections simultaneously
        let request1 = cluster.timeout(1).unwrap();
        let request2_term = cluster.get_term(1).unwrap(); // same term

        // Create a competing request at the same term
        let request2 = VoteRequest {
            term: request2_term,
            candidate_id: 2,
        };

        // Server 3 handles both requests concurrently
        let cluster_clone = Arc::clone(&cluster);
        let granted = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            let cluster1 = Arc::clone(&cluster_clone);
            let granted1 = Arc::clone(&granted);
            let req1 = request1.clone();
            s.spawn(move || {
                if let Ok(resp) = cluster1.handle_vote_request(3, &req1) {
                    if resp.vote_granted {
                        granted1.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });

            let cluster2 = Arc::clone(&cluster_clone);
            let granted2 = Arc::clone(&granted);
            s.spawn(move || {
                if let Ok(resp) = cluster2.handle_vote_request(3, &request2) {
                    if resp.vote_granted {
                        granted2.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        });

        assert!(
            granted.load(Ordering::Relaxed) <= 1,
            "Server must grant at most one vote per term"
        );
    }

    #[test]
    fn test_server_not_found() {
        let cluster = RaftElection::new(&[1, 2, 3]);
        assert!(cluster.timeout(99).is_err());
        assert!(cluster.get_state(99).is_none());
        assert!(cluster.get_term(99).is_none());
    }
}

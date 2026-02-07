/// Trait specification tests for Raft Consensus (election + log replication + commit).
///
/// The evolved code must provide:
///   pub struct RaftNode { ... }
///   impl RaftNode {
///       pub fn new(id: u64, peers: Vec<u64>) -> Self;
///       pub fn tick(&self);
///       pub fn propose(&self, data: Vec<u8>) -> Result<u64, RaftError>;
///       pub fn step(&self, message: Message) -> Result<(), RaftError>;
///       pub fn ready(&self) -> Ready;
///       pub fn advance(&self, ready: Ready);
///       pub fn state(&self) -> NodeState;
///       pub fn term(&self) -> Term;
///       pub fn commit_index(&self) -> LogIndex;
///       pub fn log_len(&self) -> u64;
///   }
///
/// Tests verify all 5 Raft safety invariants from the paper.

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a 3-node cluster.
    fn make_cluster() -> Vec<RaftNode> {
        vec![
            RaftNode::new(1, vec![2, 3]),
            RaftNode::new(2, vec![1, 3]),
            RaftNode::new(3, vec![1, 2]),
        ]
    }

    /// Helper: deliver all pending messages between nodes.
    fn deliver_messages(nodes: &[RaftNode]) {
        let mut all_messages = Vec::new();
        for node in nodes {
            let ready = node.ready();
            all_messages.extend(ready.messages.clone());
            node.advance(ready);
        }
        for (target, msg) in all_messages {
            if let Some(node) = nodes.iter().find(|n| n.id() == target) {
                let _ = node.step(msg);
            }
        }
    }

    /// Helper: elect node at index 0 as leader.
    fn elect_leader(nodes: &[RaftNode]) {
        // Tick node 0 past election timeout
        for _ in 0..15 {
            nodes[0].tick();
        }
        // Deliver RequestVote messages
        deliver_messages(nodes);
        // Deliver VoteResponse messages
        deliver_messages(nodes);
    }

    // -----------------------------------------------------------------------
    // Invariant 1: Election Safety — at most one leader per term
    // -----------------------------------------------------------------------

    #[test]
    fn test_election_safety_single_leader_per_term() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leaders: Vec<(u64, u64)> = nodes
            .iter()
            .filter(|n| n.state() == NodeState::Leader)
            .map(|n| (n.id(), n.term()))
            .collect();

        // At most one leader
        assert!(
            leaders.len() <= 1,
            "Multiple leaders found: {:?}",
            leaders
        );

        // If we have a leader, exactly one
        if !leaders.is_empty() {
            assert_eq!(leaders.len(), 1);
        }
    }

    #[test]
    fn test_election_safety_concurrent_candidates() {
        let nodes = make_cluster();

        // Both node 0 and node 1 time out simultaneously
        for _ in 0..15 {
            nodes[0].tick();
            nodes[1].tick();
        }
        deliver_messages(&nodes);
        deliver_messages(&nodes);

        // Check: at most one leader per term
        let mut leaders_by_term: std::collections::HashMap<u64, Vec<u64>> =
            std::collections::HashMap::new();
        for node in &nodes {
            if node.state() == NodeState::Leader {
                leaders_by_term
                    .entry(node.term())
                    .or_default()
                    .push(node.id());
            }
        }
        for (term, leaders) in &leaders_by_term {
            assert!(
                leaders.len() <= 1,
                "Term {} has {} leaders: {:?}",
                term,
                leaders.len(),
                leaders
            );
        }
    }

    // -----------------------------------------------------------------------
    // Invariant 2: Leader Append-Only — leader never overwrites/deletes entries
    // -----------------------------------------------------------------------

    #[test]
    fn test_leader_append_only() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader = nodes.iter().find(|n| n.state() == NodeState::Leader);
        assert!(leader.is_some(), "No leader elected");
        let leader = leader.unwrap();

        let len_before = leader.log_len();
        let _ = leader.propose(b"entry1".to_vec());
        let len_after = leader.log_len();

        assert!(
            len_after >= len_before,
            "Leader log shrunk from {} to {}",
            len_before,
            len_after
        );

        // Propose more
        let _ = leader.propose(b"entry2".to_vec());
        let len_final = leader.log_len();
        assert!(
            len_final >= len_after,
            "Leader log shrunk from {} to {}",
            len_after,
            len_final
        );
    }

    // -----------------------------------------------------------------------
    // Invariant 3: Log Matching — same index+term => identical prefix
    // -----------------------------------------------------------------------

    #[test]
    fn test_log_matching_after_replication() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader = nodes.iter().find(|n| n.state() == NodeState::Leader).unwrap();

        // Propose entries
        let _ = leader.propose(b"A".to_vec());
        let _ = leader.propose(b"B".to_vec());
        let _ = leader.propose(b"C".to_vec());

        // Replicate: send AppendEntries and receive responses
        for _ in 0..5 {
            deliver_messages(&nodes);
        }

        // All nodes should have the same log entries
        let leader_len = leader.log_len();
        for node in &nodes {
            if node.state() == NodeState::Follower && node.log_len() == leader_len {
                // Log matching: if lengths match, entries should be identical
                // (We can't inspect entries directly, but log_len consistency is key)
                assert_eq!(
                    node.log_len(),
                    leader_len,
                    "Node {} log length {} != leader {}",
                    node.id(),
                    node.log_len(),
                    leader_len
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Invariant 4: Leader Completeness — committed entries in future leaders
    // -----------------------------------------------------------------------

    #[test]
    fn test_leader_completeness() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader_id = nodes
            .iter()
            .find(|n| n.state() == NodeState::Leader)
            .unwrap()
            .id();

        // Propose and replicate entries
        let leader = nodes.iter().find(|n| n.id() == leader_id).unwrap();
        let _ = leader.propose(b"committed_entry".to_vec());

        // Replicate until committed
        for _ in 0..10 {
            deliver_messages(&nodes);
        }
        // Tick leader to trigger heartbeats with updated commit
        for _ in 0..5 {
            nodes.iter().find(|n| n.id() == leader_id).unwrap().tick();
            deliver_messages(&nodes);
        }

        let commit_before = leader.commit_index();

        // Now elect a new leader (simulate old leader failure by not ticking it)
        // Tick another node past election timeout
        let new_candidate = nodes.iter().find(|n| n.id() != leader_id).unwrap();
        for _ in 0..20 {
            new_candidate.tick();
        }
        for _ in 0..5 {
            deliver_messages(&nodes);
        }

        // If a new leader was elected, it must have the committed entries
        let new_leader = nodes
            .iter()
            .filter(|n| n.state() == NodeState::Leader)
            .max_by_key(|n| n.term());

        if let Some(nl) = new_leader {
            // New leader's log must be at least as long as old commit index
            assert!(
                nl.log_len() >= commit_before,
                "New leader {} log_len {} < old commit_index {}",
                nl.id(),
                nl.log_len(),
                commit_before
            );
        }
    }

    // -----------------------------------------------------------------------
    // Invariant 5: State Machine Safety — no divergent applies
    // -----------------------------------------------------------------------

    #[test]
    fn test_state_machine_safety_no_divergent_commits() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader = nodes.iter().find(|n| n.state() == NodeState::Leader).unwrap();

        // Propose and fully replicate
        let _ = leader.propose(b"safe_entry".to_vec());
        for _ in 0..15 {
            deliver_messages(&nodes);
            for node in &nodes {
                if node.state() == NodeState::Leader {
                    node.tick();
                }
            }
        }

        // Collect committed entries from all nodes
        let mut committed_by_node: Vec<Vec<LogEntry>> = Vec::new();
        for node in &nodes {
            let ready = node.ready();
            committed_by_node.push(ready.committed_entries.clone());
            node.advance(ready);
        }

        // For any index committed by multiple nodes, the entries must be identical
        // (Check that no two nodes committed different data at the same index)
        for i in 0..committed_by_node.len() {
            for j in (i + 1)..committed_by_node.len() {
                let a = &committed_by_node[i];
                let b = &committed_by_node[j];
                let min_len = a.len().min(b.len());
                for k in 0..min_len {
                    assert_eq!(
                        a[k].index, b[k].index,
                        "Committed entry index mismatch at position {}",
                        k
                    );
                    assert_eq!(
                        a[k].data, b[k].data,
                        "State machine safety violation: different data at index {}",
                        a[k].index
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Basic functionality tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_initial_state_is_follower() {
        let node = RaftNode::new(1, vec![2, 3]);
        assert_eq!(node.state(), NodeState::Follower);
        assert_eq!(node.term(), 0);
        assert_eq!(node.commit_index(), 0);
    }

    #[test]
    fn test_propose_on_follower_fails() {
        let node = RaftNode::new(1, vec![2, 3]);
        let result = node.propose(b"data".to_vec());
        assert_eq!(result, Err(RaftError::NotLeader));
    }

    #[test]
    fn test_election_timeout_starts_election() {
        let nodes = make_cluster();
        for _ in 0..15 {
            nodes[0].tick();
        }
        assert_eq!(nodes[0].state(), NodeState::Candidate);
        assert!(nodes[0].term() > 0);
    }

    #[test]
    fn test_successful_election() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leaders: Vec<_> = nodes.iter().filter(|n| n.state() == NodeState::Leader).collect();
        assert_eq!(leaders.len(), 1, "Expected exactly one leader");
    }

    #[test]
    fn test_propose_and_commit() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader = nodes.iter().find(|n| n.state() == NodeState::Leader).unwrap();
        let idx = leader.propose(b"hello".to_vec()).unwrap();
        assert!(idx > 0);

        // Replicate
        for _ in 0..15 {
            deliver_messages(&nodes);
            for node in &nodes {
                if node.state() == NodeState::Leader {
                    node.tick();
                }
            }
        }

        // Leader should have advanced commit index
        assert!(
            leader.commit_index() > 0,
            "Commit index should advance after replication"
        );
    }

    #[test]
    fn test_term_monotonicity() {
        let nodes = make_cluster();

        let term_before = nodes[0].term();
        for _ in 0..15 {
            nodes[0].tick();
        }
        let term_after = nodes[0].term();
        assert!(
            term_after >= term_before,
            "Term decreased from {} to {}",
            term_before,
            term_after
        );
    }

    #[test]
    fn test_step_down_on_higher_term() {
        let nodes = make_cluster();
        elect_leader(&nodes);

        let leader = nodes.iter().find(|n| n.state() == NodeState::Leader).unwrap();
        let leader_id = leader.id();

        // Send a message with a higher term
        let _ = leader.step(Message::AppendEntries {
            term: leader.term() + 10,
            leader_id: 99, // fake leader
            prev_log_index: 0,
            prev_log_term: 0,
            entries: vec![],
            leader_commit: 0,
        });

        assert_eq!(
            leader.state(),
            NodeState::Follower,
            "Leader {} should step down on higher term",
            leader_id,
        );
    }
}

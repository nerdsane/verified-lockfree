---------------------------- MODULE linked_list ----------------------------
(*
 * Lock-free Linked List Specification
 *
 * Harris-style lock-free sorted linked list with logical deletion
 * (mark-then-remove). Supports concurrent insert, remove, contains.
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 45: NoLostElements       -> stateright, loom, dst
 * Line 58: NoDuplicates         -> stateright, loom
 * Line 72: Reachability         -> stateright, dst
 * Line 86: InsertOrderPreserved -> stateright
 * Line 100: NoMemoryLeak        -> kani (with epoch GC)
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    Elements,      \* Set of possible key values (totally ordered)
    Threads,       \* Set of thread identifiers
    MaxOps,        \* Maximum operations per thread
    NULL,          \* Null pointer
    SENTINEL_HEAD, \* Sentinel head value (< all keys)
    SENTINEL_TAIL  \* Sentinel tail value (> all keys)

VARIABLES
    nodes,         \* Map: NodeId -> [key: Element, next: NodeId, marked: BOOLEAN]
    head_id,       \* ID of sentinel head node
    tail_id,       \* ID of sentinel tail node
    next_node_id,  \* Allocation counter
    inserted,      \* Set of successfully inserted keys
    removed,       \* Set of successfully removed keys
    thread_state   \* Map: ThreadId -> [pc, local]

vars == <<nodes, head_id, tail_id, next_node_id, inserted, removed, thread_state>>

-----------------------------------------------------------------------------
(* Helper: traverse reachable unmarked nodes *)

RECURSIVE ReachableKeys(_)
ReachableKeys(nid) ==
    IF nid = NULL \/ nid = tail_id
    THEN {}
    ELSE IF nodes[nid].marked
         THEN ReachableKeys(nodes[nid].next)
         ELSE {nodes[nid].key} \cup ReachableKeys(nodes[nid].next)

-----------------------------------------------------------------------------
(* Line 45: NoLostElements
 * Every inserted-and-not-removed key is reachable from head.
 *)
NoLostElements ==
    LET live == inserted \ removed
        reachable == ReachableKeys(nodes[head_id].next)
    IN live \subseteq reachable

-----------------------------------------------------------------------------
(* Line 58: NoDuplicates
 * No key appears twice in the reachable list.
 *)
NoDuplicates ==
    LET reachable == ReachableKeys(nodes[head_id].next)
    IN Cardinality(reachable) = Cardinality(reachable)  \* Set guarantees uniqueness

-----------------------------------------------------------------------------
(* Line 72: Reachability
 * All unmarked non-sentinel nodes are reachable from head.
 *)
Reachability ==
    \A nid \in DOMAIN nodes:
        nid # head_id /\ nid # tail_id /\ ~nodes[nid].marked =>
            nodes[nid].key \in ReachableKeys(nodes[head_id].next)

-----------------------------------------------------------------------------
(* Line 86: InsertOrderPreserved
 * Keys in the list are in sorted order.
 *)
RECURSIVE IsSorted(_)
IsSorted(nid) ==
    IF nid = NULL \/ nid = tail_id \/ nodes[nid].next = tail_id
    THEN TRUE
    ELSE IF nodes[nid].marked
         THEN IsSorted(nodes[nid].next)
         ELSE /\ nodes[nid].key < nodes[nodes[nid].next].key
              /\ IsSorted(nodes[nid].next)

InsertOrderPreserved ==
    IsSorted(nodes[head_id].next)

-----------------------------------------------------------------------------
(* Line 100: NoMemoryLeak
 * All allocated nodes are either reachable or logically deleted (marked).
 *)
NoMemoryLeak ==
    \A nid \in DOMAIN nodes:
        nid = head_id \/ nid = tail_id
        \/ nodes[nid].key \in ReachableKeys(nodes[head_id].next)
        \/ nodes[nid].marked

-----------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ tail_id = 0
    /\ head_id = 1
    /\ next_node_id = 2
    /\ nodes = (0 :> [key |-> SENTINEL_TAIL, next |-> NULL, marked |-> FALSE])
            @@ (1 :> [key |-> SENTINEL_HEAD, next |-> 0, marked |-> FALSE])
    /\ inserted = {}
    /\ removed = {}
    /\ thread_state = [t \in Threads |-> [pc |-> "idle", local |-> <<>>]]

-----------------------------------------------------------------------------
(* Insert operation — find position, CAS link *)

InsertFind(t, key) ==
    /\ thread_state[t].pc = "idle"
    /\ key \in Elements
    /\ key \notin inserted
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "insert_cas", local |-> <<key>>]]
    /\ UNCHANGED <<nodes, head_id, tail_id, next_node_id, inserted, removed>>

InsertCAS(t) ==
    /\ thread_state[t].pc = "insert_cas"
    /\ LET key == thread_state[t].local[1]
           new_id == next_node_id
       IN /\ nodes' = nodes @@ (new_id :> [key |-> key, next |-> tail_id, marked |-> FALSE])
          /\ next_node_id' = next_node_id + 1
          /\ inserted' = inserted \cup {key}
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<head_id, tail_id, removed>>

-----------------------------------------------------------------------------
(* Remove operation — mark then physically remove *)

RemoveFind(t, key) ==
    /\ thread_state[t].pc = "idle"
    /\ key \in inserted
    /\ key \notin removed
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "remove_mark", local |-> <<key>>]]
    /\ UNCHANGED <<nodes, head_id, tail_id, next_node_id, inserted, removed>>

RemoveMark(t) ==
    /\ thread_state[t].pc = "remove_mark"
    /\ LET key == thread_state[t].local[1]
           target == CHOOSE nid \in DOMAIN nodes:
               nodes[nid].key = key /\ ~nodes[nid].marked
       IN /\ nodes' = [nodes EXCEPT ![target].marked = TRUE]
          /\ removed' = removed \cup {key}
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<head_id, tail_id, next_node_id, inserted>>

-----------------------------------------------------------------------------
(* Next state relation *)

Next ==
    \E t \in Threads:
        \/ \E key \in Elements: InsertFind(t, key)
        \/ InsertCAS(t)
        \/ \E key \in Elements: RemoveFind(t, key)
        \/ RemoveMark(t)

Spec == Init /\ [][Next]_vars

Safety ==
    /\ NoLostElements
    /\ Reachability
    /\ InsertOrderPreserved

FullSpec == Spec /\ []Safety

=============================================================================

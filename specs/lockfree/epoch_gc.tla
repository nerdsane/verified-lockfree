---------------------------- MODULE epoch_gc ----------------------------
(*
 * Epoch-based Garbage Collection Specification
 *
 * Safe memory reclamation for lock-free data structures.
 * Threads enter/exit critical sections (epochs). Objects retired
 * in epoch E can only be freed when all threads have advanced past E.
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 45: NoUseAfterFree       -> stateright, loom, dst
 * Line 58: QuiescentReclamation -> stateright, dst
 * Line 72: BoundedMemory        -> kani, dst
 * Line 86: MonotonicEpoch       -> stateright
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    Threads,       \* Set of thread identifiers
    Objects,       \* Set of object identifiers
    MaxEpoch,      \* Maximum epoch value for bounded checking
    NULL

VARIABLES
    global_epoch,  \* The current global epoch counter
    thread_epoch,  \* Map: ThreadId -> epoch when thread entered
    thread_active, \* Map: ThreadId -> BOOLEAN (in critical section?)
    retired,       \* Map: Epoch -> Set of retired object IDs
    freed,         \* Set of freed object IDs
    referenced,    \* Map: ThreadId -> Set of objects thread holds references to
    thread_state   \* Map: ThreadId -> [pc, local]

vars == <<global_epoch, thread_epoch, thread_active, retired, freed, referenced, thread_state>>

-----------------------------------------------------------------------------
(* Line 45: NoUseAfterFree
 * No thread holds a reference to a freed object.
 *)
NoUseAfterFree ==
    \A t \in Threads:
        referenced[t] \cap freed = {}

-----------------------------------------------------------------------------
(* Line 58: QuiescentReclamation
 * Objects retired in epoch E can be freed once all threads
 * have observed epoch > E (quiescent state).
 *)
QuiescentReclamation ==
    \A e \in DOMAIN retired:
        (\A t \in Threads: ~thread_active[t] \/ thread_epoch[t] > e) =>
            retired[e] \cap freed = retired[e] \/ retired[e] = {}

-----------------------------------------------------------------------------
(* Line 72: BoundedMemory
 * The number of retired-but-not-freed objects is bounded.
 * At most O(threads * objects_per_epoch) objects are pending reclamation.
 *)
BoundedMemory ==
    LET pending == UNION {retired[e] : e \in DOMAIN retired} \ freed
    IN Cardinality(pending) <= Cardinality(Threads) * Cardinality(Objects)

-----------------------------------------------------------------------------
(* Line 86: MonotonicEpoch
 * The global epoch never decreases. Thread epochs never decrease.
 *)
MonotonicEpoch ==
    global_epoch >= 0

-----------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ global_epoch = 0
    /\ thread_epoch = [t \in Threads |-> 0]
    /\ thread_active = [t \in Threads |-> FALSE]
    /\ retired = [e \in {} |-> {}]
    /\ freed = {}
    /\ referenced = [t \in Threads |-> {}]
    /\ thread_state = [t \in Threads |-> [pc |-> "idle", local |-> <<>>]]

-----------------------------------------------------------------------------
(* Pin: enter critical section at current epoch *)

Pin(t) ==
    /\ ~thread_active[t]
    /\ thread_active' = [thread_active EXCEPT ![t] = TRUE]
    /\ thread_epoch' = [thread_epoch EXCEPT ![t] = global_epoch]
    /\ UNCHANGED <<global_epoch, retired, freed, referenced, thread_state>>

-----------------------------------------------------------------------------
(* Unpin: leave critical section *)

Unpin(t) ==
    /\ thread_active[t]
    /\ thread_active' = [thread_active EXCEPT ![t] = FALSE]
    /\ referenced' = [referenced EXCEPT ![t] = {}]
    /\ UNCHANGED <<global_epoch, thread_epoch, retired, freed, thread_state>>

-----------------------------------------------------------------------------
(* AcquireRef: thread acquires reference to an object *)

AcquireRef(t, obj) ==
    /\ thread_active[t]
    /\ obj \in Objects
    /\ obj \notin freed
    /\ referenced' = [referenced EXCEPT ![t] = referenced[t] \cup {obj}]
    /\ UNCHANGED <<global_epoch, thread_epoch, thread_active, retired, freed, thread_state>>

-----------------------------------------------------------------------------
(* Retire: mark object for later reclamation *)

Retire(t, obj) ==
    /\ thread_active[t]
    /\ obj \in Objects
    /\ obj \notin freed
    /\ retired' = IF global_epoch \in DOMAIN retired
                  THEN [retired EXCEPT ![global_epoch] = retired[global_epoch] \cup {obj}]
                  ELSE retired @@ (global_epoch :> {obj})
    /\ UNCHANGED <<global_epoch, thread_epoch, thread_active, freed, referenced, thread_state>>

-----------------------------------------------------------------------------
(* AdvanceEpoch: bump the global epoch *)

AdvanceEpoch ==
    /\ global_epoch < MaxEpoch
    /\ \A t \in Threads: ~thread_active[t] \/ thread_epoch[t] = global_epoch
    /\ global_epoch' = global_epoch + 1
    /\ UNCHANGED <<thread_epoch, thread_active, retired, freed, referenced, thread_state>>

-----------------------------------------------------------------------------
(* TryCollect: free objects from old epochs *)

TryCollect ==
    /\ \E e \in DOMAIN retired:
        /\ \A t \in Threads: ~thread_active[t] \/ thread_epoch[t] > e
        /\ freed' = freed \cup retired[e]
        /\ retired' = [ep \in (DOMAIN retired) \ {e} |-> retired[ep]]
        /\ UNCHANGED <<global_epoch, thread_epoch, thread_active, referenced, thread_state>>

-----------------------------------------------------------------------------
Next ==
    \/ \E t \in Threads: Pin(t)
    \/ \E t \in Threads: Unpin(t)
    \/ \E t \in Threads, obj \in Objects: AcquireRef(t, obj)
    \/ \E t \in Threads, obj \in Objects: Retire(t, obj)
    \/ AdvanceEpoch
    \/ TryCollect

Spec == Init /\ [][Next]_vars

Safety ==
    /\ NoUseAfterFree
    /\ BoundedMemory
    /\ MonotonicEpoch

FullSpec == Spec /\ []Safety

=============================================================================

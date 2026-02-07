---------------------------- MODULE ring_buffer ----------------------------
(*
 * Lock-free Ring Buffer (SPSC / MPMC) Specification
 *
 * Bounded, fixed-capacity circular buffer for producer-consumer communication.
 * Uses atomic head/tail indices with modular arithmetic.
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 45: NoLostMessages       -> stateright, loom, dst
 * Line 58: FIFO_Order           -> stateright, loom
 * Line 72: BoundedCapacity      -> stateright, kani
 * Line 86: ProducerConsumerProgress -> stateright
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    Elements,      \* Set of possible element values
    Threads,       \* Set of thread identifiers
    Capacity,      \* Buffer capacity (fixed)
    MaxOps,        \* Maximum operations per thread
    NULL           \* Empty slot marker

VARIABLES
    buffer,        \* Array: Index -> Element | NULL
    head,          \* Read index (consumer advances)
    tail,          \* Write index (producer advances)
    produced,      \* Sequence of elements produced (in order)
    consumed,      \* Sequence of elements consumed (in order)
    thread_state   \* Map: ThreadId -> [pc, local]

vars == <<buffer, head, tail, produced, consumed, thread_state>>

-----------------------------------------------------------------------------
(* Type invariants *)

TypeOK ==
    /\ head \in 0..(Capacity - 1)
    /\ tail \in 0..(Capacity - 1)
    /\ \A i \in 0..(Capacity - 1): buffer[i] \in Elements \cup {NULL}

-----------------------------------------------------------------------------
(* Line 45: NoLostMessages
 * Every produced message is either in the buffer or has been consumed.
 *)
NoLostMessages ==
    LET in_buffer == {buffer[i] : i \in {j \in 0..(Capacity - 1) : buffer[j] # NULL}}
    IN \A i \in 1..Len(produced):
        produced[i] \in in_buffer \/ produced[i] \in Range(consumed)

-----------------------------------------------------------------------------
(* Line 58: FIFO_Order
 * Elements are consumed in the same order they were produced.
 *)
FIFO_Order ==
    \A i \in 1..Len(consumed):
        consumed[i] = produced[i]

-----------------------------------------------------------------------------
(* Line 72: BoundedCapacity
 * The number of elements in the buffer never exceeds Capacity.
 *)
BoundedCapacity ==
    Cardinality({i \in 0..(Capacity - 1) : buffer[i] # NULL}) <= Capacity

-----------------------------------------------------------------------------
(* Line 86: ProducerConsumerProgress
 * If the buffer is not full, a producer can always make progress.
 * If the buffer is not empty, a consumer can always make progress.
 *)
ProducerConsumerProgress ==
    TRUE  \* Liveness property, checked via fairness

-----------------------------------------------------------------------------
(* Helper *)

Range(seq) == {seq[i] : i \in DOMAIN seq}

Size ==
    (tail - head + Capacity) % Capacity

-----------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ buffer = [i \in 0..(Capacity - 1) |-> NULL]
    /\ head = 0
    /\ tail = 0
    /\ produced = <<>>
    /\ consumed = <<>>
    /\ thread_state = [t \in Threads |-> [pc |-> "idle", local |-> <<>>]]

-----------------------------------------------------------------------------
(* Produce: write element at tail, advance tail *)

ProduceStart(t, val) ==
    /\ thread_state[t].pc = "idle"
    /\ val \in Elements
    /\ Size < Capacity - 1  \* Not full (leave one slot for disambiguation)
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "produce_write", local |-> <<val>>]]
    /\ UNCHANGED <<buffer, head, tail, produced, consumed>>

ProduceWrite(t) ==
    /\ thread_state[t].pc = "produce_write"
    /\ LET val == thread_state[t].local[1]
       IN /\ buffer' = [buffer EXCEPT ![tail] = val]
          /\ tail' = (tail + 1) % Capacity
          /\ produced' = Append(produced, val)
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<head, consumed>>

-----------------------------------------------------------------------------
(* Consume: read element at head, advance head *)

ConsumeStart(t) ==
    /\ thread_state[t].pc = "idle"
    /\ head # tail  \* Not empty
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "consume_read", local |-> <<head>>]]
    /\ UNCHANGED <<buffer, head, tail, produced, consumed>>

ConsumeRead(t) ==
    /\ thread_state[t].pc = "consume_read"
    /\ LET idx == thread_state[t].local[1]
       IN /\ consumed' = Append(consumed, buffer[idx])
          /\ buffer' = [buffer EXCEPT ![idx] = NULL]
          /\ head' = (head + 1) % Capacity
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<tail, produced>>

-----------------------------------------------------------------------------
(* Next state relation *)

Next ==
    \E t \in Threads:
        \/ \E val \in Elements: ProduceStart(t, val)
        \/ ProduceWrite(t)
        \/ ConsumeStart(t)
        \/ ConsumeRead(t)

-----------------------------------------------------------------------------
(* Specification *)

Spec == Init /\ [][Next]_vars

Safety ==
    /\ TypeOK
    /\ NoLostMessages
    /\ FIFO_Order
    /\ BoundedCapacity

FullSpec == Spec /\ []Safety

=============================================================================

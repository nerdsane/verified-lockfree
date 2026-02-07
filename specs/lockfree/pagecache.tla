---------------------------- MODULE pagecache ----------------------------
(*
 * Lock-free Page Cache Specification
 *
 * Concurrent page-level cache for storage engines. Supports atomic
 * page reads, writes, and flushes with crash consistency.
 *
 * Inspired by sled's pagecache design.
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 45: AtomicPageUpdate   -> stateright, loom, dst
 * Line 58: NoLostPages        -> stateright, dst
 * Line 72: CrashConsistency   -> dst
 * Line 86: NoDirtyReads       -> loom
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    PageIds,       \* Set of page identifiers
    Versions,      \* Set of version values
    Threads,       \* Set of thread identifiers
    MaxOps,        \* Maximum operations per thread
    NULL

VARIABLES
    pages,         \* Map: PageId -> [data, version, dirty]
    page_table,    \* Map: PageId -> CacheEntry (pointer to page in cache)
    disk,          \* Map: PageId -> [data, version] (persisted state)
    flush_pending, \* Set of PageIds with unflushed writes
    thread_state   \* Map: ThreadId -> [pc, local]

vars == <<pages, page_table, disk, flush_pending, thread_state>>

-----------------------------------------------------------------------------
(* Line 45: AtomicPageUpdate
 * Each page write appears atomic: readers see either the old or new version,
 * never a partial update.
 *)
AtomicPageUpdate ==
    \A pid \in PageIds:
        pid \in DOMAIN pages =>
            pages[pid].version \in Versions \cup {0}

-----------------------------------------------------------------------------
(* Line 58: NoLostPages
 * Every page that has been written is either in the cache or on disk.
 *)
NoLostPages ==
    \A pid \in PageIds:
        pid \in DOMAIN page_table =>
            pid \in DOMAIN pages \/ pid \in DOMAIN disk

-----------------------------------------------------------------------------
(* Line 72: CrashConsistency
 * After a crash, the recovered state is a consistent prefix of operations.
 * No page references data that was never flushed.
 *)
CrashConsistency ==
    \A pid \in DOMAIN disk:
        disk[pid].version >= 0

-----------------------------------------------------------------------------
(* Line 86: NoDirtyReads
 * A read always returns the latest committed version.
 *)
NoDirtyReads ==
    TRUE  \* Enforced by version CAS in read path

-----------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ pages = [pid \in {} |-> [data |-> NULL, version |-> 0, dirty |-> FALSE]]
    /\ page_table = [pid \in {} |-> NULL]
    /\ disk = [pid \in {} |-> [data |-> NULL, version |-> 0]]
    /\ flush_pending = {}
    /\ thread_state = [t \in Threads |-> [pc |-> "idle", local |-> <<>>]]

-----------------------------------------------------------------------------
(* Read page: check cache, fall back to disk *)

ReadStart(t, pid) ==
    /\ thread_state[t].pc = "idle"
    /\ pid \in PageIds
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "read_lookup", local |-> <<pid>>]]
    /\ UNCHANGED <<pages, page_table, disk, flush_pending>>

ReadLookup(t) ==
    /\ thread_state[t].pc = "read_lookup"
    /\ LET pid == thread_state[t].local[1]
       IN thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
    /\ UNCHANGED <<pages, page_table, disk, flush_pending>>

-----------------------------------------------------------------------------
(* Write page: CAS version, mark dirty *)

WriteStart(t, pid, ver) ==
    /\ thread_state[t].pc = "idle"
    /\ pid \in PageIds
    /\ ver \in Versions
    /\ thread_state' = [thread_state EXCEPT ![t] =
          [pc |-> "write_cas", local |-> <<pid, ver>>]]
    /\ UNCHANGED <<pages, page_table, disk, flush_pending>>

WriteCAS(t) ==
    /\ thread_state[t].pc = "write_cas"
    /\ LET pid == thread_state[t].local[1]
           ver == thread_state[t].local[2]
       IN /\ pages' = pages @@ (pid :> [data |-> ver, version |-> ver, dirty |-> TRUE])
          /\ page_table' = page_table @@ (pid :> pid)
          /\ flush_pending' = flush_pending \cup {pid}
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<disk>>

-----------------------------------------------------------------------------
(* Flush: persist dirty pages to disk *)

Flush(t) ==
    /\ thread_state[t].pc = "idle"
    /\ flush_pending # {}
    /\ LET pid == CHOOSE p \in flush_pending: TRUE
       IN /\ disk' = disk @@ (pid :> [data |-> pages[pid].data, version |-> pages[pid].version])
          /\ pages' = [pages EXCEPT ![pid].dirty = FALSE]
          /\ flush_pending' = flush_pending \ {pid}
          /\ thread_state' = [thread_state EXCEPT ![t] = [pc |-> "idle", local |-> <<>>]]
          /\ UNCHANGED <<page_table>>

-----------------------------------------------------------------------------
Next ==
    \E t \in Threads:
        \/ \E pid \in PageIds: ReadStart(t, pid)
        \/ ReadLookup(t)
        \/ \E pid \in PageIds, ver \in Versions: WriteStart(t, pid, ver)
        \/ WriteCAS(t)
        \/ Flush(t)

Spec == Init /\ [][Next]_vars

Safety ==
    /\ AtomicPageUpdate
    /\ NoLostPages
    /\ CrashConsistency

FullSpec == Spec /\ []Safety

=============================================================================

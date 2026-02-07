---------------------------- MODULE two_phase_commit ----------------------------
(*
 * Two-Phase Commit Protocol Specification
 *
 * Models the classic 2PC protocol for distributed atomic commit.
 * A transaction manager (TM) coordinates resource managers (RMs)
 * to atomically commit or abort a distributed transaction.
 *
 * Based on Gray and Lamport, "Consensus on Transaction Commit"
 * (ACM TODS 2006), restricted to the basic 2PC protocol.
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 55: Atomicity       -> stateright, dst
 * Line 68: Validity        -> stateright, dst
 * Line 81: Consistency     -> stateright, loom, dst
 * Line 94: Agreement       -> stateright, loom
 * Line 107: TMDecision     -> stateright, dst
 * Line 120: RMSafety       -> stateright, loom, dst
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    RMs,           \* Set of resource manager identifiers
    NULL           \* Null constant for unset values

VARIABLES
    rmState,       \* Map: RM -> {Working, Prepared, Committed, Aborted}
    tmState,       \* TM state: {Init, Preparing, Committed, Aborted}
    tmPrepared,    \* Set of RMs that have sent Prepared to TM
    messages,      \* Set of in-flight messages between TM and RMs
    history        \* Sequence of observable events for verification

vars == <<rmState, tmState, tmPrepared, messages, history>>

-----------------------------------------------------------------------------
(* Message types *)

PrepareMsg   == "Prepare"
PreparedMsg  == "Prepared"
AbortedMsg   == "Aborted"
CommitMsg    == "Commit"
AbortMsg     == "Abort"

-----------------------------------------------------------------------------
(* Type invariant *)

TypeOK ==
    /\ rmState \in [RMs -> {"Working", "Prepared", "Committed", "Aborted"}]
    /\ tmState \in {"Init", "Preparing", "Committed", "Aborted"}
    /\ tmPrepared \subseteq RMs

-----------------------------------------------------------------------------
(* Line 55: Atomicity
 * All resource managers reach the same final decision.
 * No RM commits while another RM aborts.
 *)
Atomicity ==
    \A rm1, rm2 \in RMs :
        ~(/\ rmState[rm1] = "Committed"
          /\ rmState[rm2] = "Aborted")

-----------------------------------------------------------------------------
(* Line 68: Validity
 * A transaction can only commit if all RMs voted to prepare.
 * If any RM chose to abort, the transaction must abort.
 *)
Validity ==
    tmState = "Committed" => tmPrepared = RMs

-----------------------------------------------------------------------------
(* Line 81: Consistency
 * An RM can only be in Committed state if the TM decided to commit.
 * An RM can only be in Aborted state if the TM decided to abort
 * or the RM unilaterally aborted before preparing.
 *)
Consistency ==
    \A rm \in RMs :
        /\ (rmState[rm] = "Committed" => tmState = "Committed")
        /\ (rmState[rm] = "Aborted" =>
                tmState \in {"Aborted", "Init", "Preparing"})

-----------------------------------------------------------------------------
(* Line 94: Agreement
 * Once the TM makes a decision (Commit or Abort), it never changes
 * that decision. The decision is irrevocable.
 *)
Agreement ==
    \* This is a transition property: once tmState is Committed or Aborted,
    \* it stays that way. Encoded as an invariant by checking that if
    \* any RM has committed, the TM must be in Committed state.
    (\E rm \in RMs : rmState[rm] = "Committed") => tmState = "Committed"

-----------------------------------------------------------------------------
(* Line 107: TMDecision
 * The TM only commits if ALL RMs are prepared.
 * The TM aborts if ANY RM failed to prepare.
 *)
TMDecision ==
    /\ (tmState = "Committed" => tmPrepared = RMs)
    /\ (tmState = "Aborted" =>
            \/ tmPrepared # RMs
            \/ \E rm \in RMs : rmState[rm] = "Aborted")

-----------------------------------------------------------------------------
(* Line 120: RMSafety
 * An RM only transitions to Committed after receiving a Commit message.
 * An RM only transitions to Aborted after either choosing to abort
 * locally or receiving an Abort message.
 *)
RMSafety ==
    \A rm \in RMs :
        rmState[rm] = "Committed" =>
            [type |-> CommitMsg, dst |-> rm] \in messages
            \/ tmState = "Committed"

-----------------------------------------------------------------------------
(* Initial state *)

Init ==
    /\ rmState = [rm \in RMs |-> "Working"]
    /\ tmState = "Init"
    /\ tmPrepared = {}
    /\ messages = {}
    /\ history = <<>>

-----------------------------------------------------------------------------
(* TM Actions *)

\* TMPrepare: TM sends Prepare message to all RMs
TMPrepare ==
    /\ tmState = "Init"
    /\ tmState' = "Preparing"
    /\ messages' = messages \cup
        {[type |-> PrepareMsg, dst |-> rm] : rm \in RMs}
    /\ history' = Append(history, <<"TMPrepare">>)
    /\ UNCHANGED <<rmState, tmPrepared>>

\* TMCommit: TM decides to commit (all RMs prepared)
TMCommit ==
    /\ tmState = "Preparing"
    /\ tmPrepared = RMs
    /\ tmState' = "Committed"
    /\ messages' = messages \cup
        {[type |-> CommitMsg, dst |-> rm] : rm \in RMs}
    /\ history' = Append(history, <<"TMCommit">>)
    /\ UNCHANGED <<rmState, tmPrepared>>

\* TMAbort: TM decides to abort
TMAbort ==
    /\ tmState \in {"Init", "Preparing"}
    /\ tmState' = "Aborted"
    /\ messages' = messages \cup
        {[type |-> AbortMsg, dst |-> rm] : rm \in RMs}
    /\ history' = Append(history, <<"TMAbort">>)
    /\ UNCHANGED <<rmState, tmPrepared>>

\* TMRcvPrepared: TM receives Prepared from an RM
TMRcvPrepared(rm) ==
    /\ tmState = "Preparing"
    /\ [type |-> PreparedMsg, src |-> rm] \in messages
    /\ tmPrepared' = tmPrepared \cup {rm}
    /\ messages' = messages \ {[type |-> PreparedMsg, src |-> rm]}
    /\ UNCHANGED <<rmState, tmState, history>>

\* TMRcvAborted: TM receives Aborted from an RM, decides to abort
TMRcvAborted(rm) ==
    /\ tmState = "Preparing"
    /\ [type |-> AbortedMsg, src |-> rm] \in messages
    /\ tmState' = "Aborted"
    /\ messages' = (messages \ {[type |-> AbortedMsg, src |-> rm]}) \cup
        {[type |-> AbortMsg, dst |-> r] : r \in RMs}
    /\ history' = Append(history, <<"TMRcvAborted", rm>>)
    /\ UNCHANGED <<rmState, tmPrepared>>

-----------------------------------------------------------------------------
(* RM Actions *)

\* RMPrepare: RM votes Yes (prepared) in response to Prepare message
RMPrepare(rm) ==
    /\ rmState[rm] = "Working"
    /\ [type |-> PrepareMsg, dst |-> rm] \in messages
    /\ rmState' = [rmState EXCEPT ![rm] = "Prepared"]
    /\ messages' = (messages \ {[type |-> PrepareMsg, dst |-> rm]}) \cup
        {[type |-> PreparedMsg, src |-> rm]}
    /\ history' = Append(history, <<"RMPrepare", rm>>)
    /\ UNCHANGED <<tmState, tmPrepared>>

\* RMChooseToAbort: RM unilaterally decides to abort (before preparing)
RMChooseToAbort(rm) ==
    /\ rmState[rm] = "Working"
    /\ rmState' = [rmState EXCEPT ![rm] = "Aborted"]
    /\ messages' = messages \cup {[type |-> AbortedMsg, src |-> rm]}
    /\ history' = Append(history, <<"RMChooseToAbort", rm>>)
    /\ UNCHANGED <<tmState, tmPrepared>>

\* RMRcvCommitMsg: RM receives Commit from TM
RMRcvCommitMsg(rm) ==
    /\ rmState[rm] \in {"Working", "Prepared"}
    /\ [type |-> CommitMsg, dst |-> rm] \in messages
    /\ rmState' = [rmState EXCEPT ![rm] = "Committed"]
    /\ messages' = messages \ {[type |-> CommitMsg, dst |-> rm]}
    /\ history' = Append(history, <<"RMRcvCommitMsg", rm>>)
    /\ UNCHANGED <<tmState, tmPrepared>>

\* RMRcvAbortMsg: RM receives Abort from TM
RMRcvAbortMsg(rm) ==
    /\ rmState[rm] \in {"Working", "Prepared"}
    /\ [type |-> AbortMsg, dst |-> rm] \in messages
    /\ rmState' = [rmState EXCEPT ![rm] = "Aborted"]
    /\ messages' = messages \ {[type |-> AbortMsg, dst |-> rm]}
    /\ history' = Append(history, <<"RMRcvAbortMsg", rm>>)
    /\ UNCHANGED <<tmState, tmPrepared>>

-----------------------------------------------------------------------------
(* Next state relation *)

Next ==
    \/ TMPrepare
    \/ TMCommit
    \/ TMAbort
    \/ \E rm \in RMs : TMRcvPrepared(rm)
    \/ \E rm \in RMs : TMRcvAborted(rm)
    \/ \E rm \in RMs : RMPrepare(rm)
    \/ \E rm \in RMs : RMChooseToAbort(rm)
    \/ \E rm \in RMs : RMRcvCommitMsg(rm)
    \/ \E rm \in RMs : RMRcvAbortMsg(rm)

-----------------------------------------------------------------------------
(* Fairness *)

Fairness ==
    /\ WF_vars(TMPrepare)
    /\ WF_vars(TMCommit)
    /\ \A rm \in RMs : WF_vars(TMRcvPrepared(rm))
    /\ \A rm \in RMs : WF_vars(TMRcvAborted(rm))
    /\ \A rm \in RMs : WF_vars(RMPrepare(rm))
    /\ \A rm \in RMs : WF_vars(RMRcvCommitMsg(rm))
    /\ \A rm \in RMs : WF_vars(RMRcvAbortMsg(rm))

-----------------------------------------------------------------------------
(* Specification *)

Spec == Init /\ [][Next]_vars

FairSpec == Spec /\ Fairness

-----------------------------------------------------------------------------
(* Properties to check *)

\* Safety: These must always hold
Safety ==
    /\ TypeOK
    /\ Atomicity
    /\ Validity
    /\ Consistency
    /\ Agreement
    /\ TMDecision

\* Liveness: Under fairness, all RMs eventually reach a terminal state
EventualTermination ==
    <>(\A rm \in RMs : rmState[rm] \in {"Committed", "Aborted"})

\* The complete specification with all invariants
FullSpec == FairSpec /\ []Safety

=============================================================================
\* Modification History
\* Created for verified-concurrent distributed systems evolution
\* Based on classic two-phase commit (Gray and Lamport)

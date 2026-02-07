//! Invariant traits for verified concurrent structures.
//!
//! Each module defines the properties that implementations must satisfy,
//! all traceable to TLA+ specifications.
//!
//! ## Lock-free structures
//! - `stack`: Treiber Stack invariants (NoLostElements, NoDuplicates, LIFO)
//! - `ring_buffer`: Ring Buffer invariants (NoLostMessages, FIFO_Order, BoundedCapacity)
//! - `linked_list`: Harris Linked List invariants (NoLostElements, Sorted, NoDuplicates)
//! - `radix_tree`: Radix Tree invariants (PrefixConsistency, NoLostKeys)
//! - `io_buffer`: I/O Buffer invariants (NoDataCorruption, OrderPreserved, BoundedMemory)
//! - `epoch_gc`: Epoch GC invariants (NoUseAfterFree, BoundedMemory, MonotonicEpoch)
//! - `pagecache`: Page Cache invariants (AtomicPageUpdate, NoLostPages, CrashConsistency)
//! - `btree_plus`: B+ Tree invariants (SortedOrder, BalancedHeight, NoLostKeys)
//!
//! ## Lock-based protocols
//! - `ssi`: Serializable Snapshot Isolation (FirstCommitterWins, Serializable)
//! - `cross_shard_ssi`: Cross-Shard SSI (CrossShardAtomicity, Serializable)

pub mod btree_plus;
pub mod cross_shard_ssi;
pub mod epoch_gc;
pub mod io_buffer;
pub mod linked_list;
pub mod pagecache;
pub mod radix_tree;
pub mod ring_buffer;
pub mod ssi;
pub mod stack;

pub use btree_plus::{BTreePlusProperties, BTreePlusPropertyChecker};
pub use cross_shard_ssi::{CrossShardSsiProperties, CrossShardSsiPropertyChecker, CrossShardTxnStatus};
pub use epoch_gc::{EpochGcProperties, EpochGcPropertyChecker};
pub use io_buffer::{IoBufferProperties, IoBufferPropertyChecker};
pub use linked_list::{LinkedListProperties, LinkedListPropertyChecker};
pub use pagecache::{PageCacheProperties, PageCachePropertyChecker, PageState};
pub use radix_tree::{RadixTreeProperties, RadixTreePropertyChecker};
pub use ring_buffer::{RingBufferProperties, RingBufferPropertyChecker};
pub use ssi::{
    check_all as check_all_ssi, first_committer_wins, is_serializable,
    no_committed_dangerous_structures, no_lost_writes, InvariantResult, SsiHistory,
};
pub use stack::{StackHistory, StackOperation, StackProperties, StackPropertyChecker};

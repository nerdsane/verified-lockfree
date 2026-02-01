//! Kani proof harnesses for the Treiber stack.
//!
//! These harnesses use bounded model checking to verify properties
//! of the stack implementation for all possible inputs up to a bound.
//!
//! # Running the proofs
//!
//! ```bash
//! # Run all proofs
//! cargo kani -p vf-examples
//!
//! # Run a specific proof
//! cargo kani -p vf-examples --harness proof_push_preserves_elements
//!
//! # With higher unwind bound (more thorough, slower)
//! cargo kani -p vf-examples --default-unwind 20
//! ```
//!
//! # Note on concurrency
//!
//! Kani doesn't support actual concurrent execution. These proofs verify
//! sequential correctness properties. For concurrent verification, use
//! loom (level 2) or stateright (level 4).

// Conditionally compile only when running under Kani
#[cfg(kani)]
mod proofs {
    use crate::treiber_stack::TreiberStack;

    /// Proof that push never loses elements.
    ///
    /// For any value pushed onto the stack, it must be present
    /// in the stack contents immediately after.
    #[kani::proof]
    #[kani::unwind(5)]
    fn proof_push_preserves_elements() {
        let stack = TreiberStack::new();

        // Symbolic input - Kani explores all possible values
        let value: u64 = kani::any();

        // Constrain to reasonable range to speed up verification
        kani::assume(value > 0 && value < 1000);

        stack.push(value);

        // The value must be in the stack
        let contents = stack.get_contents();
        kani::assert(
            contents.contains(&value),
            "Pushed value must be present in stack contents",
        );
    }

    /// Proof that pop returns the correct value.
    ///
    /// When we push a single value and pop, we must get that value back.
    #[kani::proof]
    #[kani::unwind(5)]
    fn proof_pop_returns_pushed_value() {
        let stack = TreiberStack::new();

        let value: u64 = kani::any();
        kani::assume(value > 0 && value < 1000);

        stack.push(value);
        let popped = stack.pop();

        kani::assert(
            popped == Some(value),
            "Pop must return the value that was just pushed",
        );
    }

    /// Proof of LIFO ordering.
    ///
    /// When we push v1 then v2, popping must return v2 first (last-in, first-out).
    #[kani::proof]
    #[kani::unwind(4)]
    fn proof_lifo_order() {
        let stack = TreiberStack::new();

        let v1: u64 = kani::any();
        let v2: u64 = kani::any();

        // Ensure distinct values
        kani::assume(v1 != v2);
        kani::assume(v1 > 0 && v1 < 100);
        kani::assume(v2 > 0 && v2 < 100);

        stack.push(v1);
        stack.push(v2);

        // Must get v2 first (LIFO)
        let first = stack.pop();
        kani::assert(
            first == Some(v2),
            "Second pushed value must be first popped (LIFO)",
        );

        let second = stack.pop();
        kani::assert(
            second == Some(v1),
            "First pushed value must be second popped (LIFO)",
        );
    }

    /// Proof that empty stack returns None.
    #[kani::proof]
    fn proof_empty_pop_returns_none() {
        let stack: TreiberStack<u64> = TreiberStack::new();
        let result = stack.pop();
        kani::assert(result.is_none(), "Pop on empty stack must return None");
    }

    /// Proof that is_empty is correct.
    #[kani::proof]
    #[kani::unwind(3)]
    fn proof_is_empty_correct() {
        let stack = TreiberStack::new();

        // Empty stack
        kani::assert(stack.is_empty(), "New stack must be empty");

        let value: u64 = kani::any();
        kani::assume(value > 0);

        stack.push(value);
        kani::assert(!stack.is_empty(), "Stack with element must not be empty");

        stack.pop();
        kani::assert(stack.is_empty(), "Stack after pop must be empty");
    }

    /// Bounded proof for a sequence of operations.
    ///
    /// Verifies that no matter what sequence of push/pop operations,
    /// we never pop more than we push.
    #[kani::proof]
    #[kani::unwind(8)]
    fn proof_pop_count_bounded() {
        let stack = TreiberStack::new();

        let mut pushed_count: u64 = 0;
        let mut popped_count: u64 = 0;

        // Up to 5 operations
        for _ in 0..5u8 {
            let is_push: bool = kani::any();

            if is_push {
                let value = pushed_count + 1;
                stack.push(value);
                pushed_count += 1;
            } else if stack.pop().is_some() {
                popped_count += 1;
            }
        }

        // Invariant: popped_count <= pushed_count
        kani::assert(
            popped_count <= pushed_count,
            "Cannot pop more elements than pushed",
        );
    }

    /// Proof that stack size is bounded.
    ///
    /// After N pushes, the stack contains exactly N elements.
    #[kani::proof]
    #[kani::unwind(6)]
    fn proof_size_after_pushes() {
        let stack = TreiberStack::new();

        let n: u8 = kani::any();
        kani::assume(n <= 5);

        for i in 0..n {
            stack.push(i as u64);
        }

        let contents = stack.get_contents();
        kani::assert(
            contents.len() == n as usize,
            "Stack size must equal number of pushes",
        );
    }
}

// When not running under Kani, provide stubs so the module compiles
#[cfg(not(kani))]
mod proofs {
    // Empty - proofs only run under Kani
}

#[cfg(test)]
mod tests {
    // Test that the module compiles (proofs themselves run with cargo kani)
    #[test]
    fn test_proofs_module_exists() {
        // This test just verifies the module structure is valid
        assert!(true);
    }
}

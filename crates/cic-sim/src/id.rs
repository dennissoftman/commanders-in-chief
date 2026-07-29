//! Stable object identifiers from a deterministic counter.
//!
//! The [determinism invariants](../../../docs/invariants/determinism.md) name the two things an
//! identifier must never be: an allocation address, and an insertion position in a hashed container.
//! Both vary per machine, and an identifier that varies per machine is a desync that looks like
//! anything but one — orders target the wrong unit, and every downstream hash differs for a reason
//! nothing reports.
//!
//! So an id is a counter: the first object of a run is `1` on every machine, and identifiers are
//! never reused within a run — a stale order for a dead unit must *miss*, not hit whatever inherited
//! the slot.

use crate::hash::StateHasher;

/// A stable identifier for a simulation object.
///
/// Zero is deliberately never allocated, so `0` is available to formats and diagnostics as "no
/// object" without an `Option` in every serialized layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u64);

impl ObjectId {
    /// The identifier no object ever has.
    pub const NONE: Self = Self(0);
}

/// The deterministic counter identifiers come from.
///
/// Part of simulation state: it is hashed with everything else, so two machines whose allocators
/// drift apart — one spawned an extra object — diverge visibly on the tick it happens.
#[derive(Debug, Clone)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    /// An allocator whose first allocation is `ObjectId(1)`.
    #[must_use]
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Allocates the next identifier.
    pub fn allocate(&mut self) -> ObjectId {
        let id = ObjectId(self.next);
        self.next += 1;
        id
    }

    /// How many identifiers have been allocated.
    #[must_use]
    pub fn allocated(&self) -> u64 {
        self.next - 1
    }

    /// Folds the allocator's state into a tick hash.
    pub fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.next);
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{IdAllocator, ObjectId};

    #[test]
    fn identifiers_count_from_one_and_never_repeat() {
        let mut allocator = IdAllocator::new();
        assert_eq!(allocator.allocate(), ObjectId(1));
        assert_eq!(allocator.allocate(), ObjectId(2));
        assert_eq!(allocator.allocate(), ObjectId(3));
        assert_eq!(allocator.allocated(), 3);
    }

    #[test]
    fn zero_is_reserved_for_no_object() {
        let mut allocator = IdAllocator::new();
        assert_ne!(allocator.allocate(), ObjectId::NONE);
    }

    #[test]
    fn two_allocators_agree_by_construction() {
        // The determinism claim in miniature: the counter's sequence depends on nothing but the
        // number of calls.
        let mut ours = IdAllocator::new();
        let mut theirs = IdAllocator::new();
        for _ in 0..100 {
            assert_eq!(ours.allocate(), theirs.allocate());
        }
    }
}

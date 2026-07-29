//! What a subsystem is, and what one tick hands it.
//!
//! A subsystem is a named piece of simulation state with a step function. The kernel runs every
//! subsystem once per tick, **in registration order** — the order is part of the contract, per the
//! [determinism invariants](../../../docs/invariants/determinism.md), because "movement before
//! combat" and "combat before movement" are different games that both look right in isolation.

use std::any::Any;

use crate::command::Command;
use crate::hash::StateHasher;
use crate::id::IdAllocator;
use crate::random::Streams;

/// Everything one tick offers a subsystem.
///
/// Presentation never holds one of these, which is how "presentation may never advance the
/// simulation" is made structural rather than aspirational: advancing requires a `TickContext`, and
/// only [`Kernel::advance`](crate::kernel::Kernel::advance) constructs them.
pub struct TickContext<'run> {
    /// The tick being computed, counting from zero.
    pub tick: u64,
    /// The fixed length of every tick, in seconds.
    pub tick_seconds: f64,
    /// The identifier counter.
    pub ids: &'run mut IdAllocator,
    /// The named random streams.
    pub streams: &'run mut Streams,
    /// This tick's commands, in arrival order. Subsystems filter for the payloads they own.
    pub commands: &'run [Command],
}

/// A named piece of simulation state with a step function.
pub trait Subsystem {
    /// The name hashes and desync reports know this subsystem by.
    ///
    /// Stable across runs and versions, because it appears in recorded hash streams: renaming a
    /// subsystem invalidates the comparison of every replay recorded before the rename.
    fn name(&self) -> &'static str;

    /// Advances the subsystem by one tick.
    fn tick(&mut self, context: &mut TickContext<'_>);

    /// Folds the subsystem's entire state into the tick hash.
    ///
    /// *Entire* is the requirement that matters: state left out of the hash is state a desync can
    /// hide in, and the report will blame whichever subsystem read the drifted value instead of the
    /// one that held it.
    fn write_state(&self, hasher: &mut StateHasher);

    /// The subsystem as `Any`, so a host can read concrete state out of a snapshot.
    ///
    /// Read is the operative word: snapshots hand out `&dyn Subsystem`, so a host can downcast and
    /// look, and cannot mutate without the `&mut` that only the kernel's own tick path holds.
    fn as_any(&self) -> &dyn Any;
}

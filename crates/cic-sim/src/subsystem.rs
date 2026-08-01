//! What a subsystem is, and what one tick hands it.
//!
//! A subsystem is a named piece of simulation state with a step function. The kernel runs every
//! subsystem once per tick, **in registration order** — the order is part of the contract, per the
//! [determinism invariants](../../../docs/invariants/determinism.md), because "movement before
//! combat" and "combat before movement" are different games that both look right in isolation.
//!
//! # A subsystem reads its peers and mutates only itself
//!
//! One tick hands a subsystem [`Peers`]: every *other* subsystem, immutably. That asymmetry is the
//! whole design. Gameplay is cross-subsystem by nature — movement asks the ground where a unit may
//! walk, combat asks movement where a unit is, a script asks all of them — and the alternatives are
//! worse in ways that are hard to undo later: merging the subsystems loses the per-subsystem hash
//! that names which one drifted, and letting a subsystem *write* to another would make the answer to
//! "who changed this" depend on execution order in a way no hash record could attribute.
//!
//! What a peer read sees is pinned by the order that already exists: **a peer registered earlier has
//! already advanced this tick, and a peer registered later has not.** That is the same contract
//! registration order always carried, now with something that can observe it, so a subsystem that
//! needs this tick's grid must be registered after the grid.
//!
//! The borrow is what enforces it. The kernel splits its own subsystem list around the one it is
//! ticking, so the running subsystem holds `&mut` to itself and `&` to everything else, and there is
//! no way to spell the mutation the rule forbids.

use std::any::Any;

use crate::command::Command;
use crate::hash::StateHasher;
use crate::id::IdAllocator;
use crate::random::Streams;

/// The other subsystems, readable but not mutable, for the length of one tick.
///
/// Constructed only by [`Kernel::advance`](crate::kernel::Kernel::advance), which splits its list
/// around the subsystem it is running — so a subsystem can never reach itself through here, and can
/// never mutate anything it reaches.
pub struct Peers<'run> {
    /// Subsystems registered before the running one, which have already advanced this tick.
    pub(crate) earlier: &'run [Box<dyn Subsystem>],
    /// Subsystems registered after it, which have not.
    pub(crate) later: &'run [Box<dyn Subsystem>],
}

impl<'run> Peers<'run> {
    /// Reads a peer by name, or `None` when nothing is registered under it.
    ///
    /// `None` is a state a subsystem must handle rather than a fault: a kernel assembled without the
    /// ground grid is a legitimate kernel — the replay tests run one — and a caller that treats an
    /// absent peer as impossible turns a host's registration choice into a panic.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'run dyn Subsystem> {
        self.earlier
            .iter()
            .chain(self.later)
            .find(|subsystem| subsystem.name() == name)
            .map(AsRef::as_ref)
    }

    /// Reads a peer by name and concrete type, the way a subsystem actually wants one.
    ///
    /// `None` covers both "nothing registered under that name" and "something else is", because a
    /// caller can do nothing different about them: either way the state it wanted is not there.
    #[must_use]
    pub fn read<T: Any>(&self, name: &str) -> Option<&'run T> {
        self.get(name)?.as_any().downcast_ref::<T>()
    }
}

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
    /// The other subsystems, immutably. See the module documentation for what a read is allowed to
    /// assume about how far through this tick a peer is.
    pub peers: Peers<'run>,
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

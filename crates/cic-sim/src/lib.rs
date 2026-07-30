//! The deterministic fixed-tick simulation kernel: what everything about gameplay and multiplayer
//! rests on.
//!
//! # What is here
//!
//! - [`kernel`] — fixed ticks over ordered subsystems, hashed every step, advanced only through
//!   [`Kernel::advance`].
//! - [`activation`] — a scenario's players and placements constructed into kernel state, in
//!   authored order.
//! - [`units`] — the first verbs: spawn, move, stop, with deterministic straight-line movement.
//! - [`scripts`] — the event dispatcher: a scenario's scripts, in authored order, and the mission
//!   state they keep on this side of the host boundary.
//! - [`subsystem`] — the [`Subsystem`] trait and the [`TickContext`] a tick hands one.
//! - [`command`] — the tick-stamped input stream, recorded so every run is reproducible.
//! - [`random`] — named, versioned, seeded streams; drawing is part of the state transition.
//! - [`id`] — stable object identifiers from a deterministic counter.
//! - [`hash`] — the platform-pinned state hasher desync diagnosis reads.
//! - [`tick`] — the fixed-timestep accumulator, the one presentation-side piece.
//!
//! # The premise
//!
//! A deterministic simulation is a pure function from an initial state and a command stream to a
//! final state. Everything here serves that: the same seed, the same subsystems in the same order,
//! and the same commands must produce identical per-tick hashes on every machine, every run —
//! which is what lockstep multiplayer synchronises on and what a replay file *is*. The property is
//! enforced by CI, not asserted: `tests/replay.rs` runs a scenario twice and requires every hash
//! to match, and the [determinism invariants](../../../docs/invariants/determinism.md) say why each
//! rule exists.
//!
//! # What this crate is not
//!
//! No gameplay: units, orders, combat and economy are M6, built *on* this. No networking: lockstep
//! sessions are M7, built on these hashes. No scenario activation yet — see
//! [the milestone](../../../docs/milestones/m5-simulation.md) for what remains.
//!
//! # Arithmetic
//!
//! Simulation state is bound by [ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md): `f64`
//! restricted to correctly-rounded operations, transcendentals from `cic-math` when a subsystem
//! needs one, and this crate carries decision 8's textual scan in
//! `tests/arithmetic_restriction.rs`. The kernel itself is almost entirely integer — the one `f64`
//! it owns is the tick length, one division fixed at construction.

pub mod activation;
pub mod command;
pub mod hash;
pub mod id;
pub mod kernel;
pub mod random;
pub mod scripts;
pub mod subsystem;
pub mod tick;
pub mod units;

pub use activation::{ActivationError, Forces, Placed, Player, activate};
pub use command::{Command, CommandError, CommandLog, PlayerId};
pub use hash::StateHasher;
pub use id::{IdAllocator, ObjectId};
pub use kernel::{
    Divergence, Kernel, KernelConfig, KernelError, SubsystemHash, TickHashes, first_divergence,
};
pub use random::{Stream, Streams};
pub use scripts::{Mission, ScriptFault, ScriptLoadError, Scripts};
pub use subsystem::{Subsystem, TickContext};
pub use tick::TickAccumulator;
pub use units::{Unit, Units, move_command, spawn_command, stop_command};

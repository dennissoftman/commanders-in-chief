//! The kernel: fixed ticks over ordered subsystems, hashed every step.
//!
//! [`Kernel::advance`] is the only way simulation state moves, and it moves one whole tick at a
//! time. Frame rate does not exist here: a host that renders at 240 Hz and a host that renders at
//! 30 Hz both advance the same ticks with the same commands and hold the same state, which is the
//! entire premise lockstep multiplayer rests on.

use crate::command::Command;
use crate::hash::StateHasher;
use crate::id::IdAllocator;
use crate::random::Streams;
use crate::subsystem::{Subsystem, TickContext};

/// How a kernel is constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelConfig {
    /// The session seed every random stream derives from.
    pub seed: u64,
    /// Ticks per second. The tick length in seconds is `1.0 / ticks_per_second`, computed once —
    /// a single correctly-rounded division, identical everywhere.
    pub ticks_per_second: u32,
}

/// One subsystem's contribution to a tick's hash record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsystemHash {
    /// The subsystem's stable name.
    pub name: &'static str,
    /// Its state hash after the tick.
    pub hash: u64,
}

/// The hash record of one tick: the evidence a desync diagnosis reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickHashes {
    /// The tick these hashes describe.
    pub tick: u64,
    /// The kernel's own state — the id counter and every random stream — followed by each
    /// subsystem, in execution order.
    pub entries: Vec<SubsystemHash>,
    /// All entries folded together with this tick's commands: one number to compare per tick.
    pub combined: u64,
}

/// Where two hash records first disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// The first tick whose records differ.
    pub tick: u64,
    /// The first differing entry's name — which subsystem drifted — or `None` when the records
    /// differ in shape rather than value (different subsystems, different tick counts).
    pub entry: Option<&'static str>,
}

/// Why the kernel refused to advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// A command was stamped for a different tick than the one being advanced.
    CommandForWrongTick {
        /// The tick being advanced.
        advancing: u64,
        /// The tick the command was stamped for.
        stamped: u64,
    },
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandForWrongTick { advancing, stamped } => write!(
                formatter,
                "advancing tick {advancing} but a command is stamped for tick {stamped}"
            ),
        }
    }
}

impl std::error::Error for KernelError {}

/// The deterministic fixed-tick simulation kernel.
pub struct Kernel {
    tick: u64,
    tick_seconds: f64,
    ids: IdAllocator,
    streams: Streams,
    subsystems: Vec<Box<dyn Subsystem>>,
}

impl Kernel {
    /// A kernel at tick zero with no subsystems and no streams.
    ///
    /// # Panics
    ///
    /// Panics if `ticks_per_second` is zero — a simulation that never advances is a configuration
    /// error, not a state.
    #[must_use]
    pub fn new(config: KernelConfig) -> Self {
        assert!(
            config.ticks_per_second > 0,
            "a kernel needs at least one tick per second"
        );
        Self {
            tick: 0,
            tick_seconds: 1.0 / f64::from(config.ticks_per_second),
            ids: IdAllocator::new(),
            streams: Streams::new(config.seed),
            subsystems: Vec::new(),
        }
    }

    /// Registers a random stream. See [`Streams::register`] for the panics and the reasoning.
    pub fn register_stream(&mut self, name: &'static str, version: u32) {
        self.streams.register(name, version);
    }

    /// Adds a subsystem. **Registration order is execution order**, and it is part of the
    /// simulation's contract: reordering subsystems is a change to what the simulation computes,
    /// exactly as editing one of them would be.
    ///
    /// # Panics
    ///
    /// Panics on a repeated name — hash records key on names, so a duplicate would make every
    /// desync report ambiguous about which instance drifted.
    pub fn add_subsystem(&mut self, subsystem: Box<dyn Subsystem>) {
        assert!(
            !self
                .subsystems
                .iter()
                .any(|existing| existing.name() == subsystem.name()),
            "subsystem `{}` is already registered",
            subsystem.name()
        );
        self.subsystems.push(subsystem);
    }

    /// The tick the kernel is about to compute.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The fixed tick length, in seconds.
    #[must_use]
    pub fn tick_seconds(&self) -> f64 {
        self.tick_seconds
    }

    /// Reads a subsystem by name, immutably: the snapshot the interface may look at.
    ///
    /// Presentation reads state through this and can never advance or mutate it — mutation needs
    /// `&mut self`, which the render loop does not hold.
    #[must_use]
    pub fn subsystem(&self, name: &str) -> Option<&dyn Subsystem> {
        self.subsystems
            .iter()
            .find(|subsystem| subsystem.name() == name)
            .map(AsRef::as_ref)
    }

    /// Advances exactly one tick, applying this tick's commands, and returns the hash record.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::CommandForWrongTick`] if any command is stamped for a tick other
    /// than the one being advanced. Refused outright rather than filtered, because a mis-stamped
    /// command reaching a kernel means the layer above lost track of time, and executing *around*
    /// it would turn that bug into a silent input drop — the lockstep failure that produces two
    /// honest machines with two different histories.
    pub fn advance(&mut self, commands: &[Command]) -> Result<TickHashes, KernelError> {
        for command in commands {
            if command.tick != self.tick {
                return Err(KernelError::CommandForWrongTick {
                    advancing: self.tick,
                    stamped: command.tick,
                });
            }
        }

        let mut context = TickContext {
            tick: self.tick,
            tick_seconds: self.tick_seconds,
            ids: &mut self.ids,
            streams: &mut self.streams,
            commands,
        };
        for subsystem in &mut self.subsystems {
            subsystem.tick(&mut context);
        }

        let record = self.hashes(commands);
        self.tick += 1;
        Ok(record)
    }

    /// The hash record for the tick just computed.
    fn hashes(&self, commands: &[Command]) -> TickHashes {
        let mut entries = Vec::with_capacity(self.subsystems.len() + 2);

        let mut ids = StateHasher::new();
        self.ids.write_state(&mut ids);
        entries.push(SubsystemHash {
            name: "kernel.ids",
            hash: ids.finish(),
        });

        let mut streams = StateHasher::new();
        self.streams.write_state(&mut streams);
        entries.push(SubsystemHash {
            name: "kernel.streams",
            hash: streams.finish(),
        });

        for subsystem in &self.subsystems {
            let mut hasher = StateHasher::new();
            subsystem.write_state(&mut hasher);
            entries.push(SubsystemHash {
                name: subsystem.name(),
                hash: hasher.finish(),
            });
        }

        let mut combined = StateHasher::new();
        combined.write_u64(self.tick);
        for command in commands {
            command.write_state(&mut combined);
        }
        for entry in &entries {
            combined.write_str(entry.name);
            combined.write_u64(entry.hash);
        }

        TickHashes {
            tick: self.tick,
            entries,
            combined: combined.finish(),
        }
    }
}

/// Where two runs' hash records first disagree, or `None` if they agree throughout.
///
/// This is the desync diagnosis the per-subsystem split exists for: not "the runs differ" but
/// *which subsystem* drifted and *on which tick* — the difference between rereading one module and
/// rereading the game.
#[must_use]
pub fn first_divergence(ours: &[TickHashes], theirs: &[TickHashes]) -> Option<Divergence> {
    for (our_tick, their_tick) in ours.iter().zip(theirs) {
        if our_tick.tick != their_tick.tick {
            return Some(Divergence {
                tick: our_tick.tick.min(their_tick.tick),
                entry: None,
            });
        }
        for (our_entry, their_entry) in our_tick.entries.iter().zip(&their_tick.entries) {
            if our_entry.name != their_entry.name {
                return Some(Divergence {
                    tick: our_tick.tick,
                    entry: None,
                });
            }
            if our_entry.hash != their_entry.hash {
                return Some(Divergence {
                    tick: our_tick.tick,
                    entry: Some(our_entry.name),
                });
            }
        }
        if our_tick.entries.len() != their_tick.entries.len()
            || our_tick.combined != their_tick.combined
        {
            return Some(Divergence {
                tick: our_tick.tick,
                entry: None,
            });
        }
    }
    if ours.len() != theirs.len() {
        return Some(Divergence {
            tick: ours.len().min(theirs.len()).try_into().unwrap_or(u64::MAX),
            entry: None,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Kernel, KernelConfig, KernelError, first_divergence};
    use crate::command::{Command, PlayerId};
    use crate::hash::StateHasher;
    use crate::subsystem::{Subsystem, TickContext};

    /// The smallest possible subsystem: counts the ticks it has seen.
    struct Counter {
        ticks: u64,
    }

    impl Subsystem for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }

        fn tick(&mut self, _context: &mut TickContext<'_>) {
            self.ticks += 1;
        }

        fn write_state(&self, hasher: &mut StateHasher) {
            hasher.write_u64(self.ticks);
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn kernel() -> Kernel {
        let mut kernel = Kernel::new(KernelConfig {
            seed: 7,
            ticks_per_second: 30,
        });
        kernel.add_subsystem(Box::new(Counter { ticks: 0 }));
        kernel
    }

    #[test]
    fn a_command_stamped_for_another_tick_is_refused() {
        let mut kernel = kernel();
        let command = Command {
            tick: 3,
            player: PlayerId(0),
            payload: Vec::new(),
        };
        assert_eq!(
            kernel.advance(std::slice::from_ref(&command)),
            Err(KernelError::CommandForWrongTick {
                advancing: 0,
                stamped: 3
            })
        );
    }

    #[test]
    fn the_snapshot_reads_state_without_mutating_it() {
        let mut kernel = kernel();
        kernel.advance(&[]).unwrap();
        kernel.advance(&[]).unwrap();
        let counter = kernel
            .subsystem("counter")
            .and_then(|subsystem| subsystem.as_any().downcast_ref::<Counter>())
            .expect("the counter is registered");
        assert_eq!(counter.ticks, 2);
    }

    #[test]
    fn a_duplicate_subsystem_name_is_refused_at_registration() {
        let mut kernel = kernel();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kernel.add_subsystem(Box::new(Counter { ticks: 0 }));
        }));
        assert!(result.is_err());
    }

    #[test]
    fn divergence_reports_the_entry_and_the_tick() {
        let mut ours = kernel();
        let mut theirs = kernel();
        let our_hashes = vec![ours.advance(&[]).unwrap(), ours.advance(&[]).unwrap()];
        let mut their_hashes = vec![theirs.advance(&[]).unwrap(), theirs.advance(&[]).unwrap()];
        assert_eq!(first_divergence(&our_hashes, &their_hashes), None);

        // Corrupt the counter's recorded hash on the second tick, as a drifted machine would.
        their_hashes[1]
            .entries
            .iter_mut()
            .find(|entry| entry.name == "counter")
            .expect("the counter entry exists")
            .hash ^= 1;
        let divergence = first_divergence(&our_hashes, &their_hashes).expect("a divergence");
        assert_eq!(divergence.tick, 1);
        assert_eq!(divergence.entry, Some("counter"));
    }
}

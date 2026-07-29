//! Command recording: the tick-stamped input stream that produced a run.
//!
//! A deterministic simulation is a pure function from an initial state and an input stream to a
//! final state. The kernel is the function; this module is the input stream — and recording it is
//! what makes every run reproducible after the fact, which is what a replay *is* and what a desync
//! report needs before it can say anything.
//!
//! # The payload is opaque here
//!
//! What a command *means* — move these units, set this rally point — is gameplay, which is M6's to
//! define. The kernel needs exactly three facts: **when** (the tick), **who** (the player), and
//! **what bytes** to hand to the subsystems. Keeping the payload opaque keeps the kernel below the
//! game and keeps this format stable while gameplay grows above it.

use crate::hash::StateHasher;

/// A lockstep player slot.
///
/// Not a profile, not a name — the seat at the table. Slot numbers are assigned by the session and
/// are identical on every machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlayerId(pub u8);

/// One tick-stamped input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The tick this command is applied on.
    pub tick: u64,
    /// The seat it came from.
    pub player: PlayerId,
    /// What it says, in an encoding the gameplay layer defines.
    pub payload: Vec<u8>,
}

impl Command {
    /// Folds the command into a tick hash.
    ///
    /// Commands are hashed with the state they produce, so two machines fed different inputs on the
    /// same tick — the lockstep transport's failure, not the simulation's — diverge on that tick's
    /// hash rather than a later one, and the report points at the inputs.
    pub fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.tick);
        hasher.write_bytes(&[self.player.0]);
        hasher.write_u64(self.payload.len() as u64);
        hasher.write_bytes(&self.payload);
    }
}

/// Why a command could not be recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    /// The command's tick precedes one already recorded.
    OutOfOrder {
        /// The tick of the offending command.
        tick: u64,
        /// The latest tick already in the log.
        latest: u64,
    },
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfOrder { tick, latest } => write!(
                formatter,
                "command for tick {tick} recorded after one for tick {latest}"
            ),
        }
    }
}

impl std::error::Error for CommandError {}

/// The recorded input stream of a run.
///
/// Append-only and tick-ordered. Within one tick, commands stay in arrival order — that order is
/// part of the record, because subsystems see the slice in order and a different order is a
/// different run.
#[derive(Debug, Clone, Default)]
pub struct CommandLog {
    commands: Vec<Command>,
}

impl CommandLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a command.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::OutOfOrder`] if the command's tick precedes the latest recorded
    /// tick. Refused rather than sorted, because a log that reorders its input is a log that
    /// cannot testify about what actually arrived.
    pub fn record(&mut self, command: Command) -> Result<(), CommandError> {
        if let Some(last) = self.commands.last()
            && command.tick < last.tick
        {
            return Err(CommandError::OutOfOrder {
                tick: command.tick,
                latest: last.tick,
            });
        }
        self.commands.push(command);
        Ok(())
    }

    /// The commands stamped for one tick, in arrival order.
    #[must_use]
    pub fn for_tick(&self, tick: u64) -> &[Command] {
        let start = self.commands.partition_point(|command| command.tick < tick);
        let end = self
            .commands
            .partition_point(|command| command.tick <= tick);
        &self.commands[start..end]
    }

    /// Every recorded command, in order.
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// The latest tick any command is stamped for, or `None` for an empty log.
    #[must_use]
    pub fn latest_tick(&self) -> Option<u64> {
        self.commands.last().map(|command| command.tick)
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, CommandError, CommandLog, PlayerId};

    fn command(tick: u64, payload: &[u8]) -> Command {
        Command {
            tick,
            player: PlayerId(0),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn a_tick_slice_is_exactly_that_ticks_commands_in_arrival_order() {
        let mut log = CommandLog::new();
        log.record(command(1, b"a")).unwrap();
        log.record(command(3, b"b")).unwrap();
        log.record(command(3, b"c")).unwrap();
        log.record(command(5, b"d")).unwrap();

        assert!(log.for_tick(0).is_empty());
        assert_eq!(log.for_tick(1).len(), 1);
        assert!(log.for_tick(2).is_empty());
        let third = log.for_tick(3);
        assert_eq!(third.len(), 2);
        assert_eq!(third[0].payload, b"b");
        assert_eq!(third[1].payload, b"c");
        assert_eq!(log.latest_tick(), Some(5));
    }

    #[test]
    fn a_command_from_the_past_is_refused_rather_than_sorted() {
        let mut log = CommandLog::new();
        log.record(command(4, b"a")).unwrap();
        let result = log.record(command(2, b"b"));
        assert_eq!(result, Err(CommandError::OutOfOrder { tick: 2, latest: 4 }));
    }

    #[test]
    fn same_tick_commands_are_not_out_of_order() {
        let mut log = CommandLog::new();
        log.record(command(2, b"a")).unwrap();
        assert!(log.record(command(2, b"b")).is_ok());
    }
}

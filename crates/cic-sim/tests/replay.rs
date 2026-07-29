//! The milestone's exit condition, as a test: a recorded command stream replayed against the same
//! initial state reproduces identical per-tick state hashes.
//!
//! The subsystem under simulation is deliberately trivial — wandering points that can be culled by
//! command — because [M5](../../../docs/milestones/m5-simulation.md) says why: a kernel proven
//! deterministic against a trivial subsystem is a better foundation than one debugged alongside the
//! gameplay it runs. What the wanderers *do* exercise is everything the kernel owns: identifier
//! allocation, stream draws, `f64` state accumulated across ticks, command application, and the
//! per-subsystem hashes.

use std::any::Any;
use std::collections::BTreeMap;

use cic_sim::{
    Command, CommandLog, Kernel, KernelConfig, ObjectId, PlayerId, StateHasher, Subsystem,
    TickContext, TickHashes, first_divergence,
};

/// Wandering points: spawn on the first tick, drift by stream draws, die by command.
struct Wanderers {
    /// Positions keyed by id — a `BTreeMap` so the hash folds in a pinned order.
    positions: BTreeMap<ObjectId, (f64, f64)>,
    /// How many to spawn on tick zero.
    spawn: u32,
    /// Draw one extra random number on this tick — the planted bug the divergence test uses.
    extra_draw_on: Option<u64>,
}

impl Wanderers {
    fn new(spawn: u32) -> Self {
        Self {
            positions: BTreeMap::new(),
            spawn,
            extra_draw_on: None,
        }
    }
}

impl Subsystem for Wanderers {
    fn name(&self) -> &'static str {
        "wanderers"
    }

    fn tick(&mut self, context: &mut TickContext<'_>) {
        if context.tick == 0 {
            for _ in 0..self.spawn {
                let id = context.ids.allocate();
                self.positions.insert(id, (0.0, 0.0));
            }
        }

        // A command whose payload is [b'K', low byte of an id] culls that wanderer.
        for command in context.commands {
            if let [b'K', low] = command.payload.as_slice() {
                self.positions.remove(&ObjectId(u64::from(*low)));
            }
        }

        // Everyone drifts by a stream draw, scaled by the tick length: f64 state accumulating
        // across ticks, which is exactly the arithmetic a desync would live in.
        let step = context.tick_seconds;
        for position in self.positions.values_mut() {
            let stream = context.streams.stream("wander");
            position.0 += (stream.next_real() - 0.5) * step;
            position.1 += (stream.next_real() - 0.5) * step;
        }

        if self.extra_draw_on == Some(context.tick) {
            // The planted bug: one draw nobody accounts for.
            let _ = context.streams.stream("wander").next_u64();
        }
    }

    fn write_state(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.positions.len() as u64);
        for (id, (x, y)) in &self.positions {
            hasher.write_u64(id.0);
            hasher.write_f64(*x);
            hasher.write_f64(*y);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A kernel with the wanderers registered, as every run in this file builds it.
fn kernel(extra_draw_on: Option<u64>) -> Kernel {
    let mut kernel = Kernel::new(KernelConfig {
        seed: 0xC1C_5EED,
        ticks_per_second: 30,
    });
    kernel.register_stream("wander", 1);
    let mut wanderers = Wanderers::new(8);
    wanderers.extra_draw_on = extra_draw_on;
    kernel.add_subsystem(Box::new(wanderers));
    kernel
}

/// The command stream every test in this file runs: two culls, ticks apart.
fn recorded_log() -> CommandLog {
    let mut log = CommandLog::new();
    for (tick, id) in [(10, 3_u8), (45, 5_u8)] {
        log.record(Command {
            tick,
            player: PlayerId(0),
            payload: vec![b'K', id],
        })
        .expect("the ticks are recorded in order");
    }
    log
}

/// Runs a kernel for `ticks` ticks against a log, collecting every tick's hashes.
fn run(kernel: &mut Kernel, log: &CommandLog, ticks: u64) -> Vec<TickHashes> {
    (0..ticks)
        .map(|tick| {
            kernel
                .advance(log.for_tick(tick))
                .expect("the log's commands are stamped for the ticks they are fed on")
        })
        .collect()
}

#[test]
fn a_replay_reproduces_every_per_tick_hash() {
    // The exit condition itself. One hundred and twenty ticks, commands landing mid-run, and a
    // second kernel built the same way fed the same log: every tick's every hash must match.
    let log = recorded_log();

    let mut original = kernel(None);
    let original_hashes = run(&mut original, &log, 120);

    let mut replay = kernel(None);
    let replay_hashes = run(&mut replay, &log, 120);

    assert_eq!(first_divergence(&original_hashes, &replay_hashes), None);
    assert_eq!(original_hashes, replay_hashes);
}

#[test]
fn the_commands_are_part_of_the_run_not_decoration() {
    // The same kernel with a different input stream is a different run. Without this, "replay
    // works" could be true of a kernel that ignores its inputs entirely.
    let log = recorded_log();
    let mut with_commands = kernel(None);
    let with = run(&mut with_commands, &log, 60);

    let empty = CommandLog::new();
    let mut without_commands = kernel(None);
    let without = run(&mut without_commands, &empty, 60);

    let divergence = first_divergence(&with, &without).expect("the culls must change the run");
    assert_eq!(divergence.tick, 10, "the first cull lands on tick ten");
}

#[test]
fn one_extra_stream_draw_is_caught_and_attributed() {
    // The desync-diagnosis claim: not merely that the runs differ, but *which* state drifted and
    // *when*. The planted bug draws one extra number on tick fifty; the streams are hashed as
    // kernel state, so the report names them on exactly that tick — long before the positions
    // computed from later draws would have made anything visible.
    let log = recorded_log();
    let mut honest = kernel(None);
    let honest_hashes = run(&mut honest, &log, 120);

    let mut buggy = kernel(Some(50));
    let buggy_hashes = run(&mut buggy, &log, 120);

    let divergence =
        first_divergence(&honest_hashes, &buggy_hashes).expect("the extra draw must diverge");
    assert_eq!(divergence.tick, 50);
    assert_eq!(divergence.entry, Some("kernel.streams"));
}

#[test]
fn a_snapshot_reads_the_wanderers_without_advancing_them() {
    let log = recorded_log();
    let mut kernel = kernel(None);
    let _ = run(&mut kernel, &log, 20);

    let wanderers = kernel
        .subsystem("wanderers")
        .and_then(|subsystem| subsystem.as_any().downcast_ref::<Wanderers>())
        .expect("the wanderers are registered");
    // Eight spawned, one culled on tick ten.
    assert_eq!(wanderers.positions.len(), 7);
    assert_eq!(kernel.tick(), 20, "reading a snapshot advanced nothing");
}

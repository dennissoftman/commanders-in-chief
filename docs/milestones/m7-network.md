# M7: Network

Multiplayer over the deterministic kernel, with desync diagnosis that names the cause.

**Status:** Planned.

## Charter

- Lockstep: each client simulates every tick locally and exchanges only commands, not state.
- Command scheduling with a latency budget, so input is applied a fixed number of ticks ahead and a
  slow link degrades smoothly instead of stuttering.
- Session setup: host, join, map and slot negotiation, and an agreed initial state including every
  random seed.
- Desync detection via the per-tick subsystem hashes from M5, reporting the tick and the subsystem.
- Reconnection, or a clean failure that does not corrupt the other clients' runs.
- Replay of a networked match from the recorded command stream.

## Exit condition

A multi-client match runs to completion with identical per-tick hashes on every client, and a
deliberately injected divergence is reported with its tick and subsystem rather than as a generic
failure.

## Design notes

Lockstep rather than state replication, because the simulation is already deterministic and lockstep's
bandwidth cost is a function of player count rather than of world size. An RTS with thousands of units
cannot afford to replicate state.

Transport rides an established reliability layer over UDP, not raw sockets in either flavour. Raw
TCP is out because one lost packet stalls every byte behind it — head-of-line blocking is the worst
possible property for tick-paced command delivery, and it is what makes TCP's effective throughput
collapse on a lossy link. Raw UDP is out because commands must arrive, and hand-rolling
acknowledgement, retransmission, and ordering means re-earning a decade of someone else's bug fixes.
The candidates are the well-worn ones — QUIC via `quinn`, or the ENet lineage — and the selection is
an ADR when this milestone starts, weighed on dependency surface, pure-Rust preference, and behaviour
as a link degrades.

Desync diagnosis is a first-class deliverable, not a debugging afterthought. A desync that reports only
"clients diverged" costs days; one that reports "pathfinding, tick 4,182" costs minutes. This is the
whole return on M5's per-subsystem hashing, and it is why that work is specified there rather than
being added when the first desync appears.

## Explicitly not done

- No matchmaking, no accounts, no server infrastructure. Direct connection and a lobby are enough to
  prove the model.
- No anti-cheat. Lockstep means every client simulates everything, which makes maphacking possible by
  construction; addressing it is a separate design problem.
- No spectating. It is replay with a live stream, and wants replay to work first.

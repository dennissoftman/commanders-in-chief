# R5: Deterministic simulation kernel

Consume R3's immutable scenario description to introduce fixed 30 Hz ticks, stable runtime IDs,
versioned seeded RNG streams, ordered scheduling, command recording, replay, and subsystem state
hashes. R5 owns player/team activation, spawn assignment, live-object construction, script opcode
dispatch, conditions, actions, timers, and all mutation implied by MAP data. Script support begins
from the raw versioned R3 tree; unsupported actions fail or remain inert deterministically rather
than being guessed. R4 UI may submit typed commands and display immutable snapshots but cannot
execute scripts or own authoritative objects.

**Status:** Planned.

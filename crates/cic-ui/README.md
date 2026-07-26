# cic-ui

Retained, renderer-neutral user-interface state instantiated from immutable WND definitions.

## Responsibilities

- Instantiate an immutable layout into a retained control tree with stable, source-order identity.
- Own presentation state: visibility, enablement, hover, press, focus, selection, text, scroll.
- Answer layout, hit-testing, focus-traversal, and control-invariant questions.
- Hold the shell's screen stack, its separate draw order, and its push/pop shutdown protocol.
- Classify a retained callback name against the original's own function tables, and decide through a
  project-owned allowlist which controls may run a typed action.
- Emit typed UI events and renderer-neutral frames.

## Prohibited dependencies

- Simulation, networking, audio, filesystem, and archive mounting.
- Rendering APIs. A frame is a list of instructions; `cic-render` executes them.
- Retail assets or retail-derived test fixtures.

## Boundaries

- UI state is presentation state, never authoritative game state.
- Callback names are retained as data. Nothing here resolves a name to a function; classification
  reports what the original would have found, and an unknown or `[None]` name is inert.
- Source order controls the tree, focus traversal, and draw submission, and its reverse controls hit
  testing, because that is the order the original's window manager stores windows in. No iteration
  depends on a host hash, a clock, or filesystem order.

# ADR 0003: Renderer boundary and validated shaders

- Status: accepted

## Context

The renderer is the largest subsystem and the one most prone to entangling itself with everything else.
It needs asset data, a camera, a window, and a GPU, and the naive arrangement has it reach directly for
all four.

## Decision

Three boundaries, and one testing rule.

1. **The renderer consumes assets; assets never know about rendering.** The dependency runs
   `cic-assets → cic-render` and never back.
2. **The camera is a standalone model with no window, input, or GPU dependency.** Callers translate their
   own bindings into semantic intents and supply ground heights through a trait.
3. **GPU-independent bookkeeping is separated from GPU work.** Deciding which terrain pages to stage and
   evict for a given view is arithmetic, and lives in a module that can be tested without a device.
4. **Shaders are validated in tests, by the same WGSL front end the GPU backend uses.**

## Rationale

The camera boundary pays for itself three times: the game, the editor, and debug viewers all need a
camera, and only one of them exists yet. Making it depend on a window would mean either duplicating it or
dragging a window system into the editor.

Separating residency bookkeeping from GPU work matters because that logic is where the subtle bugs are —
staging the wrong level of detail, evicting a page still in use, thrashing at a zoom threshold. Those are
all testable as pure functions, and untestable if they are interleaved with device calls.

Shader validation exists because of a specific failure mode: a shader is code, but it is not compiled by
`cargo build`. A copy error or a syntax regression produces a clean build, a green test suite, and a blank
frame. Parsing and validating every shader in a test moves that failure to where it is cheap.

## Consequences

- Shaders are compiled into the binary with `include_str!` rather than loaded from disk, so a build cannot
  disagree with the files next to it.
- The shader front end is a dev-dependency: nothing at runtime needs to parse WGSL, since the GPU backend
  does that itself.
- Residency logic exposes types that no pipeline consumes yet. They are public API rather than
  `#[allow(dead_code)]`, because the bookkeeping is a deliverable in its own right.
- A green test suite is explicitly **not** sufficient verification for a rendering change. Capture-based
  visual regression is a deliverable of [M3](../milestones/m3-renderer.md), not follow-up work.

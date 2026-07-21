# ADR 0004: Renderer boundary and wgpu backend

- Status: accepted
- Date: 2026-07-21

## Context

R2 now exposes immutable, bounded W3D model, hierarchy, animation, and fixed-function material
values. The next gate needs a cross-platform GPU boundary and deterministic headless captures
before an interactive viewer introduces windows, event loops, or wall-clock animation.

## Decision

Add `cic-render` above `cic-formats`. It accepts already validated immutable values, copies them
into stable file-order staging values, and never parses resources or owns filesystem, simulation,
audio, or network state. Pose time and transforms are explicit caller inputs. GPU submissions keep
mesh, pass, stage, and triangle order supplied by the decoded model.

Use `wgpu` 30 with WGSL and the native Vulkan, Metal, and Direct3D 12 backends. This is a safe Rust
API with first-class coverage for the project's Linux, Apple, and Windows targets, and its minimum
Rust version is below the workspace's pinned Rust 1.93. The first gate is surface-free: it renders
to an RGBA8 texture and copies rows into a bounded readback buffer. A window/event-loop dependency
is deferred until the interactive viewer gate demonstrates its ownership boundary.

Backend evidence is the official [`wgpu` 30 crate documentation](https://docs.rs/crate/wgpu/30.0.0),
including its [feature matrix](https://docs.rs/crate/wgpu/30.0.0/features) and native-platform
support table. No upstream renderer source was copied or translated.

The checked-in capture hash covers a project-authored synthetic triangle at an explicit pose. The
capture path clears deterministically, disables blending, depth, and multisampling, uses no clock,
and hashes tightly packed RGBA bytes after removing backend row padding. Tests may skip only when
the host exposes no native or fallback adapter; local milestone verification must record the
adapter and matching hash.

`cic-inspect w3d-render` is the tools-layer bridge from installed profiles or explicit BIG mounts.
It composes and validates the W3D model before constructing `StagedModel`; the renderer receives no
archive handle or path. The initial model capture applies the hierarchy bind pose, uses stable
model/triangle order, orthographically frames bounds, and writes a depth-tested geometry PPM.
Vertex-material colors and diagnostic lighting are an explicit approximation; texture and
fixed-function pass submission remain a later gate.

Capture dimensions are bounded to 4,096 per axis and 64 MiB of padded readback. Staged vertex and
index buffers are independently bounded to 512 MiB, and transformed positions/normals must remain
finite with components no greater than 1,000,000,000 in magnitude before GPU allocation.

## Consequences

- Core, formats, VFS, and simulation remain renderer-independent.
- Headless validation and local visual captures do not require a window system.
- Backend choice is explicit and versioned; a frontend can add a presentation surface later.
- GPU output equality is currently asserted only for the deliberately simple RGBA8 diagnostic.
  Fixed-function blending, textures, hierarchy skinning, and animated installed-model captures
  remain subsequent renderer gates.

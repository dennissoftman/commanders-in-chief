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

The interactive gate adds `winit` 0.30 as a renderer presentation dependency. `cic-tools` retains
VFS/profile/model-composition ownership and passes one already validated immutable model to
`cic-render`; the renderer owns the window, event loop, surface, device, and queue only. Integer
animation frame and Z-up model rotation remain explicit staging inputs. The viewer alone maps
elapsed presentation time to those inputs, so deterministic captures and simulation remain free of
wall-clock state. Surface acquisition handles timeout, occlusion, resize, and outdated states
without mutating decoded data.

The viewer uses a 960x720 window, orthographic auto-fit, and a fixed 45-degree elevated camera.
Left/Right switch clips and reset presentation time. Translation and quaternion channels reuse the
project's glTF preview semantics; scalar rotation channels compose X/Y/Z axis rotations. Legacy
offscreen helper-bone translations use the same model-relative bounded near-zero-scale preview
policy as glTF export. The decoded channels remain unchanged.

Clip selection computes one model-space center and conservative bounding-sphere scale from frame
zero. Every subsequent tick applies only the decoded pose and Z-up model rotation relative to that
fixed framing; it does not recenter or rescale from the current animated bounds. Resizing adjusts
only the projection aspect ratio. Selecting another clip computes a new framing once.

Texture lookup and image decoding remain in `cic-tools`, which resolves only pass-zero/stage-zero
images through the VFS. It skips repeated decoding when aliases resolve to the same virtual path.
The renderer's bounded `TextureResourceManager` validates dimensions and RGBA length, caps each
image and aggregate retained bytes, normalizes aliases, and deduplicates decoded content by
dimensions plus SHA-256 of straight-alpha RGBA bytes. Mesh staging expands triangle corners for
per-face UV indices, preserves triangle order, and deduplicates effective materials by texture ID,
sampler clamp state, alpha test, and blend policy. GPU upload creates each unique sRGB image once;
opaque, source-alpha, and `ONE + ONE` additive pipelines reuse those image views and material bind
groups. Remaining W3D passes, additional stages, and mapper animation stay explicit later gates.

## Consequences

- Core, formats, VFS, and simulation remain renderer-independent.
- Headless validation and local visual captures do not require a window system.
- Backend choice is explicit and versioned; interactive presentation is an opt-in tools workflow.
- The renderer now owns a `winit` presentation surface for the opt-in viewer while headless capture
  remains window-free.
- GPU output equality is currently asserted only for the deliberately simple RGBA8 diagnostic.
  Fixed-function blending, textures, and deterministic animated-pose captures remain subsequent
  renderer gates.

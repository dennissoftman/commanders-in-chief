# Architecture

## Dependency direction

```text
retail files / mods
        |
        v
  cic-vfs <--- archive and directory providers
        |
        v
bounded parsing / immutable format IR
        |                         |
        v                         v
definition database          asset database
        |                         |
        v                         v
commands -> deterministic simulation -> immutable render snapshot
                                  |
                                  v
                           renderer / viewers
```

The current workspace has five deliberately narrow crates:

- `cic-core`: dependency-free invariants and bounded binary input.
- `cic-formats`: bounded decoders and immutable, renderer-neutral format values.
- `cic-vfs`: normalized paths, providers, overlay order, and asset provenance.
- `cic-render`: stable model staging, bounded texture resources, deterministic
  diagnostic capture, and interactive `wgpu` presentation.
- `cic-tools`: diagnostic applications that compose the public VFS, format, and
  renderer APIs.

Simulation, AI, networking, and scripting remain excluded until their milestones begin.

## Boundaries

- VFS providers expose bytes plus provenance; parsers do not inspect physical paths.
- Parsers return immutable semantic values or structured errors.
- Rendering owns GPU/window resources but never parsing, VFS, or simulation state.
- Texture images are bounded and content-addressed; aliases and effective materials reuse
  existing resources without changing stable draw order.
- Tools may format diagnostics but must not contain parsing rules.
- Deterministic behavior is an API property and must be tested at each boundary.

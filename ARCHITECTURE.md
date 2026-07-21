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
```

The current workspace has four deliberately narrow crates:

- `cic-core`: dependency-free invariants and bounded binary input.
- `cic-formats`: bounded decoders and immutable, renderer-neutral format values.
- `cic-vfs`: normalized paths, providers, overlay order, and asset provenance.
- `cic-tools`: diagnostic applications built on public lower-level APIs.

Simulation, rendering, AI, networking, and scripting remain excluded until their
milestones begin.

## Boundaries

- VFS providers expose bytes plus provenance; parsers do not inspect physical paths.
- Parsers return immutable semantic values or structured errors.
- Tools may format diagnostics but must not contain parsing rules.
- Deterministic behavior is an API property and must be tested at each boundary.

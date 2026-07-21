# cic-vfs

Owns normalized virtual paths, resource providers, deterministic mount precedence, and
asset provenance. Later mounts override earlier mounts, while complete provider history
remains available for diagnostics.

BIG archive indexing belongs here because archives are storage providers rather than
semantic asset formats. The parser is bounded, retains table order, and preserves
duplicate-entry history with last-entry-wins resolution.

It may depend on `cic-core`. It must not contain semantic format decoding, rendering,
simulation, or gameplay policy.

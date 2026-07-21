# cic-vfs

Owns normalized virtual paths, resource providers, deterministic mount precedence, and
asset provenance. Later mounts override earlier mounts, while complete provider history
remains available for diagnostics.

It may depend on `cic-core`. It must not contain semantic format decoding, rendering,
simulation, or gameplay policy.


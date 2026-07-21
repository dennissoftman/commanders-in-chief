# cic-formats

Bounded, renderer-neutral decoders for immutable game-format values.

## Responsibilities

- Decode supported files from VFS-provided byte regions.
- Apply explicit limits before allocation and return structured errors.
- Preserve stable file order and unknown-compatible raw values where specified.

## Prohibited dependencies

- Physical filesystems and archive mounting (`cic-vfs` owns those).
- Rendering, audio playback, simulation, networking, and gameplay policy.
- Retail assets or retail-derived test fixtures.

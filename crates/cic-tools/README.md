# cic-tools

Deterministic diagnostic applications over VFS resources and immutable format values.

- `cic-inspect manifest <mount>...` reports resolved resource paths and providers.
- `cic-inspect csf <virtual-path> <mount>...` reports decoded localization records.

Owns user-facing diagnostic programs. Tools compose public VFS and format APIs and may
format reports, but must not duplicate parsing rules or engine behavior.

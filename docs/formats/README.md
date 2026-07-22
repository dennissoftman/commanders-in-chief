# Format Specifications

- `big.md`: BIGF/BIG4 archive directory and payload boundaries.
- `csf.md`: CSF localization headers, labels, strings, and wave names.
- `w3d.md`: recursive W3D chunk framing and preservation rules.
- `map.md`: MAP symbol/chunk framing, implemented terrain/water records, and the complete R3
  object/road/spawn/team/script semantic-gate plan.
- `wnd.md`: source-established WND hierarchy/control vocabulary and the R4 retained UI,
  resource-resolution, rendering, and menu-navigation plan.

Specifications distinguish source-established facts, retail observations, implemented
policy, and open compatibility questions. Synthetic fixtures are original project data.

Each specification records source provenance, observed variants, byte layout, limits,
unknown fields, synthetic fixture construction, and verification status. Implementation
must not precede a bounded specification for untrusted binary input.

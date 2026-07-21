# ADR 0001: GPL license and source provenance

- Status: accepted
- Date: 2026-07-21

## Decision

Original project code is licensed under GPL-3.0-only. Contributors may inspect and derive
work from GPL-licensed Generals/Zero Hour source releases. Any incorporated or translated
work must retain the upstream copyright notice, license, applicable GNU GPL Section 7
terms, upstream repository URL, and exact revision.

Original files must not claim Electronic Arts authorship. Source-derived files must be
marked as modified and must not be presented as original EA files. No trademark or
publicity rights are assumed.

Retail assets are inputs supplied by users and are never committed or redistributed.
Synthetic fixtures must be original and minimal.

## Consequences

- Clean-room separation is not a project requirement.
- Provenance remains mandatory for auditability and license compliance.
- A dependency or source-import review is required before merging copied or translated
  third-party code.
- The project name and UI must remain neutral and clearly unofficial.

This ADR is an engineering policy, not legal advice.


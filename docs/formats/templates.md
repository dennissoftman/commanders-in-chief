# Template set (`templates.json`)

What a `template:` identifier resolves to: the data format defining what a unit, structure, prop, or
faction *is*. JSON, schema version 1.

[M6](../milestones/m6-gameplay.md) deferred this format from M2 on purpose — *written once its
consumers are known* — and it is written now because the first consumers exist: scenario activation
resolves every placement and every player's faction against a set, and a drawing host looks up which
model a placed object wears.

## Example

```json
{
  "format_version": 1,
  "templates": [
    { "id": "prop/pine", "kind": "prop", "model": "models/pine.glb" },
    { "id": "structure/depot", "kind": "structure", "model": "models/depot.glb", "name": "template.depot" },
    { "id": "faction/vanguard", "kind": "faction", "name": "faction.vanguard" }
  ]
}
```

## Fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `format_version` | yes | — | Must be 1. |
| `templates` | yes | — | `id` must be unique and non-blank. |
| `templates[].kind` | yes | — | One of `unit`, `structure`, `prop`, `faction`. |
| `templates[].model` | for placeable kinds | absent | Package-relative `.glb` path. Required for `unit`, `structure`, `prop`; refused for `faction`, which has no pose to draw at. |
| `templates[].name` | no | absent | String-table key for the display name. |

## Deliberately minimal, and how it grows

Health, speed, cost, weapons, footprints: none are here yet, and that is the point rather than an
oversight. A field nothing consumes is a field nothing tests, which is the same argument that deferred
the whole format from M2. Each arrives with the M6 mechanic that reads it. Adding an optional field
later does not break existing files; changing what an existing field means takes a version bump.

## One document, overridden wholesale

The set lives at a well-known path, so the resource layer's ordered mounts apply to it as to any other
file: a map package or a mod providing its own `templates.json` replaces the one mounted beneath it
entirely. Per-template merging across mounts is a modding decision for later, taken deliberately rather
than fallen into — wholesale replacement is at least never surprising.

## Where references are checked

The format validates itself (version, unique ids, model presence by kind). What it cannot check is
whether a *scenario's* references resolve, because the scenario and the set may come from different
mounts — so that check lives in **activation**, the last line before a name becomes kernel state, the
same reasoning that puts scenario-versus-terrain bounds checking in the package loader.

## Unknown fields are rejected

Deliberately, as everywhere in this project: a template set is hand-edited, and a typo in a key should
be a loud error at load rather than a silently-defaulted value that surfaces as a balance bug.

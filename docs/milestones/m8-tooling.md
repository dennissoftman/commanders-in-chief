# M8: Tooling

The tools that let someone who is not the engine's author make content for it.

**Status:** Planned.

## Charter

- A map editor writing the native formats directly: terrain sculpting, texture layer painting, object
  placement, player slots, waypoints.
- Live validation in the editor: the same cross-checks the package loader performs, surfaced while
  editing rather than at load.
- Package building: assemble a map package, with the scenario stored uncompressed so it stays diffable
  inside the archive.
- An asset pipeline: validate and import glTF models, report what an asset uses that the renderer does
  not yet support.
- A command-line inspector for every native format, so a broken file can be diagnosed without the
  editor.

## Exit condition

A map authored entirely in the editor, by someone who has not read the format specifications, loads and
plays.

## Design notes

The editor writes the same formats the engine reads, with no intermediate project format. This costs
some editor convenience and buys something worth more: there is no export step to diverge, and a map
that opens in the editor is a map the engine can load by construction.

Validation is shared code with the package loader, not a reimplementation. Two validators drift, and
the one in the editor is the one that would drift toward permissiveness.

The command-line inspector exists because the editor will eventually fail to open something. A format
whose only reader is a GUI is a format nobody can debug.

## Explicitly not done

- No model or animation authoring. glTF was chosen precisely so existing content tools do that job.
- No in-editor scripting. It follows a scripting layer existing at all.
- No asset store or mod distribution. The resource layer's mount ordering already supports mods being
  installed; distributing them is not an engine problem.

# M4: Interface

A retained user-interface layer: layout, widgets, input routing, and the screen stack the game is
navigated through.

**Status:** In progress. The layout foundation has landed — the format, the solver, the string table, and
the action set. Widget behaviour, input routing, the screen stack, and drawing are still ahead.

## Charter

- A layout model of the project's own design, defined in a text format that is authored and reviewed
  like the scenario format is. **Done** — see [Landed](#landed).
- A widget set covering what an RTS shell actually needs: buttons, labels, lists, sliders, checkboxes,
  text entry, tabs, and a scrollable container. **Kinds exist and position correctly; none of them
  behaves yet.**
- Retained state across frames, so a scroll position or a text cursor survives a redraw.
- Input routing with focus, hover, and keyboard navigation, expressed as semantic events rather than
  raw key codes — the same separation the camera uses. **Hit testing exists; focus and navigation do
  not.**
- A screen stack: push a modal, pop back, and have the screen underneath still be there.
- Resolution and DPI independence, because a fixed-pixel layout is a bug on every display that is not
  the developer's. **Done**, and structural rather than added: a layout is authored in logical units and
  physical pixels exist only on the way out of the solver.

## Landed

- **`cic-ui`**, depending on nothing but `serde`. Free of any window, GPU, or font dependency for the
  reason `cic-camera` is: the same interface model has to serve the game, a map editor, and any debug
  tool, and none of them should inherit a graphics stack by depending on a layout solver.
- **The layout format** — see [the specification](../formats/ui-layout.md). JSON, because the charter
  asks for a format authored and reviewed the way the scenario format is and that one is JSON. Unknown
  fields are rejected, and so are a wrong version, a duplicate id, a negative measurement, and a zero
  fill weight.
  - **Nesting and node count are bounded** at 64 and 4096. Decoding, validation and solving all walk the
    tree recursively, so unbounded nesting is a stack overflow reachable from a data file — an abort
    rather than an error. `serde_json` has a recursion limit of its own, but leaning on a dependency's
    default to enforce this project's invariant would leave the bound unstated and untested.
- **The solver**, in two passes, because `Auto` propagates upward while `Fill` propagates downward and
  one pass cannot do both. Intrinsic sizes bottom-up in logical units, then positions top-down in
  physical pixels.
  - **Text measurement is a caller-supplied trait**, the same device the camera uses for ground height.
    How wide a label is depends on a font, a size and a shaping pass, none of which belong in a layout
    solver — and the trait keeps the solver testable with a stub.
  - **Edges snap to whole pixels; sizes are whatever follows.** Rounding position and size separately
    *introduces* a defect: three columns of a 1000-pixel row land on thirds, and independently rounded
    widths lose a pixel between the second column and the third. Rounding edges keeps a shared edge
    shared. Adjacency survives, an exact width does not, and only adjacency is visible.
  - **The result is flat, in pre-order.** Drawing wants parents first and hit testing wants the topmost
    first, which is the same sequence reversed, so the walk happens once.
  - **Overflow is not an error.** A layout is authored against a range of viewports and the smallest is
    often genuinely too small; refusing to lay out would replace a cramped screen with no screen.
- **A string table**, so no layout file holds display text. Translating is content work for later;
  making it possible is structural and cannot be retrofitted cheaply. A missing key renders as the key
  rather than as blank, because a blank button reads as a rendering bug while `menu.absent` names its own
  fix.
- **A closed action set**, so a layout cannot name an effect the engine does not define. An unknown
  action fails to load rather than failing to find a handler when somebody clicks.

## Remaining

- **Widget behaviour.** The kinds exist and position correctly; nothing toggles, slides, scrolls, or
  accepts a keystroke yet.
- **Retained state across frames**, keyed by the node ids the format already carries — which is why they
  are validated as unique now rather than when something needs them.
- **Input routing**: focus, hover, and keyboard navigation as semantic events. Hit testing is done,
  including the part worth naming — a click resolves to the topmost *activatable* node, because the panel
  beneath a button contains the point too and reporting it would swallow the press.
- **The screen stack**, and the transactional settings apply the design notes below require.
- **Drawing.** `ui.wgsl` is already in the shader set marked `staged`, and the M3 capture harness is what
  will cover the rendered result — which is now worth having, since it runs in CI.

## Exit condition

A navigable shell: main menu, settings with transactional apply-and-rollback, and a skirmish setup
screen that can launch a map. Layout and widget behaviour covered by tests; the rendered result covered
by the M3 capture harness.

**Not met.** The layout half is covered — 44 tests across the format, the solver, the string table and
the action set — and there is no shell yet, so nothing is navigable.

## Design notes

Settings get transactional confirmation and rollback for a specific reason: a display-mode change can
leave a user unable to see the screen well enough to undo it. The commit has to survive a revert timer
rather than depend on the user being able to click.

Callbacks are typed events from an explicit set, not arbitrary handlers looked up by name. A layout
file is data, and data should not be able to name an action the engine did not define.

## Explicitly not done

- No in-game HUD. The shell comes first because it is what makes the engine navigable; the HUD needs
  gameplay in M6 to display.
- No localisation beyond keeping strings out of layout files and in a string table from the start.
  Actually translating is a content task, but making it possible later is a structural one that cannot
  be retrofitted cheaply.

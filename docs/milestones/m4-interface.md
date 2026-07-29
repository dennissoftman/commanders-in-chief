# M4: Interface

A retained user-interface layer: layout, widgets, input routing, and the screen stack the game is
navigated through.

**Status:** In progress. The layout foundation, widget behaviour, and the screen stack with transactional
settings have landed — the format, the solver, the string table, the action set, retained state, input
routing including input-method composition, and a shell that is navigable in tests. Drawing is still
ahead.

## Charter

- A layout model of the project's own design, defined in a text format that is authored and reviewed
  like the scenario format is. **Done** — see [Landed](#landed).
- A widget set covering what an RTS shell actually needs: buttons, labels, lists, sliders, checkboxes,
  text entry, tabs, and a scrollable container. **Behaviour done; none of them draws yet.**
- Retained state across frames, so a scroll position or a text cursor survives a redraw. **Done**, keyed
  by node id, which is why the format requires one on every widget that holds state.
- Input routing with focus, hover, and keyboard navigation, expressed as semantic events rather than
  raw key codes — the same separation the camera uses. **Done**, including input-method composition.
- A screen stack: push a modal, pop back, and have the screen underneath still be there. **Done** — see
  [Landed](#landed).
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

- **Widget behaviour and retained state**, as pure logic against a solved layout.
  - **Semantic input**, not key codes: a caller maps its own devices to `UiEvent`, so a key-binding screen
    changes which key produces `Activate` without any widget learning that happened.
  - **A press arms and a release fires**, and only if the release lands on the same control. Pressing a
    button and letting go somewhere else is how a user cancels after aiming wrong; its absence reads as a
    bug.
  - **State is keyed by node id**, so a scroll offset and a half-typed name survive a resize. Keyed by
    rectangle or by index they would not, since re-solving replaces both — which is why the format now
    *requires* an id on any widget that holds state or takes focus rather than treating it as optional.
  - **Values are not in the layout file.** A slider's range is, because it describes the control; its value
    is not, because it describes whatever the screen is editing, and a layout stating one would be a second
    source of truth for a setting the host owns.
  - **Focus order is reading order**, not screen position. Position would make the tab sequence depend on a
    solved layout, so a resize could silently reorder it.
  - **Everything adjustable is bounded by the layout**: a slider by its own range, a list or tab strip by
    the children it actually has, typed text by `max_length`. Clamped rather than wrapped, because a list
    jumping from its last row to its first on one key press reads as a lost keystroke.
- **Input-method composition**, so Chinese, Japanese and Korean text can be typed at all.
  - A single character per keystroke is the *Latin* case. Under an input method a user types keys that
    produce no text, an uncommitted composition appears and changes as they continue, and only then is text
    committed — possibly several characters at once. That cannot be a sequence of inserts, because the
    composition is replaced rather than appended to.
  - The composition lives *inside* the field as a character range, so a renderer draws one string and marks
    a span of it rather than stitching two together and getting the caret wrong at the join.
  - Two readers, and using the wrong one is a real bug: the text to **draw** includes the composition, the
    field's **value** does not. Saving the former stores a half-formed word as though it were finished.
  - `ime_wanted` and `ime_cursor_area` are what a host drives `set_ime_allowed` and `set_ime_cursor_area`
    from. Without the first, an input method is either off everywhere or on over menus; without the second,
    the candidate window appears in a corner instead of beside the text.
  - **This is why it is here rather than later.** Retrofitting composition means changing the event
    vocabulary, the field's representation, and every renderer that assumed one string with one cursor.
  - The cursor is a **character** index throughout. Byte offsets are what `String` indexes by and what a
    naive implementation reaches for, and they land inside a multi-byte character the first time somebody
    types one — which panics rather than merely looking wrong.

- **The screen stack and the transactional settings** the design notes below call for, plus the shell
  that routes between them.
  - **Each open screen keeps its own retained state**, which is what a stack buys over one current
    screen: closing a modal leaves the menu underneath exactly as it was, and a single current screen
    means rebuilding it from nothing. Rebuilding is not merely wasteful, it is *visible* — everything
    the user had done that the host does not separately own is gone.
  - **A screen appears at most once.** Navigation is by destination, so asking for one already open
    unwinds to it rather than stacking a duplicate nobody can reach. That also removes a bound that
    would otherwise have to be invented: input can push screens, and anything input can grow without a
    limit is a leak reachable from a keyboard. With no duplicates the depth cannot exceed the number of
    screens the engine defines, so the limit is structural rather than a number somebody chose.
  - **A settings apply is undone by a machine, not by a user.** A change goes in force and a 15-second
    window opens; the *absence* of a confirmation is what brings the previous settings back. Confirming
    is one interaction, failing to confirm needs none — which is the only shape of undo that works when
    the user cannot see the screen.
  - **A second apply inside the window keeps the first restore point.** The subtle one: what is worth
    returning to is the last state somebody confirmed, not the previous attempt at replacing it.
    Overwriting it leaves two bad display modes in a row with a restore point holding the first bad one.
  - **Confirming confirms what is in force, not what is staged.** A user can go on editing while the
    countdown runs, and confirming their unapplied edits would put settings into force that nobody had
    seen the effect of — the exact failure this mechanism exists to prevent.
  - **Three rules about leaving**, which is where all the interesting routing turned out to be:
    applying must not move the stack, since the revert window is only useful while the confirm button is
    reachable; closing the settings screen with a change unconfirmed reverts it, since nobody will
    confirm on a screen that is not open; and going back at the root asks whether to leave rather than
    doing nothing, because Escape on the main menu meaning nothing at all reads as a broken key.
  - **Time arrives as an argument.** Nothing here reads a clock, so the whole window is exercised in
    microseconds. Which clock a host passes matters, and it is the one countdown in the engine that must
    **not** be scene time: a display mode producing no frames advances no frame counter, and a revert
    that depends on rendering succeeding cannot fire in the case it exists for.
  - **The outcome of an event is a struct, not an enum.** One action can genuinely do two things a host
    must react to — going back from settings both navigates *and* changes what is in force — and an enum
    would force dropping one of them.

## Remaining

- **Drawing.** `ui.wgsl` is already in the shader set marked `staged`, and the M3 capture harness is what
  will cover the rendered result — which is now worth having, since it runs in CI. Text rendering is the
  substantial part, and it is also what would let `ime_cursor_area` narrow from the field to the caret.
- **A caret-tight IME cursor area**, which needs the text metrics drawing will bring.
- **The layout files themselves.** The shell is navigable against layouts a test constructs; the four
  authored `.ciclayout.json` screens come with drawing, since a screen nobody can see is not reviewable.

## Exit condition

A navigable shell: main menu, settings with transactional apply-and-rollback, and a skirmish setup
screen that can launch a map. Layout and widget behaviour covered by tests; the rendered result covered
by the M3 capture harness.

**Half met.** The shell is navigable and covered — 126 tests across the format, the solver, retained
state, input routing, composition, the string table, the action set, the screen stack, the settings
transaction, and the routing between them. Nothing is drawn yet, so the second half of the condition is
open and so are the authored layouts.

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

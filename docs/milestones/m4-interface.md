# M4: Interface

A retained user-interface layer: layout, widgets, input routing, and the screen stack the game is
navigated through.

**Status:** Exit condition met, and the charter is now complete. The layout foundation, widget behaviour,
the screen stack with transactional settings, drawing, and animated screen changes have all landed, the four
authored screens are covered by the M3 capture harness, and **tabs switch pages** — the one widget that was
half a widget.

## Charter

- A layout model of the project's own design, defined in a text format that is authored and reviewed
  like the scenario format is. **Done** — see [Landed](#landed).
- A widget set covering what an RTS shell actually needs: buttons, labels, lists, sliders, checkboxes,
  text entry, tabs, a dropdown, and a scrollable container. **Done**, behaviour and drawing both. The
  dropdown is past what the charter asked for and was added because a settings screen needs one — see
  [Landed](#landed). `tabs` was the
  exception for a while and was half a widget: it selected, highlighted the chosen tab, and switched
  nothing, which is the failure mode this format's validation exists to prevent everywhere else. Found by
  auditing the charter rather than by anything failing, and now closed — see [Landed](#landed).
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
- **A `combo`: a real dropdown**, which is the control a resolution list, a quality preset and a
  level-of-detail choice all want. A closed box showing one of a set, opening a list over whatever is beneath
  it. Its options cost one row of screen whatever their number, where a `tabs` strip costs a row each.
  - **It breaks the one assumption the flat solved sequence rests on**, which is why it is a widget kind and
    not a convention over panels: a node's place in the sequence is its place in the stacking order. A combo
    early in a screen opens a list over siblings authored *after* it, so drawing in sequence order paints the
    list behind them and hit-testing in reverse hands a click on the list to whatever is behind it. The open
    combo's subtree is therefore named as an **overlay** — drawn last, searched first, clipped to the viewport
    rather than to whatever encloses the control. One name, so the two orderings cannot disagree.
  - **The solver places the options**, below the control and at its width. That is the one arrangement no
    `direction` can express, because the list belongs outside the box that owns it. `padding` is ignored on a
    combo for the same reason — it would inset the list from the control it hangs off.
  - **Whether the list exists at all is state**, so it arrives through the same `Selections` trait a tab
    strip's chosen page does, and a closed combo's options are invisible by the same flag. That is why the
    trait has two methods rather than there being two mechanisms: a third piece of state that decides what is
    on screen would be added in one place, not three.
  - **A click outside dismisses it and reaches nothing beneath.** A dropdown that closed *and* passed the
    click through is the behaviour people complain about. Escape closes the list rather than leaving the
    screen, for the same reason: innermost first.
  - **The wedge is three rectangles.** The primitive set has no triangle and the font this layer cannot reach
    is where an arrow glyph would have come from. It reads as a caret and costs three fills.
  - The chosen option's text is drawn from the option's own node, inset exactly as a text entry's contents
    are — so the value does not appear to move sideways when the list opens under it. Getting that wrong was
    visible immediately and only in a capture.
  - **The row under the pointer is marked, and the chosen row still wins where they are the same.** Two facts
    rather than one: somebody moving down a list has chosen nothing yet, so a control marking only the choice
    looked inert until they clicked. It cannot reuse `hover`, which holds an *id* — a dropdown's rows have
    none, being the combo's own children — so an index into those children is what names one, and it is
    cleared everywhere the list closes, which is five different places.
- **A hit test now agrees with where things are drawn**, which is what made a `list` selectable by pointer.
  It was previously arrow-keys-only, and the reason was recorded as a limitation rather than fixed: a list
  scrolls, so its rows are *drawn* somewhere other than where the layout placed them, and hit-testing the
  placement selects the wrong row.
  - **The limitation was wider than the widget.** Every control inside a scrolled container was hit-tested
    where it was not drawn — a button in a scrolled panel would have been clickable at the wrong place. The
    fix is one field: `SolvedNode::scroll_offset`, the accumulated offset of a node's *enclosing* scrollable
    containers, and `visual_rect()` for the rectangle that follows from it. A container's own offset is
    excluded, because a scrollable box stays where the layout put it and moves its contents.
  - Which makes three pieces of state that decide where a node is on screen rather than how it looks — the
    chosen tab page, the open dropdown, and now the scroll offset — and all three arrive through the same
    `Selections` trait. That is the point of routing them together: the fourth gets added in one place.
  - Verified by breaking it on purpose. With the hit test back on the placed rectangle, a click on the fifth
    row of a list scrolled by two rows reports the third.
- **Applying settings asks in a dialog**, rather than leaving a Keep and a Revert button on the settings
  screen. The screen has Back and Apply; applying puts the change in force and pushes
  `Screen::SettingsConfirm` over it, with the countdown on it.
  - **The question only exists while a change is unconfirmed**, and a button that is inert most of the time
    teaches people to ignore it. A dialog also makes the countdown unmissable and puts the decision in front
    of the user rather than beside the control they were adjusting.
  - **The window running out closes the dialog**, which is not a convenience: this is the case the whole
    mechanism exists for, and somebody who cannot see the screen cannot dismiss a dialog either. So
    `Shell::tick` now takes a `Measure` — a tick can change the stack, and a stack change is a re-solve.
  - **Dismissing the dialog is not answering it.** Escape leaves the change in force with its window still
    running, and the clock decides. Reverting there instead would take a setting away from somebody who was
    still looking at it; refusing to close would trap them in a dialog.
- **The settings screen no longer has a "Profile name" field.** It was a `text_entry` nothing read — there so
  that one screen exercised every widget kind — and a control that does nothing on a player-facing screen is
  exactly what this format's validation refuses everywhere else. The widget kind is still covered by a
  capture, because the skirmish screen's commander field is a real one.
  - If quality *profiles* arrive — low, medium, high — they are **data, not a typed name**: a list of named
    presets in a JSON file, chosen through a `combo`. A player naming their own profile is a different and
    much later feature, and it was never what that field was.
- **Tabs that switch pages.** A `tabs` node's children are its *headers*; its `pages` field names the
  container holding the bodies. Before this, `Widget::Tabs` tracked a number and nothing acted on it — the
  format's own comment said "switches between sibling pages" and nothing did.
  - **The two readings of this were genuinely unsettled, and the one not taken is worth stating.** Either a
    strip's children are the headers and a new field links the pages, or a strip's children are the *pages*
    and the strip is drawn from them — which costs no format field but makes `Tabs` a switcher rather than a
    strip, and would mean the highlight the paint layer draws was highlighting the wrong node. The first was
    taken because a header and a page want different boxes and the format should say which is which.
  - **The pages cannot be the strip's children**, because a header sits in a strip and a page fills the
    body, and no single container arranges both. So the strip names what it switches, and validation checks
    the two agree: three headers over two pages is a screen whose third tab shows nothing, and neither node
    is wrong on its own. The container must also `stack` its pages, since one shows and the rest must
    neither take space nor leave a gap.
  - **Visibility is decided in the solver**, which is the one place state flows *into* layout — through a
    `Selections` trait, so the solver stays testable against a stub exactly as text measurement is. The
    alternative was leaving each consumer to filter, and hit testing, keyboard navigation and drawing all
    read the same solved sequence: one of the three forgetting is a control the user cannot see taking a
    click. A hidden page is still solved, because its rectangles are correct for when its tab is chosen.
  - **So a tab change is a relayout**, exactly as a resize is, and the shell does it: `Shell::handle`
    compares each strip's chosen page before and after an event and re-solves when one moved. That check
    earns its place — a tab strip usually carries no action, so the routing would otherwise have returned
    idle and left the cached layout showing the page the user had just navigated away from. Compared rather
    than done unconditionally, because solving every open screen on every pointer move is what the cached
    layout exists to avoid.
  - **The paint layer skips a page that is not showing** by the same flag, which is what keeps "on screen"
    one answer rather than three. It had to: all the pages overlap, so a walk that drew every node would
    paint the last-authored page over the chosen one.
  - **A pointer names a tab and the keyboard steps it.** A strip is one focusable control, so a release
    inside it is resolved against its own children — without that, a tab strip could only be driven from the
    keyboard and clicking the third tab would select whatever the arrow keys had last left behind. A release
    in the strip's padding or gaps leaves the selection alone rather than jumping.
  - Deliberately *not* extended to a `list`, though the two share the same stored value and the same bound.
    A list scrolls, and its scroll offset is retained state the solved rectangles do not yet carry — so
    hit-testing a scrolled list's rows against unscrolled rectangles would pick the wrong row, confidently.
    A tab strip does not scroll.
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

- **Drawing**, in three parts, split where the mistakes are.
  - **A paint layer with no GPU in it** (`cic_ui::paint`). Which colour a focused button takes, where a
    checkbox's indicator sits, how far along its track a slider's knob is, how a scroll offset moves a
    container's contents — all arithmetic over a solved layout, and all of it testable by asserting on a
    list rather than by capturing an image.
  - **A layout names a role, never a colour.** The same argument the string table makes about text: an
    authored colour is a decision about appearance spread across every screen file. Six roles, three for
    a panel and three for a label, and a role that does not suit its widget is refused.
  - **Colours are sRGB bytes in and linear floats out.** A shader writing to an sRGB target must emit
    linear values; passing `byte / 255` through is what makes every surface too bright, and it is
    invisible in a test that compares numbers to themselves.
  - **The clip travels with every primitive** rather than as a push-and-pop marker, so no consumer has to
    replay a state machine and leak a scissor into the rest of the frame.
- **An authored typeface** (`cic_render::text`), which is the substantial part.
  - **Written here rather than loaded**, and the licence is the reason. A font file is a binary asset with
    its own obligations, and this tree exists to have one set; a *system* font makes the rendered result
    depend on which machine drew it, which a byte-comparison harness cannot tolerate. See
    [LICENSING.md](../../LICENSING.md).
  - **Stroked rather than filled.** Ninety-five glyphs as lines and elliptical arcs on one integer grid,
    given width by measuring each pixel's distance to the nearest stroke. A stroke has no inside, so no
    scanline pass and no winding rule — coverage falling to zero across the last pixel of the half-width
    *is* the antialiasing.
  - **Its limitation is stated, not discovered.** No CJK glyphs, and a character without one draws as a
    hollow box. The composition model in `cic-ui` is unaffected and not wasted: that is the expensive
    thing to retrofit, and a loaded font can go behind the same type.
  - **The atlas declares its sizes.** A lazily-grown one re-uploads its texture mid-frame; declared up
    front, the drawing path only reads.
- **A caret-tight IME cursor area.** `Interface::ime_cursor_area` reports the field because that module
  cannot measure text; `Painter::ime_cursor_area` knows the caret's offset along the string, and on a wide
  field those are a long way apart.
- **The five authored screens**, in `content/ui/`, and the capture tests load *those* rather than fixtures
  — because a fixture can be the bug, twice already in this tree, and a layout written to be photographed
  would go on passing while the screens the game navigates rotted.

- **Animated screen changes** (`cic_ui::transition`), which the charter does not ask for and which the
  screen stack could not have been given from outside.
  - **The stack keeps the departing screen alive**, because `pop` otherwise drops its state the instant
    navigation happens and there would be nothing left to draw on the way out. A host keeping its own copy
    would be duplicating what the stack had just discarded, and that copy goes stale.
  - **The curve eases out, not in and out.** A symmetric curve barely moves for the frames a user is
    deciding whether the interface responded, so it reads as latency even though it finishes at the same
    moment.
  - **A duration of zero is an ordinary case**, and it is two things at once: the default, and what a
    reduce-motion preference maps to. A special path for it would be one nobody exercises.
  - **Input reaches the arriving screen at once and the departing one never**, which falls out of routing
    to the top of the stack. Getting either wrong is a click landing on something fading out, or the
    animation's duration added to the latency of every navigation.
  - **A transition is an opacity and an offset over a primitive list**, applied by the paint layer, so the
    renderer needed no change at all. That was the test of whether it was in the right layer.
  - **A non-finite clock reading completes the change**, which is the *opposite* of the choice the settings
    revert window makes on the same input — because the hazards are opposite. There, never firing leaves
    somebody unable to see; here, never finishing leaves the interface stuck half-faded between two
    screens. Both resolve toward the state the user is not trapped in.

## Remaining

- Nothing. The charter's last open line was `tabs`, and it is closed — see [Landed](#landed).
  - `scroll` is unused by the five authored screens and is deliberately not listed as a gap: it is
    complete — an offset, a clip, a proportional indicator — and it has a capture of its own. Being unused
    is not the same as being unfinished, which is exactly the distinction `tabs` failed for a while.

Two further things noted for later, neither of them M4's:

- **A loaded-font path**, whenever text beyond Latin is needed. The seam is [`Font`], and the licence
  question is answered in [LICENSING.md](../../LICENSING.md).
- **A themed file.** The theme is a struct with a default; making it authored data is a small change and
  nothing yet needs it.

## Exit condition

A navigable shell: main menu, settings with transactional apply-and-rollback, and a skirmish setup
screen that can launch a map. Layout and widget behaviour covered by tests; the rendered result covered
by the M3 capture harness.

**Met.** 198 tests in `cic-ui` cover the format, the solver, retained state, input routing, composition,
tab pages, dropdowns, the string table, the action set, the screen stack, the settings transaction, the paint
layer, screen transitions, and the routing between them; 38 in `cic-render` cover the typeface, the
rasteriser, the atlas, the draw list, and the authored screens' own strings and geometry;
and eight committed reference images cover the rendered result — the main menu, the settings screen with
every widget kind it has, that same screen at one and a half times the pixel density, a modal over the
screen it covers, a scrolled container clipped to itself, a screen change partway through with both screens
drawn, an open dropdown over the rows it covers, and the keep-or-revert dialog over the screen that applied
it.

**And it runs in a window**, which this project treats as a separate obligation: `cargo run -p cic-render
--example shell`. The window opened at a scale of 1.5, which is what prompted the density reference — a
capture at 1.0 cannot show that a theme's sizes were multiplied, an atlas rebuilt, and every quad still
landed on whole pixels.

No authored screen uses a tab strip yet, so none of the six references that existed then moved when tabs
learned to switch pages. That is worth stating rather than leaving to inference: it is the reason this change is covered by
unit tests and by no new capture.

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

# M4: Interface

A retained user-interface layer: layout, widgets, input routing, and the screen stack the game is
navigated through.

**Status:** Planned.

## Charter

- A layout model of the project's own design, defined in a text format that is authored and reviewed
  like the scenario format is.
- A widget set covering what an RTS shell actually needs: buttons, labels, lists, sliders, checkboxes,
  text entry, tabs, and a scrollable container.
- Retained state across frames, so a scroll position or a text cursor survives a redraw.
- Input routing with focus, hover, and keyboard navigation, expressed as semantic events rather than
  raw key codes — the same separation the camera uses.
- A screen stack: push a modal, pop back, and have the screen underneath still be there.
- Resolution and DPI independence, because a fixed-pixel layout is a bug on every display that is not
  the developer's.

## Exit condition

A navigable shell: main menu, settings with transactional apply-and-rollback, and a skirmish setup
screen that can launch a map. Layout and widget behaviour covered by tests; the rendered result covered
by the M3 capture harness.

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

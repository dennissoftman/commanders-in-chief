# Interface layout (`*.ciclayout.json`)

One screen's structure: a tree of boxes, what kind of control each one is, and what activating it asks
the engine to do. JSON, schema version 1.

Authored in **logical units**, never in pixels. A layout file says a button is 32 units tall; how many
pixels that is depends on the display, and [the solver](../../crates/cic-ui/src/solve.rs) is the only
thing that decides.

## Example

```json
{
  "format_version": 1,
  "root": {
    "id": "main_menu",
    "width": { "fill": 1 },
    "height": { "fill": 1 },
    "direction": "column",
    "justify": "center",
    "align": "center",
    "gap": 12.0,
    "padding": { "left": 32.0, "top": 32.0, "right": 32.0, "bottom": 32.0 },
    "children": [
      { "widget": "label", "text_key": "menu.title" },
      {
        "id": "skirmish",
        "widget": "button",
        "width": { "fixed": 240.0 },
        "height": { "fixed": 40.0 },
        "text_key": "menu.skirmish",
        "action": "open_skirmish_setup"
      },
      {
        "id": "quit",
        "widget": "button",
        "width": { "fixed": 240.0 },
        "height": { "fixed": 40.0 },
        "text_key": "menu.quit",
        "action": "quit"
      }
    ]
  }
}
```

## Node fields

| Field | Required | Default | Notes |
|---|---|---|---|
| `id` | **sometimes** | absent | Unique within a layout. **Required** on any widget that takes focus or holds state — see below. |
| `widget` | no | `"panel"` | One of the widget kinds below. |
| `style` | no | absent | A role, not an appearance — see *Style roles*. Only on a `panel` or a `label`. |
| `direction` | no | `"column"` | `row`, `column`, or `stack`. Ignored by a node with no children. |
| `width`, `height` | no | `"auto"` | See *Sizing*. |
| `padding` | no | all zero | `{ "left", "top", "right", "bottom" }`, logical units. |
| `gap` | no | `0.0` | Space between adjacent children, logical units. |
| `align` | no | `"stretch"` | Cross-axis placement: `start`, `center`, `end`, `stretch`. |
| `justify` | no | `"start"` | Main-axis distribution: `start`, `center`, `end`, `space_between`. |
| `text_key` | no | absent | Key into the string table. **Never literal text.** |
| `action` | no | absent | One of the closed action set. Only on an activatable widget. |
| `range` | **on a slider** | — | `{ "min", "max", "step" }`. Only on a `slider`, and required there. |
| `max_length` | no | `256` | Longest text accepted. Only on a `text_entry`. |
| `children` | no | `[]` | In drawing and navigation order. |

## Widget kinds

`panel`, `label`, `button`, `checkbox`, `slider`, `text_entry`, `list`, `tabs`, `scroll`.

A kind decides three things beyond how it draws, and the layout is validated against all three:

| Kind | May carry an `action` | Takes focus | Holds state | Needs an `id` |
|---|---|---|---|---|
| `panel`, `label` | no | no | no | no |
| `button` | yes | yes | no | **yes** |
| `checkbox` | yes | yes | yes | **yes** |
| `slider` | no | yes | yes | **yes** |
| `text_entry` | no | yes | yes | **yes** |
| `list`, `tabs` | yes | yes | yes | **yes** |
| `scroll` | no | no | yes | **yes** |

**Why an id is required rather than optional.** Retained state is keyed by id — that is what makes a
scroll offset or a half-typed name survive a window resize, since every rectangle and every index changes
when the layout is re-solved. Focus is named the same way. A checkbox with no id cannot remember whether
it is checked, so the file is refused at load rather than silently forgetting at run time.

**Why `scroll` takes no focus.** It holds an offset, but nothing inside it *is* it, so landing keyboard
focus on the container would give the user a tab stop where no key does anything. Scrolling follows the
pointer.

## Style roles

`style` says what a node **is**, never what it looks like. A theme decides the second, for the same
reason text lives in a string table: an authored colour is a decision about the interface's appearance
spread across every screen file, and changing it means finding every literal.

| Role | On | Draws |
|---|---|---|
| `scrim` | `panel` | A translucent wash over everything beneath, for a modal's backdrop. |
| `card` | `panel` | A raised surface with a border: a modal's body, or a settings page. |
| `divider` | `panel` | A hairline rule. |
| `title` | `label` | Larger text, centred unless `align` says otherwise. |
| `caption` | `label` | Smaller, dimmer text. |
| `warning` | `label` | Text that wants attention, such as a countdown about to expire. |

**A node with no role draws nothing.** That is the common case and it is deliberate: a layout tree is
mostly structure — rows and columns whose job is to place their children — and giving every panel a
background would paint the screen over in nested rectangles.

**A widget kind that already looks like something takes no role.** How a button, a slider or a checkbox
looks is not a per-node decision, so a role on one is refused: a surface role on a label or a text role on
a panel would draw nothing, which is the same class of mistake as an action on a panel.

**Text alignment comes from `align` on a childless node.** `align` defaults to `"stretch"`, and a node
with no children has nothing to stretch — so `"center"` on a label can only mean the text, while a
defaulted `"stretch"` leaves the widget kind to decide. That is what centres a button's label without
every button having to say so.

**Dynamic text is a stored value, not a layout field.** A label with an `id` draws whatever the host
stored against that id, falling back to its `text_key`. That is the channel for text no string table can
hold — a countdown, a chosen map's name — and it keeps per-frame values out of the table.

## Actions

`action` names one of a closed set. A layout naming anything else fails to load, so a typo is a load
error rather than a control that silently never fires.

| Action | What it does |
|---|---|
| `quit` | Leave the game. |
| `back` | Close the top screen. At the root, the shell asks whether to leave instead. |
| `open_main_menu` | Show the main menu. |
| `open_settings` | Show settings. |
| `open_skirmish_setup` | Show skirmish setup. |
| `open_quit_confirm` | Ask whether to leave the game. |
| `apply_settings` | Put the staged settings in force and start the revert countdown. |
| `confirm_settings` | Keep what is in force, ending the countdown. |
| `revert_settings` | Discard staged edits, and take back an applied change that is still unconfirmed. |
| `launch_skirmish` | Start the configured skirmish. |

**Why `open_quit_confirm` and `quit` are both here.** A main menu's exit button carries the first and the
modal's confirm button carries the second. One action meaning "ask" on one screen and "do it" on another
is exactly the context-dependence a closed set exists to avoid.

**The four navigation actions correspond one-to-one with the screens**, which is what lets `Screen`
be derived from an action rather than mapped by a table somebody has to keep in step.

## Screens and the stack

Each screen is one layout file, named after the screen: `main_menu.ciclayout.json`,
`settings.ciclayout.json`, `skirmish_setup.ciclayout.json`, `quit_confirm.ciclayout.json`. A catalogue
missing any of them is refused when the shell is built, because a screen with no layout is a button that
navigates to a blank surface — which reads as an unfinished screen rather than as a missing file.

Open screens form a stack, and each one keeps its own retained state while something sits on top of it.
That is the point of a stack rather than one current screen: closing a modal has to leave the screen
underneath exactly as it was, and a single current screen means rebuilding it from nothing.

**A screen appears at most once.** Navigation is by destination, so asking for a screen that is already
open unwinds to it rather than stacking a second copy — main menu → settings → main menu returns to the
menu that was already there. That also removes a bound that would otherwise have to be invented: input
can push screens, and with no duplicates the depth cannot exceed the number of screens the engine
defines.

Closing a screen discards what it held, which is why reopening settings shows what is in force rather
than the edits somebody walked away from.

## Settings are applied, then confirmed

A display change can leave the person who made it unable to see the screen well enough to undo it — a
resolution the monitor cannot sync to, a scale that puts the buttons off the panel, a full-screen mode
that comes up black. In all of those the interface is still there, still listening, and unreachable. So
an undo that depends on the user clicking is not an undo.

`apply_settings` therefore puts the change in force and opens a **15-second window**. Confirming closes
it and keeps the change; doing nothing closes it and takes the change back. Three rules follow, and each
one is a way of getting *leaving* right:

- **Applying does not move the stack.** The revert window is only useful while the confirm button is
  somewhere the user can reach.
- **A second apply inside the window keeps the first restore point.** What is worth returning to is the
  last state somebody confirmed, not the previous attempt at replacing it — otherwise two bad display
  modes in a row leave the restore point holding the first bad one.
- **Closing the settings screen with a change unconfirmed reverts it.** Nobody is going to confirm a
  change on a screen that is no longer open, and a user who navigated away already said what they meant.

`confirm_settings` confirms what is **in force**, not what is staged. A user can go on editing while the
countdown runs, and confirming their unapplied edits would put settings into force that nobody had seen
the effect of — the exact failure the mechanism exists to prevent.

The countdown is advanced by the host, in seconds, through `Shell::tick`. Nothing here reads a clock:
a countdown that read one could not be tested without waiting, and a test that waits is flaky. Which
clock a host passes matters — this is the one countdown in the engine that must **not** come from scene
time, because a display mode that produces no frames also advances no frame counter, and a revert that
depends on rendering succeeding cannot fire in the case it exists for.

## Sizing

Per axis, one of three:

| Form | Meaning |
|---|---|
| `"auto"` | As large as the content needs — children for a container, measured content for a leaf. |
| `{ "fixed": 240.0 }` | Exactly that many logical units. |
| `{ "fill": 3 }` | A share of the parent's leftover space, proportional to the weight. |

**Weights rather than percentages.** A percentage of a box whose siblings are `auto` is undefined until
those siblings are measured, and weights compose without an author having to keep a set of numbers adding
to a hundred. A row of `{ "fill": 1 }` and `{ "fill": 3 }` splits one-quarter to three-quarters.

A weight of zero is refused: it asks for a share of nothing, which is `{ "fixed": 0 }` said confusingly,
and refusing it means every division in the solver has a positive divisor.

## Validation

Beyond what the shape enforces:

- `format_version` must be 1.
- An `id`, where present, must be unique across the layout and not blank; where the widget takes focus or
  holds state, it must be present.
- Every measurement — a fixed size, a gap, each padding side — must be finite and not negative.
- A fill weight must be positive.
- An `action` on a widget that cannot be activated is refused. Otherwise the control looks correct and
  silently never fires, which is the hardest class of authoring mistake to notice.
- A `slider` must declare a `range`, and that range must be one it can move over: `max` strictly above
  `min`, and a positive `step`. A collapsed range is a slider that cannot move and puts a division by zero
  one arithmetic step away.
- `range` on anything but a slider, or `max_length` on anything but a text entry, is refused — the same
  posture as an action on a panel, since inert authoring is a mistake that looks correct.
- A `style` role must suit its widget: a surface role only on a `panel`, a text role only on a `label`,
  and neither on a kind that already looks like something.
- `max_length` may not be zero.
- Nesting is limited to 64 levels and a layout to 4096 nodes. The tree is walked recursively by
  decoding, validation, and solving alike, so unbounded nesting is a stack overflow reachable from a
  data file — an abort rather than an error, which the
  [binary-parsing invariant](../invariants/binary-parsing.md) forbids.

## Two things a layout file may not contain

Both are structural, both are cheap now and expensive later.

**No literal display text.** A node names a `text_key` and the text lives in a string table. Translating
is content work nobody is asking for yet; *making it possible* is structural, and the alternative is
hunting for string literals spread through every layout file, looking exactly like the strings that must
not be translated.

**No named handlers.** `action` deserialises into a closed enum, so a layout naming an effect the engine
does not define fails to load. The alternative — a handler name looked up at activation time — defers a
typo from load to the moment a user clicks, and once mods can supply layouts it is an open channel into
whatever the lookup table happens to hold.

## Why JSON

The same three reasons the [scenario format](scenario.md) is JSON, and the charter asks for a format
authored and reviewed the way that one is. A layout is **diffable**, so a review can see that a button
moved; `git blame` attributes a change; and a broken layout is repairable in a text editor when the tool
that wrote it has a bug.

A bespoke layout language would read better in places and would cost a hand-written parser, its own error
reporting, and its own documentation, for no property anybody needs. Output is pretty-printed with a
trailing newline, because the file exists to be read.

## What a host has to wire

Input arrives as **semantic events**, never as key codes — the same separation the camera uses. A caller
translates its own devices into `UiEvent`, so nothing in `cic-ui` knows about `winit` or scan codes, and a
key-binding screen changes which key produces `Activate` without any widget knowing that happened.

Two things a host must drive that are easy to miss, both of them about text input:

**An input method needs to be told when to turn on.** `Interface::ime_wanted` is true exactly while a
`text_entry` holds focus; drive `set_ime_allowed` from it. Left on everywhere, a candidate window can
appear over a menu; left off, Chinese, Japanese and Korean text cannot be typed at all.

**An input method needs to be told where the text is.** `Interface::ime_cursor_area` reports the focused
field's rectangle for `set_ime_cursor_area`, so the candidate list appears beside what is being typed
rather than in a corner. The rectangle is the whole field, because a caret-tight one needs text metrics
this crate deliberately does not have.

Composition itself is three events rather than a stream of characters, because a composition is *replaced*
as it grows rather than appended to, and because it has to be drawn differently — conventionally
underlined — so a user can see what is not yet real:

| Event | From `winit` | Meaning |
|---|---|---|
| `Compose { text, cursor }` | `Ime::Preedit` | The whole current composition, replacing any previous one. Empty withdraws it. |
| `Commit(text)` | `Ime::Commit` | Finished text, possibly several characters at once. |
| `ComposeCancelled` | `Ime::Disabled` | The composition was abandoned; committed text is untouched. |

A `TextField` therefore has two readers, and using the wrong one is a real bug: `text()` is what to
**draw**, composition included, and `committed()` is the field's **value**. Saving `text()` would store a
half-formed word as though the user had finished typing it.

## What the solver guarantees

- **Edges land on whole pixels, sizes are whatever follows.** Rounding position and size separately
  introduces seams: three columns of a 1000-pixel row land on thirds, and independently rounded widths
  lose a pixel between the second and the third. Rounding *edges* means a shared edge stays shared.
- **Overflow is not an error.** Children too large for their parent overflow it and the rectangles say so.
  A layout is authored against a range of viewports and the smallest is often genuinely too small;
  refusing to lay out would replace a cramped screen with no screen. Clipping is the drawing layer's call.
- **Nothing reads a clock, a font, or a device.** Text measurement arrives through a caller-supplied
  trait, the same way the camera takes ground height, so the solver is testable with a stub and drags no
  font library into a crate that otherwise needs none.

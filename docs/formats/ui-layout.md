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
| `pages` | no | absent | Id of the container this tab strip switches. Only on a `tabs` — see *Tabs*. |
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

## Tabs

A `tabs` node's own children are the **headers**, one per tab, in order. Its `pages` field names the
container holding the bodies:

```json
{
  "children": [
    {
      "id": "settings_tabs",
      "widget": "tabs",
      "direction": "row",
      "height": { "fixed": 32.0 },
      "pages": "settings_pages",
      "children": [
        { "widget": "label", "text_key": "settings.display" },
        { "widget": "label", "text_key": "settings.audio" }
      ]
    },
    {
      "id": "settings_pages",
      "direction": "stack",
      "width": { "fill": 1 },
      "height": { "fill": 1 },
      "children": [
        { "id": "display_page", "children": [] },
        { "id": "audio_page", "children": [] }
      ]
    }
  ]
}
```

**Why the pages are not the strip's children.** A header sits in a strip and a page fills the body, and no
single container arranges both — one is a row of small boxes and the other is one large box. So the strip
names what it switches, and the two are validated against each other.

**Why the container must `stack`.** Pages occupy the same box: one shows and the rest must neither take
space nor leave a gap where they would have been. A column of pages lays out without error and is visibly
wrong, so the arrangement is checked at load.

**What the selection does.** Only the chosen page is on screen; every other page and everything inside it
is skipped by hit testing, by keyboard navigation, and by drawing. A page's rectangles are still solved —
they are correct for when its tab is chosen — but nothing on a hidden page can be clicked or focused, which
is what stops a control the user cannot see from stealing a tab stop.

**A tab change is a relayout**, exactly as a resize is. The selection is state, so a host passes its
`Interface` to `solve_selected` and re-solves after handling input, which is what it already does per
frame. Nothing chosen yet means the first tab.

**Clicking a header picks it.** A tab strip is a single focusable control, so a release inside it is
resolved against the strip's own children; a release in the strip's padding or gaps leaves the selection
alone rather than jumping. Left and right — `Adjust` — step it from the keyboard, clamped to the tabs that
exist rather than wrapping.

**A `tabs` without `pages` is a segmented picker** whose selection some other screen reads, which is why
`pages` is optional.

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
- `range` on anything but a slider, `max_length` on anything but a text entry, or `pages` on anything but a
  tab strip is refused — the same posture as an action on a panel, since inert authoring is a mistake that
  looks correct.
- `max_length` may not be zero.
- A `pages` container must exist, must `stack` its children, must hold exactly as many as the strip has
  headers, and must not be the strip itself or anything inside it. The count is the check worth having:
  three headers over two pages is a screen whose third tab shows nothing, and neither node is wrong on its
  own. Naming a container inside the strip would make each page one of the strip's own headers, so
  selecting a page would change what "it" is.
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

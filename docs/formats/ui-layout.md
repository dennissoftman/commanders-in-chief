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
| `id` | no | absent | Unique within a layout. Names the node for retained state and focus. |
| `widget` | no | `"panel"` | One of the widget kinds below. |
| `direction` | no | `"column"` | `row`, `column`, or `stack`. Ignored by a node with no children. |
| `width`, `height` | no | `"auto"` | See *Sizing*. |
| `padding` | no | all zero | `{ "left", "top", "right", "bottom" }`, logical units. |
| `gap` | no | `0.0` | Space between adjacent children, logical units. |
| `align` | no | `"stretch"` | Cross-axis placement: `start`, `center`, `end`, `stretch`. |
| `justify` | no | `"start"` | Main-axis distribution: `start`, `center`, `end`, `space_between`. |
| `text_key` | no | absent | Key into the string table. **Never literal text.** |
| `action` | no | absent | One of the closed action set. Only on an activatable widget. |
| `children` | no | `[]` | In drawing and navigation order. |

Widget kinds: `panel`, `label`, `button`, `checkbox`, `slider`, `text_entry`, `list`, `tabs`, `scroll`.
Of these, `button`, `checkbox`, `list` and `tabs` may carry an `action`.

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
- An `id`, where present, must be unique across the layout and not blank.
- Every measurement — a fixed size, a gap, each padding side — must be finite and not negative.
- A fill weight must be positive.
- An `action` on a widget that cannot be activated is refused. Otherwise the control looks correct and
  silently never fires, which is the hardest class of authoring mistake to notice.
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

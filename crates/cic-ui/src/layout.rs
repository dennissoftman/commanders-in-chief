//! The authored layout tree, and its text format.
//!
//! # Why JSON, and why "like the scenario format"
//!
//! The charter asks for a text format authored and reviewed the way the scenario format is, and the
//! scenario format is JSON — so this is JSON, with the same three properties that decision bought
//! there: it is diffable, so a review can see that a button moved; `git blame` attributes a change to
//! whoever made it; and a broken layout is repairable in a text editor when the tool that wrote it has
//! a bug. A bespoke layout language would read better in a few places and would cost a hand-written
//! parser, its own error reporting, and its own documentation, for no property anybody needs.
//!
//! Unknown fields are rejected, for the reason the scenario format rejects them: a layout is
//! hand-edited, and a mistyped key that silently defaults is a defect discovered by a player rather
//! than by a loader.
//!
//! # What a layout file may not contain
//!
//! Two things, both structural, both cheap now and expensive later.
//!
//! **No literal display text.** A node carries a `text_key` resolved against a
//! [`StringTable`](crate::StringTable). See that module for why a key beats a string even before
//! there is a second language.
//!
//! **No named handlers.** A node's `action` deserialises into [`Action`], a closed enum, so a layout
//! cannot name an effect the engine does not define. This is the layering rule the crate exists
//! under, not a preference.
//!
//! # Why depth and node count are bounded
//!
//! The tree is recursive, and so are decoding, validation, and solving. Unbounded nesting is
//! therefore a stack overflow reachable from a data file, which is an abort rather than an error and
//! is exactly what the project's bounded-parsing invariant forbids. `serde_json` has a recursion
//! limit of its own, but relying on a dependency's default to enforce this crate's invariant would
//! leave the bound unstated and untested — so [`MAX_DEPTH`] and [`MAX_NODES`] are checked here and
//! have their own tests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Action;
use crate::geometry::Insets;

/// The layout schema version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Deepest nesting accepted, counting the root as depth one.
///
/// Far above any interface anybody would author — a settings screen reaches about six — and far below
/// what recursion can survive.
pub const MAX_DEPTH: usize = 64;

/// Most nodes accepted in one layout.
pub const MAX_NODES: usize = 4_096;

/// How a node's extent along one axis is decided.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    /// As large as the content needs: the children for a container, the measured content for a leaf.
    ///
    /// The default, because a node that says nothing about its size should take the space it needs
    /// rather than silently claim the whole axis.
    #[default]
    Auto,
    /// Exactly this many logical units.
    Fixed(f32),
    /// A share of whatever the parent has left over, proportional to this weight.
    ///
    /// Weights rather than percentages because a percentage of a box whose siblings are `Auto` is
    /// undefined until those siblings are measured, and because weights compose without the author
    /// having to make a set of numbers add to a hundred.
    Fill(u32),
}

/// Which way a container arranges its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Left to right.
    Row,
    /// Top to bottom. The default, because a menu is a column.
    #[default]
    Column,
    /// All children occupy the whole box, drawn in order.
    ///
    /// What a modal, a tooltip, and a background image behind a panel all need.
    Stack,
}

/// Where a child sits on the axis it does not consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Against the leading edge.
    Start,
    /// Centred.
    Center,
    /// Against the trailing edge.
    End,
    /// Filling the axis. The default, because a row of buttons wants equal heights.
    #[default]
    Stretch,
}

/// How leftover space on the main axis is distributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    /// Packed at the leading edge.
    #[default]
    Start,
    /// Packed in the middle.
    Center,
    /// Packed at the trailing edge.
    End,
    /// Spread, with equal gaps between children and none at the ends.
    SpaceBetween,
}

/// What kind of control a node is.
///
/// The set the charter names for an RTS shell. A kind decides three things beyond how it draws: whether
/// an [`Action`] on it means anything, whether it can take focus, and whether it remembers anything
/// between frames. All three are answered here rather than in the interaction code, so a layout can be
/// *validated* against them at load — which is what turns "a checkbox with no id" from a control that
/// silently forgets its value into a refused file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    /// A container that draws a background and nothing else.
    #[default]
    Panel,
    /// Text.
    Label,
    /// Activates once per press.
    Button,
    /// Toggles a boolean.
    Checkbox,
    /// Picks a number in a range.
    Slider,
    /// Accepts typed text and holds a cursor.
    TextEntry,
    /// A scrollable set of selectable rows.
    List,
    /// Switches between sibling pages.
    Tabs,
    /// Clips its child and holds a scroll offset.
    Scroll,
}

impl Widget {
    /// Whether attaching an [`Action`] to this widget means anything.
    ///
    /// A layout naming an action on a `Panel` is a typo that would otherwise do nothing at all, which
    /// is the hardest kind of mistake to notice: the control looks right and simply never fires.
    #[must_use]
    pub const fn activatable(self) -> bool {
        matches!(
            self,
            Self::Button | Self::Checkbox | Self::List | Self::Tabs
        )
    }

    /// Whether keyboard navigation stops here.
    ///
    /// A `Scroll` is deliberately absent: it holds an offset but nothing inside it is *it*, so landing
    /// focus on the container would give the user a stop where no key does anything. Scrolling follows
    /// the pointer, or the focused control inside it once that exists.
    #[must_use]
    pub const fn focusable(self) -> bool {
        matches!(
            self,
            Self::Button
                | Self::Checkbox
                | Self::Slider
                | Self::TextEntry
                | Self::List
                | Self::Tabs
        )
    }

    /// Whether this widget remembers something between frames.
    ///
    /// A `Button` does not: being armed lasts from press to release and is not state anybody wants
    /// preserved across a relayout.
    #[must_use]
    pub const fn retains_state(self) -> bool {
        matches!(
            self,
            Self::Checkbox
                | Self::Slider
                | Self::TextEntry
                | Self::List
                | Self::Tabs
                | Self::Scroll
        )
    }

    /// Whether an authored node of this kind must carry an id.
    ///
    /// Retained state is keyed by id — that is what makes a scroll offset survive a resize — and focus
    /// is named the same way. A widget that needs either and has neither cannot work, so it is refused
    /// at load rather than silently forgetting its value at run time.
    #[must_use]
    pub const fn needs_id(self) -> bool {
        self.focusable() || self.retains_state()
    }
}

/// Longest text a `text_entry` accepts when the layout does not say.
///
/// Bounded because typed input is input, and the project's rule is that nothing grows without a limit
/// somebody chose. A default rather than a required field so the common case stays short to author.
pub const DEFAULT_MAX_LENGTH: usize = 256;

/// The span a `slider` moves over.
///
/// Structural, so it belongs in the layout: it describes the control rather than its value. The *value*
/// does not appear here — that comes from whatever the screen is editing, and a layout file stating one
/// would be a second source of truth for a setting the host already owns.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// How far one keyboard press or notch moves it.
    #[serde(default = "one")]
    pub step: f32,
}

const fn one() -> f32 {
    1.0
}

impl Range {
    /// Clamps a value into this range.
    #[must_use]
    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }

    /// How far along the range a value sits, from zero to one.
    ///
    /// A zero-width range reports zero rather than dividing by it. Such a range is refused by
    /// validation, so this is a guard against arithmetic rather than a supported case.
    #[must_use]
    pub fn fraction(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            return 0.0;
        }
        ((value - self.min) / span).clamp(0.0, 1.0)
    }

    /// The value a fraction of the way along.
    #[must_use]
    pub fn at(&self, fraction: f32) -> f32 {
        self.clamp(self.min + (self.max - self.min) * fraction.clamp(0.0, 1.0))
    }
}

/// One authored node.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Identifier, unique within a layout, naming this node for retained state and focus.
    ///
    /// Optional because most nodes are structure with nothing to remember.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What kind of control this is.
    #[serde(default)]
    pub widget: Widget,
    /// How children are arranged. Ignored by a node with none.
    #[serde(default)]
    pub direction: Direction,
    /// Extent along X.
    #[serde(default)]
    pub width: Sizing,
    /// Extent along Y.
    #[serde(default)]
    pub height: Sizing,
    /// Space between this node's edges and its children, in logical units.
    #[serde(default)]
    pub padding: Insets,
    /// Space between adjacent children, in logical units.
    #[serde(default)]
    pub gap: f32,
    /// Where children sit on the cross axis.
    #[serde(default)]
    pub align: Align,
    /// How leftover main-axis space is distributed.
    #[serde(default)]
    pub justify: Justify,
    /// Key into the string table for this node's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_key: Option<String>,
    /// What activating this node asks the engine to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    /// The span a `slider` moves over. Only meaningful on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    /// Longest text a `text_entry` accepts. Only meaningful on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// The id of the container holding this tab strip's pages. Only meaningful on a `tabs`.
    ///
    /// A `tabs` node's own children are the *headers* — one per tab, in order. The pages are elsewhere in
    /// the tree, because a header sits in a strip and a page fills the body, and no single container
    /// arranges both. So the strip names what it switches, and validation checks that the two agree about
    /// how many tabs there are: three headers over two pages is a layout whose third tab shows nothing,
    /// which is exactly the class of mistake this format refuses at load rather than in front of a player.
    ///
    /// Optional, because a tab strip with no pages is a useful control in its own right — a segmented
    /// picker whose selection some other screen reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    /// Children, in drawing and navigation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

impl Node {
    /// The longest text this node accepts, defaulted when the layout did not say.
    ///
    /// Named for the question rather than after the field, so reading `node.max_length` and
    /// `node.text_limit()` side by side makes the difference between "what was authored" and "what
    /// applies" obvious.
    #[must_use]
    pub fn text_limit(&self) -> usize {
        self.max_length.unwrap_or(DEFAULT_MAX_LENGTH)
    }
}

/// A whole authored layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// Schema version, so a future change is detected rather than mis-parsed.
    pub format_version: u32,
    /// The outermost node, solved against the whole viewport.
    pub root: Node,
}

impl Layout {
    /// Decodes and validates a layout from JSON bytes.
    ///
    /// Takes bytes rather than a path, because nothing above the resource layer opens a file.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError::Decode`] when the bytes are not a layout of this shape, or whichever
    /// validation error [`Self::validate`] finds.
    pub fn from_json(bytes: &[u8]) -> Result<Self, LayoutError> {
        let layout: Self = serde_json::from_slice(bytes)
            .map_err(|error| LayoutError::Decode(error.to_string()))?;
        layout.validate()?;
        Ok(layout)
    }

    /// Encodes to pretty-printed JSON with a trailing newline.
    ///
    /// Pretty-printed for the same reason the scenario format is: the file exists to be reviewed.
    ///
    /// # Errors
    ///
    /// Returns the `serde_json` error when encoding fails.
    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Checks everything the shape cannot.
    ///
    /// # Errors
    ///
    /// Returns [`LayoutError`] for a version mismatch, a duplicate or blank id, a nesting depth or
    /// node count past the bound, a non-finite or negative measurement, a zero fill weight, an
    /// action on a widget that cannot be activated, or a tab strip whose pages container is missing,
    /// wrongly arranged, or a different length from the strip.
    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.format_version != FORMAT_VERSION {
            return Err(LayoutError::Version {
                found: self.format_version,
                expected: FORMAT_VERSION,
            });
        }
        let mut seen = BTreeMap::new();
        let mut counted = 0usize;
        let mut links = Vec::new();
        validate_node(&self.root, 1, &mut seen, &mut counted, &mut links)?;
        // Every tab link is checked *after* the walk, because a strip may name a container that appears
        // later in the tree — which is the ordinary case, since a strip is authored above its own pages.
        for link in links {
            let Some(target) = seen.get(link.pages) else {
                return Err(LayoutError::UnknownPages {
                    id: link.pages.to_owned(),
                });
            };
            // Pages occupy the same box as each other: only one is shown, and the others must not take
            // space or leave a gap where they would have been. `Stack` is the arrangement that says so,
            // and requiring it here means a layout cannot look right in the file and wrong on screen.
            if target.direction != Direction::Stack {
                return Err(LayoutError::PagesNotStacked {
                    id: link.pages.to_owned(),
                });
            }
            if target.children.len() != link.headers {
                return Err(LayoutError::PageCountMismatch {
                    id: link.pages.to_owned(),
                    headers: link.headers,
                    pages: target.children.len(),
                });
            }
        }
        Ok(())
    }

    /// Every `text_key` the tree names, in tree order and with duplicates kept.
    ///
    /// Feeds [`StringTable::missing`](crate::StringTable::missing), which sorts and deduplicates, so
    /// this does neither.
    #[must_use]
    pub fn text_keys(&self) -> Vec<&str> {
        let mut keys = Vec::new();
        collect_keys(&self.root, &mut keys);
        keys
    }
}

fn collect_keys<'a>(node: &'a Node, keys: &mut Vec<&'a str>) {
    if let Some(key) = node.text_key.as_deref() {
        keys.push(key);
    }
    for child in &node.children {
        collect_keys(child, keys);
    }
}

/// One tab strip's claim about its pages, checked once the whole tree has been walked.
struct TabLink<'a> {
    pages: &'a str,
    headers: usize,
}

fn validate_node<'a>(
    node: &'a Node,
    depth: usize,
    seen: &mut BTreeMap<&'a str, &'a Node>,
    counted: &mut usize,
    links: &mut Vec<TabLink<'a>>,
) -> Result<(), LayoutError> {
    if depth > MAX_DEPTH {
        return Err(LayoutError::TooDeep { limit: MAX_DEPTH });
    }
    *counted += 1;
    if *counted > MAX_NODES {
        return Err(LayoutError::TooManyNodes { limit: MAX_NODES });
    }

    if let Some(id) = node.id.as_deref() {
        if id.trim().is_empty() {
            return Err(LayoutError::BlankId);
        }
        if seen.insert(id, node).is_some() {
            return Err(LayoutError::DuplicateId { id: id.to_owned() });
        }
    } else if node.widget.needs_id() {
        return Err(LayoutError::MissingId {
            widget: node.widget,
        });
    }

    check_sizing(node.width, "width")?;
    check_sizing(node.height, "height")?;
    check_measurement(node.gap, "gap")?;
    for (value, field) in [
        (node.padding.left, "padding.left"),
        (node.padding.top, "padding.top"),
        (node.padding.right, "padding.right"),
        (node.padding.bottom, "padding.bottom"),
    ] {
        check_measurement(value, field)?;
    }

    if node.action.is_some() && !node.widget.activatable() {
        return Err(LayoutError::ActionOnInertWidget {
            widget: node.widget,
        });
    }

    // A `range` or a `max_length` on the wrong widget is inert, and inert authoring is the class of
    // mistake this format refuses everywhere else. A slider *without* a range is refused too, because
    // there is no defensible default span for a value the layout knows nothing about.
    match (node.widget, node.range) {
        (Widget::Slider, None) => return Err(LayoutError::SliderWithoutRange),
        (Widget::Slider, Some(range)) => check_range(range)?,
        (widget, Some(_)) => {
            return Err(LayoutError::FieldOnWrongWidget {
                field: "range",
                widget,
            });
        }
        (_, None) => {}
    }
    if node.max_length.is_some() && node.widget != Widget::TextEntry {
        return Err(LayoutError::FieldOnWrongWidget {
            field: "max_length",
            widget: node.widget,
        });
    }
    if node.max_length == Some(0) {
        return Err(LayoutError::ZeroMaxLength);
    }

    match (node.widget, node.pages.as_deref()) {
        (Widget::Tabs, Some(pages)) => {
            // A strip cannot hold its own pages, and the check is a subtree search rather than an equality
            // test because the paradox is the same one level down: a page inside the strip would be one of
            // the strip's own headers, so selecting it would change what "it" is. The search is over the
            // strip's subtree only, and there are few tab strips in a layout.
            if names_within(node, pages) {
                return Err(LayoutError::PagesInsideTabs {
                    id: pages.to_owned(),
                });
            }
            links.push(TabLink {
                pages,
                headers: node.children.len(),
            });
        }
        (widget, Some(_)) => {
            return Err(LayoutError::FieldOnWrongWidget {
                field: "pages",
                widget,
            });
        }
        (_, None) => {}
    }

    for child in &node.children {
        validate_node(child, depth + 1, seen, counted, links)?;
    }
    Ok(())
}

/// Whether `id` names this node or anything beneath it.
fn names_within(node: &Node, id: &str) -> bool {
    node.id.as_deref() == Some(id) || node.children.iter().any(|child| names_within(child, id))
}

fn check_range(range: Range) -> Result<(), LayoutError> {
    for (value, field) in [
        (range.min, "range.min"),
        (range.max, "range.max"),
        (range.step, "range.step"),
    ] {
        if !value.is_finite() {
            return Err(LayoutError::Measurement { field, value });
        }
    }
    // `max > min` rather than `>=`: a collapsed range is a slider that cannot move, and a step of zero
    // is one whose keys do nothing. Both are almost certainly a mistake, and allowing either would put a
    // division by zero one arithmetic step away.
    if range.max <= range.min || range.step <= 0.0 {
        return Err(LayoutError::EmptyRange { range });
    }
    Ok(())
}

fn check_sizing(sizing: Sizing, field: &'static str) -> Result<(), LayoutError> {
    match sizing {
        Sizing::Fixed(amount) => check_measurement(amount, field),
        // A weight of zero asks for a share of nothing, which is `Fixed(0)` expressed confusingly.
        // Refusing it means every `Fill` in a solved tree has a positive divisor.
        Sizing::Fill(0) => Err(LayoutError::ZeroFillWeight { field }),
        Sizing::Auto | Sizing::Fill(_) => Ok(()),
    }
}

fn check_measurement(value: f32, field: &'static str) -> Result<(), LayoutError> {
    if !value.is_finite() || value < 0.0 {
        return Err(LayoutError::Measurement { field, value });
    }
    Ok(())
}

/// Why a layout could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    /// The bytes were not a layout of this shape.
    Decode(String),
    /// The schema version was not the one this build reads.
    Version {
        /// What the file declared.
        found: u32,
        /// What this build expects.
        expected: u32,
    },
    /// A node carried an id that was empty or only whitespace.
    BlankId,
    /// Two nodes shared an id.
    DuplicateId {
        /// The repeated identifier.
        id: String,
    },
    /// Nesting exceeded the bound.
    TooDeep {
        /// The deepest accepted nesting.
        limit: usize,
    },
    /// The tree held more nodes than the bound.
    TooManyNodes {
        /// The largest accepted count.
        limit: usize,
    },
    /// A measurement was negative or not finite.
    Measurement {
        /// Which field.
        field: &'static str,
        /// The rejected value.
        value: f32,
    },
    /// A fill weight was zero.
    ZeroFillWeight {
        /// Which field.
        field: &'static str,
    },
    /// An action was attached to a widget that cannot be activated.
    ActionOnInertWidget {
        /// The widget that carried it.
        widget: Widget,
    },
    /// A widget that needs an id to hold focus or state did not have one.
    MissingId {
        /// The widget that needed one.
        widget: Widget,
    },
    /// A slider declared no range.
    SliderWithoutRange,
    /// A range could not be moved over.
    EmptyRange {
        /// The rejected range.
        range: Range,
    },
    /// A field appeared on a widget it means nothing to.
    FieldOnWrongWidget {
        /// Which field.
        field: &'static str,
        /// The widget that carried it.
        widget: Widget,
    },
    /// A text entry accepted no characters at all.
    ZeroMaxLength,
    /// A tab strip named a pages container no node carries.
    UnknownPages {
        /// The identifier it named.
        id: String,
    },
    /// A tab strip's pages container did not stack its children.
    PagesNotStacked {
        /// The container's identifier.
        id: String,
    },
    /// A tab strip and its pages container disagreed about how many tabs there are.
    PageCountMismatch {
        /// The container's identifier.
        id: String,
        /// Headers in the strip.
        headers: usize,
        /// Pages in the container.
        pages: usize,
    },
    /// A tab strip named a pages container inside itself.
    PagesInsideTabs {
        /// The container's identifier.
        id: String,
    },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(detail) => write!(formatter, "not a readable layout: {detail}"),
            Self::Version { found, expected } => write!(
                formatter,
                "layout format version {found} is not the {expected} this build reads"
            ),
            Self::BlankId => write!(formatter, "a node id must not be blank"),
            Self::DuplicateId { id } => {
                write!(formatter, "two nodes share the id {id:?}")
            }
            Self::TooDeep { limit } => {
                write!(
                    formatter,
                    "layout nesting exceeds the depth limit of {limit}"
                )
            }
            Self::TooManyNodes { limit } => {
                write!(formatter, "a layout may hold at most {limit} nodes")
            }
            Self::Measurement { field, value } => write!(
                formatter,
                "{field} must be finite and not negative, found {value}"
            ),
            Self::ZeroFillWeight { field } => {
                write!(formatter, "{field} has a fill weight of zero")
            }
            Self::ActionOnInertWidget { widget } => write!(
                formatter,
                "a {widget:?} cannot be activated, so it must not carry an action"
            ),
            Self::MissingId { widget } => write!(
                formatter,
                "a {widget:?} takes focus or holds state, both of which are keyed by id, so it needs one"
            ),
            Self::SliderWithoutRange => {
                write!(formatter, "a Slider must declare the range it moves over")
            }
            Self::EmptyRange { range } => write!(
                formatter,
                "a range of {}..{} stepping {} cannot be moved over",
                range.min, range.max, range.step
            ),
            Self::FieldOnWrongWidget { field, widget } => write!(
                formatter,
                "{field} means nothing on a {widget:?} and must be left out"
            ),
            Self::ZeroMaxLength => {
                write!(
                    formatter,
                    "a TextEntry accepting no characters is not usable"
                )
            }
            Self::UnknownPages { id } => write!(
                formatter,
                "a Tabs names {id:?} as its pages, and no node carries that id"
            ),
            Self::PagesNotStacked { id } => write!(
                formatter,
                "the pages container {id:?} must stack its children, since only one page is shown at a time"
            ),
            Self::PageCountMismatch { id, headers, pages } => write!(
                formatter,
                "a Tabs has {headers} headers and its pages container {id:?} has {pages}, so at least one \
                 tab would show nothing"
            ),
            Self::PagesInsideTabs { id } => write!(
                formatter,
                "a Tabs names {id:?} as its pages, and that node is inside the strip itself"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
mod tests {
    // The range arithmetic below is over halves and quarters, which are exact in binary, so an exact
    // comparison is the assertion rather than a tolerance nobody chose.
    #![allow(clippy::float_cmp)]

    use super::{
        Align, Direction, Justify, Layout, LayoutError, MAX_DEPTH, MAX_NODES, Node, Range, Sizing,
        Widget,
    };
    use crate::Action;
    use crate::geometry::Insets;

    fn wrap(root: Node) -> Layout {
        Layout {
            format_version: super::FORMAT_VERSION,
            root,
        }
    }

    fn button(id: &str) -> Node {
        Node {
            id: Some(id.to_owned()),
            widget: Widget::Button,
            action: Some(Action::Quit),
            ..Node::default()
        }
    }

    /// Nests `depth` nodes inside one another.
    fn nested(depth: usize) -> Node {
        let mut node = Node::default();
        for _ in 1..depth {
            node = Node {
                children: vec![node],
                ..Node::default()
            };
        }
        node
    }

    #[test]
    fn a_menu_round_trips_through_json() {
        let layout = wrap(Node {
            id: Some("main".to_owned()),
            direction: Direction::Column,
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            padding: Insets::uniform(24.0),
            gap: 12.0,
            align: Align::Center,
            justify: Justify::Center,
            children: vec![
                Node {
                    widget: Widget::Label,
                    text_key: Some("menu.title".to_owned()),
                    ..Node::default()
                },
                button("play"),
            ],
            ..Node::default()
        });
        let encoded = layout.to_json().expect("encode");
        assert!(
            encoded.ends_with(b"\n"),
            "a reviewed file ends with a newline"
        );
        let decoded = Layout::from_json(&encoded).expect("decode");
        assert_eq!(decoded, layout);
    }

    #[test]
    fn defaults_keep_an_authored_file_short() {
        // Everything but the version and the widget kind is defaulted, because a layout file that has
        // to state every field is one nobody will keep tidy.
        let layout = Layout::from_json(br#"{"format_version":1,"root":{"widget":"label"}}"#)
            .expect("decode");
        assert_eq!(layout.root.widget, Widget::Label);
        assert_eq!(layout.root.width, Sizing::Auto);
        assert_eq!(layout.root.direction, Direction::Column);
        assert_eq!(layout.root.align, Align::Stretch);
        assert_eq!(layout.root.justify, Justify::Start);
        assert_eq!(layout.root.padding, Insets::ZERO);
        assert!(layout.root.children.is_empty());
    }

    #[test]
    fn a_mistyped_key_is_refused_rather_than_defaulted() {
        // The scenario format's reasoning: a hand-edited file's typo should be a load error, not a
        // silently-defaulted value that surfaces later as a layout that looks subtly wrong.
        let refused = Layout::from_json(br#"{"format_version":1,"root":{"widht":"label"}}"#);
        assert!(matches!(refused, Err(LayoutError::Decode(_))));
    }

    #[test]
    fn a_version_this_build_does_not_read_is_refused() {
        let refused = Layout::from_json(br#"{"format_version":2,"root":{}}"#);
        assert_eq!(
            refused,
            Err(LayoutError::Version {
                found: 2,
                expected: 1
            })
        );
    }

    #[test]
    fn an_action_a_layout_names_must_be_one_the_engine_defines() {
        // The layering rule: data must not be able to name an effect that does not exist. It fails at
        // load, naming the file, rather than at the moment somebody clicks.
        let refused = Layout::from_json(
            br#"{"format_version":1,"root":{"widget":"button","action":"drop_tables"}}"#,
        );
        assert!(matches!(refused, Err(LayoutError::Decode(_))));
    }

    #[test]
    fn an_action_on_a_widget_that_cannot_be_activated_is_refused() {
        // Otherwise the control looks correct and silently never fires, which is the hardest class of
        // authoring mistake to notice.
        let refused = wrap(Node {
            widget: Widget::Panel,
            action: Some(Action::Quit),
            ..Node::default()
        })
        .validate();
        assert_eq!(
            refused,
            Err(LayoutError::ActionOnInertWidget {
                widget: Widget::Panel
            })
        );
        // The activatable set is accepted. Each carries an id because all four take focus, which is
        // keyed by id.
        for widget in [Widget::Button, Widget::Checkbox, Widget::List, Widget::Tabs] {
            assert!(
                wrap(Node {
                    id: Some(format!("{widget:?}")),
                    widget,
                    action: Some(Action::Back),
                    ..Node::default()
                })
                .validate()
                .is_ok(),
                "{widget:?} must accept an action"
            );
        }
    }

    #[test]
    fn a_widget_that_needs_an_id_to_work_must_carry_one() {
        // Focus and retained state are both keyed by id. A checkbox without one cannot remember whether
        // it is checked, so this is refused at load rather than forgotten at run time.
        for widget in [
            Widget::Button,
            Widget::Checkbox,
            Widget::Slider,
            Widget::TextEntry,
            Widget::List,
            Widget::Tabs,
            Widget::Scroll,
        ] {
            let refused = wrap(Node {
                widget,
                range: (widget == Widget::Slider).then_some(Range {
                    min: 0.0,
                    max: 1.0,
                    step: 0.1,
                }),
                ..Node::default()
            })
            .validate();
            assert_eq!(
                refused,
                Err(LayoutError::MissingId { widget }),
                "{widget:?} must require an id"
            );
        }
        // Structure does not need one, because it remembers nothing and takes no focus.
        for widget in [Widget::Panel, Widget::Label] {
            assert!(
                wrap(Node {
                    widget,
                    ..Node::default()
                })
                .validate()
                .is_ok(),
                "{widget:?} must not require an id"
            );
        }
    }

    #[test]
    fn a_slider_must_declare_a_range_it_can_actually_move_over() {
        let slider = |range: Option<Range>| {
            wrap(Node {
                id: Some("volume".to_owned()),
                widget: Widget::Slider,
                range,
                ..Node::default()
            })
            .validate()
        };
        assert_eq!(slider(None), Err(LayoutError::SliderWithoutRange));
        // Collapsed, inverted, and a step of zero are all sliders that cannot move.
        for (min, max, step) in [(1.0, 1.0, 0.1), (2.0, 1.0, 0.1), (0.0, 1.0, 0.0)] {
            let range = Range { min, max, step };
            assert_eq!(
                slider(Some(range)),
                Err(LayoutError::EmptyRange { range }),
                "{min}..{max} stepping {step} must be refused"
            );
        }
        assert!(
            slider(Some(Range {
                min: 0.5,
                max: 2.0,
                step: 0.25
            }))
            .is_ok()
        );
    }

    #[test]
    fn a_field_on_a_widget_it_means_nothing_to_is_refused() {
        // The same posture as an action on a panel: inert authoring is a mistake that looks correct.
        let refused = wrap(Node {
            id: Some("label".to_owned()),
            widget: Widget::Label,
            range: Some(Range {
                min: 0.0,
                max: 1.0,
                step: 0.1,
            }),
            ..Node::default()
        })
        .validate();
        assert_eq!(
            refused,
            Err(LayoutError::FieldOnWrongWidget {
                field: "range",
                widget: Widget::Label
            })
        );
        let wrong_length = wrap(Node {
            id: Some("check".to_owned()),
            widget: Widget::Checkbox,
            max_length: Some(8),
            ..Node::default()
        })
        .validate();
        assert_eq!(
            wrong_length,
            Err(LayoutError::FieldOnWrongWidget {
                field: "max_length",
                widget: Widget::Checkbox
            })
        );
    }

    /// A tab strip over a pages container, with whatever is being varied applied by the caller.
    fn tabbed(headers: usize, pages: Vec<Node>, direction: Direction) -> Layout {
        wrap(Node {
            children: vec![
                Node {
                    id: Some("tabs".to_owned()),
                    widget: Widget::Tabs,
                    pages: Some("pages".to_owned()),
                    children: (0..headers).map(|_| Node::default()).collect(),
                    ..Node::default()
                },
                Node {
                    id: Some("pages".to_owned()),
                    direction,
                    children: pages,
                    ..Node::default()
                },
            ],
            ..Node::default()
        })
    }

    #[test]
    fn a_tab_strip_and_its_pages_container_must_agree_about_how_many_tabs_there_are() {
        // The check that earns the linkage its place in the format. Three headers over two pages is a screen
        // whose third tab shows nothing — and it is a mistake that looks entirely correct in the file,
        // because neither node is wrong on its own.
        let refused =
            tabbed(3, vec![Node::default(), Node::default()], Direction::Stack).validate();
        assert_eq!(
            refused,
            Err(LayoutError::PageCountMismatch {
                id: "pages".to_owned(),
                headers: 3,
                pages: 2,
            })
        );
        // And the matching case loads, so this is a bound rather than a refusal of tabs generally.
        assert!(
            tabbed(2, vec![Node::default(), Node::default()], Direction::Stack)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_pages_container_must_stack_its_pages() {
        // Pages occupy the same box: only one shows, and the others must not take space or leave a gap where
        // they would have been. A column of pages lays out without error and is visibly wrong, which is
        // exactly the class of mistake this format refuses at load.
        let refused =
            tabbed(2, vec![Node::default(), Node::default()], Direction::Column).validate();
        assert_eq!(
            refused,
            Err(LayoutError::PagesNotStacked {
                id: "pages".to_owned()
            })
        );
    }

    #[test]
    fn a_tab_strip_cannot_name_pages_that_are_absent_itself_or_inside_itself() {
        let absent = wrap(Node {
            id: Some("tabs".to_owned()),
            widget: Widget::Tabs,
            pages: Some("nowhere".to_owned()),
            children: vec![Node::default()],
            ..Node::default()
        })
        .validate();
        assert_eq!(
            absent,
            Err(LayoutError::UnknownPages {
                id: "nowhere".to_owned()
            })
        );

        // Its own pages would make each page one of its own headers, so selecting a page would change what
        // "it" is. Caught by a subtree search rather than an equality test, because the paradox is the same
        // one level down.
        let inside = wrap(Node {
            id: Some("tabs".to_owned()),
            widget: Widget::Tabs,
            pages: Some("inner".to_owned()),
            children: vec![Node {
                id: Some("inner".to_owned()),
                direction: Direction::Stack,
                children: vec![Node::default()],
                ..Node::default()
            }],
            ..Node::default()
        })
        .validate();
        assert_eq!(
            inside,
            Err(LayoutError::PagesInsideTabs {
                id: "inner".to_owned()
            })
        );
    }

    #[test]
    fn pages_named_by_anything_but_a_tab_strip_is_refused() {
        let refused = wrap(Node {
            widget: Widget::Panel,
            pages: Some("pages".to_owned()),
            ..Node::default()
        })
        .validate();
        assert_eq!(
            refused,
            Err(LayoutError::FieldOnWrongWidget {
                field: "pages",
                widget: Widget::Panel,
            })
        );
    }

    #[test]
    fn a_tab_strip_without_pages_is_a_selector_and_still_loads() {
        // A strip that switches nothing is a segmented picker whose selection some other screen reads, so
        // `pages` is optional rather than required on a `tabs`.
        let accepted = wrap(Node {
            id: Some("difficulty".to_owned()),
            widget: Widget::Tabs,
            children: vec![Node::default(), Node::default(), Node::default()],
            ..Node::default()
        })
        .validate();
        assert_eq!(accepted, Ok(()));
    }

    #[test]
    fn typed_text_is_bounded_by_default_and_never_by_zero() {
        let entry = |max_length: Option<usize>| Node {
            id: Some("name".to_owned()),
            widget: Widget::TextEntry,
            max_length,
            ..Node::default()
        };
        // Bounded because typed input is input, and nothing here grows without a limit somebody chose.
        assert_eq!(entry(None).text_limit(), super::DEFAULT_MAX_LENGTH);
        assert_eq!(entry(Some(12)).text_limit(), 12);
        assert_eq!(
            wrap(entry(Some(0))).validate(),
            Err(LayoutError::ZeroMaxLength)
        );
    }

    #[test]
    fn a_range_maps_between_values_and_fractions_without_dividing_by_zero() {
        let range = Range {
            min: 0.5,
            max: 2.0,
            step: 0.25,
        };
        assert_eq!(range.fraction(0.5), 0.0);
        assert_eq!(range.fraction(2.0), 1.0);
        assert_eq!(range.fraction(1.25), 0.5);
        assert_eq!(range.at(0.0), 0.5);
        assert_eq!(range.at(1.0), 2.0);
        assert_eq!(range.at(0.5), 1.25);
        // Out of range in either direction clamps rather than extrapolating.
        assert_eq!(range.fraction(-10.0), 0.0);
        assert_eq!(range.fraction(10.0), 1.0);
        assert_eq!(range.at(-1.0), 0.5);
        assert_eq!(range.clamp(99.0), 2.0);
        // Validation refuses a collapsed range, so this is a guard rather than a supported case.
        let collapsed = Range {
            min: 1.0,
            max: 1.0,
            step: 1.0,
        };
        assert_eq!(collapsed.fraction(1.0), 0.0);
    }

    #[test]
    fn ids_must_be_unique_and_not_blank() {
        let duplicated = wrap(Node {
            children: vec![button("same"), button("same")],
            ..Node::default()
        })
        .validate();
        assert_eq!(
            duplicated,
            Err(LayoutError::DuplicateId {
                id: "same".to_owned()
            })
        );
        let blank = wrap(Node {
            id: Some("   ".to_owned()),
            ..Node::default()
        })
        .validate();
        assert_eq!(blank, Err(LayoutError::BlankId));
    }

    #[test]
    fn nesting_is_bounded_because_the_tree_is_walked_recursively() {
        // Unbounded nesting from a data file is a stack overflow, which aborts rather than erroring,
        // and the bounded-parsing invariant exists to forbid exactly that.
        assert!(wrap(nested(MAX_DEPTH)).validate().is_ok());
        assert_eq!(
            wrap(nested(MAX_DEPTH + 1)).validate(),
            Err(LayoutError::TooDeep { limit: MAX_DEPTH })
        );
    }

    #[test]
    fn the_node_count_is_bounded_independently_of_depth() {
        // A shallow tree can still be enormous, so depth alone is not the bound.
        let wide = wrap(Node {
            children: vec![Node::default(); MAX_NODES],
            ..Node::default()
        });
        assert_eq!(
            wide.validate(),
            Err(LayoutError::TooManyNodes { limit: MAX_NODES })
        );
    }

    #[test]
    fn a_measurement_must_be_finite_and_not_negative() {
        for sizing in [Sizing::Fixed(f32::NAN), Sizing::Fixed(-1.0)] {
            let refused = wrap(Node {
                width: sizing,
                ..Node::default()
            })
            .validate();
            assert!(
                matches!(refused, Err(LayoutError::Measurement { .. })),
                "{sizing:?} must be refused"
            );
        }
        let bad_gap = wrap(Node {
            gap: f32::INFINITY,
            ..Node::default()
        })
        .validate();
        assert!(matches!(bad_gap, Err(LayoutError::Measurement { .. })));
        let bad_padding = wrap(Node {
            padding: Insets {
                left: -4.0,
                ..Insets::ZERO
            },
            ..Node::default()
        })
        .validate();
        assert!(matches!(bad_padding, Err(LayoutError::Measurement { .. })));
    }

    #[test]
    fn a_fill_weight_of_zero_is_refused_so_every_divisor_is_positive() {
        let refused = wrap(Node {
            width: Sizing::Fill(0),
            ..Node::default()
        })
        .validate();
        assert!(matches!(refused, Err(LayoutError::ZeroFillWeight { .. })));
    }

    #[test]
    fn text_keys_come_back_in_tree_order() {
        let layout = wrap(Node {
            text_key: Some("root".to_owned()),
            children: vec![
                Node {
                    text_key: Some("first".to_owned()),
                    ..Node::default()
                },
                Node {
                    children: vec![Node {
                        text_key: Some("nested".to_owned()),
                        ..Node::default()
                    }],
                    ..Node::default()
                },
            ],
            ..Node::default()
        });
        assert_eq!(layout.text_keys(), vec!["root", "first", "nested"]);
    }
}

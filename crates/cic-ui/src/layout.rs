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

use std::collections::BTreeSet;

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
/// The set the charter names for an RTS shell. This slice positions them; their behaviour — focus,
/// hover, retained state — is the next one, which is why there is nothing here but the kind.
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
    /// Children, in drawing and navigation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
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
    /// node count past the bound, a non-finite or negative measurement, a zero fill weight, or an
    /// action on a widget that cannot be activated.
    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.format_version != FORMAT_VERSION {
            return Err(LayoutError::Version {
                found: self.format_version,
                expected: FORMAT_VERSION,
            });
        }
        let mut seen = BTreeSet::new();
        let mut counted = 0usize;
        validate_node(&self.root, 1, &mut seen, &mut counted)
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

fn validate_node<'a>(
    node: &'a Node,
    depth: usize,
    seen: &mut BTreeSet<&'a str>,
    counted: &mut usize,
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
        if !seen.insert(id) {
            return Err(LayoutError::DuplicateId { id: id.to_owned() });
        }
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

    for child in &node.children {
        validate_node(child, depth + 1, seen, counted)?;
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
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
mod tests {
    use super::{
        Align, Direction, Justify, Layout, LayoutError, MAX_DEPTH, MAX_NODES, Node, Sizing, Widget,
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
        // The activatable set is accepted.
        for widget in [Widget::Button, Widget::Checkbox, Widget::List, Widget::Tabs] {
            assert!(
                wrap(Node {
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

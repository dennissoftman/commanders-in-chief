//! Turning an authored tree into positioned rectangles.
//!
//! # Why content measurement is a trait
//!
//! An `Auto`-sized label is as wide as its text, and how wide that is depends on a font, a size, and a
//! shaping pass — none of which belong in a layout solver, and all of which would drag a font library
//! into a crate that otherwise needs nothing. [`Measure`] is the same device the camera crate uses for
//! ground height: the caller supplies the one fact the model cannot derive, and the model stays
//! testable with a stub.
//!
//! # Why the result is flat rather than a tree
//!
//! Both consumers want a sequence. Drawing wants parents before children, which is pre-order. Hit
//! testing wants the topmost first, which is pre-order reversed. A tree would be walked into a sequence
//! at both call sites, so the walk happens once, here.
//!
//! # Why there are two passes
//!
//! `Auto` propagates upward — a column is as tall as its children — while `Fill` propagates downward,
//! since a share of the leftover cannot be known until the siblings that are *not* filling have been
//! sized. One pass cannot do both. Intrinsic sizes are computed bottom-up first, in logical units, and
//! positions are assigned top-down second, in physical pixels.
//!
//! The two passes walk the same tree in the same pre-order, which is what lets the second one find the
//! first one's answer for a node by index. Each entry therefore records its subtree size, so stepping
//! from one child's slot to the next sibling's is arithmetic rather than another traversal — the
//! alternative, recounting descendants at every level, is quadratic in the node count.
//!
//! # Why overflow is not an error
//!
//! Children whose fixed sizes exceed their parent overflow it rather than failing the solve. A layout is
//! authored against a range of viewports and the smallest is often genuinely too small; refusing to lay
//! out at all would replace a cramped screen with no screen. The rectangles stay honest about where
//! things went, and clipping is the drawing layer's decision.

// Child counts are bounded by `layout::MAX_NODES` and fill weights are small integers, so every
// integer converted below is exactly representable in `f32`.
#![allow(clippy::cast_precision_loss)]

use crate::geometry::{Rect, Viewport};
use crate::layout::{Align, Direction, Justify, Layout, Node, Sizing, Widget};
use crate::{Action, StringTable};

/// Supplies the intrinsic size of content the solver cannot measure itself.
///
/// Sizes are in **logical units**, matching the layout file, so an implementation does not have to know
/// the display scale. One that rasterises text can apply the scale itself.
pub trait Measure {
    /// The logical size this node's own content wants, given the logical space available to it.
    ///
    /// `available` is advisory: it is what the parent could offer before this node was sized, which is
    /// what a wrapping implementation needs. Returning something larger is allowed, and overflows.
    fn measure(&self, node: &Node, available: [f32; 2]) -> [f32; 2];
}

impl<F> Measure for F
where
    F: Fn(&Node, [f32; 2]) -> [f32; 2],
{
    fn measure(&self, node: &Node, available: [f32; 2]) -> [f32; 2] {
        self(node, available)
    }
}

/// A [`Measure`] for layouts with nothing to measure.
///
/// Every leaf reports no intrinsic size, so `Auto` leaves collapse. Useful for a purely structural
/// layout, and for tests about arrangement rather than about content.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoContent;

impl Measure for NoContent {
    fn measure(&self, _node: &Node, _available: [f32; 2]) -> [f32; 2] {
        [0.0, 0.0]
    }
}

/// One node's intrinsic size, with the size of the subtree it heads.
#[derive(Debug, Clone, Copy)]
struct Intrinsic {
    /// Logical size this node wants.
    size: [f32; 2],
    /// This node plus every descendant, so a sibling's slot is one addition away.
    subtree: usize,
}

/// One node, positioned.
///
/// Carries copies of the authored fields a consumer needs rather than a reference into the layout, so a
/// solved frame can outlive the borrow and be handed to a renderer or an input router on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SolvedNode {
    /// Where it landed, in physical pixels, with edges on whole pixels.
    pub rect: Rect,
    /// What kind of control it is.
    pub widget: Widget,
    /// Its identifier, when it has one.
    pub id: Option<String>,
    /// Its text key, when it has one.
    pub text_key: Option<String>,
    /// What activating it asks for.
    pub action: Option<Action>,
    /// Index of the parent in the same sequence, absent for the root.
    pub parent: Option<usize>,
    /// Nesting level, the root being one.
    pub depth: usize,
}

/// A whole layout, positioned.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    nodes: Vec<SolvedNode>,
}

impl Solved {
    /// Every node in pre-order, which is back-to-front drawing order.
    #[must_use]
    pub fn nodes(&self) -> &[SolvedNode] {
        &self.nodes
    }

    /// How many nodes there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether nothing was solved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node at an index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&SolvedNode> {
        self.nodes.get(index)
    }

    /// Finds a node by its authored id.
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&SolvedNode> {
        self.nodes
            .iter()
            .find(|node| node.id.as_deref() == Some(id))
    }

    /// The topmost node containing a physical point, or nothing if the point misses everything.
    ///
    /// Searched in reverse, because pre-order draws parents first and so the *last* match is the one on
    /// top — the one a click has to reach. Searching forward would hand every click to the backdrop.
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Option<&SolvedNode> {
        self.nodes
            .iter()
            .rev()
            .find(|node| node.rect.contains(x, y))
    }

    /// The topmost activatable node containing a physical point.
    ///
    /// What a click actually wants. The panel beneath a button contains the point too, and reporting it
    /// would swallow the press.
    #[must_use]
    pub fn hit_activatable(&self, x: f32, y: f32) -> Option<&SolvedNode> {
        self.nodes
            .iter()
            .rev()
            .find(|node| node.widget.activatable() && node.rect.contains(x, y))
    }

    /// Resolves every node's text against a table, in pre-order, skipping nodes without a key.
    #[must_use]
    pub fn texts<'a>(&'a self, strings: &'a StringTable) -> Vec<(&'a SolvedNode, &'a str)> {
        self.nodes
            .iter()
            .filter_map(|node| {
                node.text_key
                    .as_deref()
                    .map(|key| (node, strings.text(key)))
            })
            .collect()
    }
}

/// Positions a layout against a viewport.
///
/// The root is solved against the whole viewport, so a root sized `Fill` covers the surface and a root
/// sized `Auto` collapses to its content.
#[must_use]
pub fn solve(layout: &Layout, viewport: Viewport, measure: &impl Measure) -> Solved {
    let scale = viewport.scale();
    let bounds = viewport.bounds();
    let available = [bounds.width / scale, bounds.height / scale];

    let mut intrinsics = Vec::new();
    measure_node(&layout.root, available, measure, &mut intrinsics);

    let root = root_rect(&layout.root, bounds, scale, intrinsics[0].size);
    let mut arrange = Arrange {
        scale,
        intrinsics: &intrinsics,
        nodes: Vec::with_capacity(intrinsics.len()),
    };
    arrange.node(&layout.root, root, None, 1, 0);
    Solved {
        nodes: arrange.nodes,
    }
}

/// The arrange pass's invariant state.
///
/// Bundled rather than passed as five more parameters, because the recursion threads the scale, the
/// measure pass's answers, and the growing output through every level unchanged.
struct Arrange<'a> {
    scale: f32,
    intrinsics: &'a [Intrinsic],
    nodes: Vec<SolvedNode>,
}

/// The box the root occupies.
fn root_rect(root: &Node, bounds: Rect, scale: f32, natural: [f32; 2]) -> Rect {
    let resolve = |sizing: Sizing, natural: f32, extent: f32| match sizing {
        Sizing::Fixed(amount) => amount * scale,
        Sizing::Fill(_) => extent,
        // Clamped, because a root wider than the window is not a layout anybody can use, and the
        // window is the one bound that is not negotiable.
        Sizing::Auto => (natural * scale).min(extent),
    };
    Rect::new(
        0.0,
        0.0,
        resolve(root.width, natural[0], bounds.width),
        resolve(root.height, natural[1], bounds.height),
    )
}

/// Computes intrinsic logical sizes, appending one entry per node in pre-order.
///
/// The entry is reserved before recursing, so an index assigned here means the same node when the
/// arrange pass walks the tree again.
fn measure_node(
    node: &Node,
    available: [f32; 2],
    measure: &impl Measure,
    intrinsics: &mut Vec<Intrinsic>,
) -> [f32; 2] {
    let slot = intrinsics.len();
    intrinsics.push(Intrinsic {
        size: [0.0, 0.0],
        subtree: 0,
    });

    let inner = [
        (available[0] - node.padding.horizontal()).max(0.0),
        (available[1] - node.padding.vertical()).max(0.0),
    ];

    let natural = if node.children.is_empty() {
        measure.measure(node, inner)
    } else {
        let mut sizes = Vec::with_capacity(node.children.len());
        for child in &node.children {
            sizes.push(measure_node(child, inner, measure, intrinsics));
        }
        combine(node, &sizes)
    };

    // A node's own sizing overrides what its content asked for, because `Fixed` is a statement about
    // the box rather than a suggestion. `Fill` contributes no intrinsic minimum: it takes leftover, and
    // counting it as content would make its parent large enough that there was none.
    let size = [
        match node.width {
            Sizing::Fixed(amount) => amount,
            Sizing::Fill(_) => 0.0,
            Sizing::Auto => natural[0] + node.padding.horizontal(),
        },
        match node.height {
            Sizing::Fixed(amount) => amount,
            Sizing::Fill(_) => 0.0,
            Sizing::Auto => natural[1] + node.padding.vertical(),
        },
    ];
    intrinsics[slot] = Intrinsic {
        size,
        subtree: intrinsics.len() - slot,
    };
    size
}

/// Folds children's intrinsic sizes into the space their parent's arrangement needs.
fn combine(node: &Node, sizes: &[[f32; 2]]) -> [f32; 2] {
    let gaps = node.gap * sizes.len().saturating_sub(1) as f32;
    let sum = |axis: usize| sizes.iter().map(|size| size[axis]).sum::<f32>();
    let largest = |axis: usize| sizes.iter().map(|size| size[axis]).fold(0.0f32, f32::max);
    match node.direction {
        Direction::Row => [sum(0) + gaps, largest(1)],
        Direction::Column => [largest(0), sum(1) + gaps],
        Direction::Stack => [largest(0), largest(1)],
    }
}

impl Arrange<'_> {
    /// Assigns rectangles, appending one node per entry in the pre-order the measure pass used.
    fn node(&mut self, node: &Node, rect: Rect, parent: Option<usize>, depth: usize, slot: usize) {
        let index = self.nodes.len();
        self.nodes.push(SolvedNode {
            rect: rect.snapped(),
            widget: node.widget,
            id: node.id.clone(),
            text_key: node.text_key.clone(),
            action: node.action,
            parent,
            depth,
        });

        if node.children.is_empty() {
            return;
        }

        let content = rect.inset(node.padding.scaled(self.scale));
        let slots = child_slots(node, self.intrinsics, slot);
        let rects = child_rects(node, content, self.scale, self.intrinsics, &slots);
        for ((child, child_slot), child_rect) in node.children.iter().zip(&slots).zip(rects) {
            self.node(child, child_rect, Some(index), depth + 1, *child_slot);
        }
    }
}

/// The intrinsic slot of each child, stepping over whole subtrees.
fn child_slots(node: &Node, intrinsics: &[Intrinsic], slot: usize) -> Vec<usize> {
    let mut slots = Vec::with_capacity(node.children.len());
    let mut next = slot + 1;
    for _ in &node.children {
        slots.push(next);
        next += intrinsics.get(next).map_or(1, |entry| entry.subtree);
    }
    slots
}

/// Distributes a container's content box among its children.
fn child_rects(
    node: &Node,
    content: Rect,
    scale: f32,
    intrinsics: &[Intrinsic],
    slots: &[usize],
) -> Vec<Rect> {
    let natural = |index: usize| {
        slots
            .get(index)
            .and_then(|slot| intrinsics.get(*slot))
            .map_or([0.0, 0.0], |entry| entry.size)
    };

    if node.direction == Direction::Stack {
        return node
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| stacked(child, natural(index), content, node.align, scale))
            .collect();
    }

    let count = node.children.len();
    let main = usize::from(node.direction == Direction::Column);
    let cross = 1 - main;
    let gap = node.gap * scale;

    // Main axis: everything not filling takes what it asked for, and the remainder is shared by weight
    // among those that are.
    let mut mains = vec![0.0f32; count];
    let mut total_weight = 0u32;
    let mut consumed = gap * count.saturating_sub(1) as f32;
    for (index, child) in node.children.iter().enumerate() {
        match sizing(child, main) {
            Sizing::Fixed(amount) => {
                mains[index] = amount * scale;
                consumed += mains[index];
            }
            Sizing::Auto => {
                mains[index] = natural(index)[main] * scale;
                consumed += mains[index];
            }
            Sizing::Fill(weight) => total_weight += weight,
        }
    }
    let leftover = (axis(content, main) - consumed).max(0.0);
    if total_weight > 0 {
        let per_weight = leftover / total_weight as f32;
        for (index, child) in node.children.iter().enumerate() {
            if let Sizing::Fill(weight) = sizing(child, main) {
                mains[index] = per_weight * weight as f32;
            }
        }
    }

    // Only slack nothing claimed can be redistributed: a filling child has already taken it.
    let slack = if total_weight > 0 { 0.0 } else { leftover };
    let (mut position, extra_gap) = pack(node.justify, axis_start(content, main), slack, count);

    let cross_extent = axis(content, cross);
    let mut rects = Vec::with_capacity(count);
    for (index, child) in node.children.iter().enumerate() {
        let cross_size = match sizing(child, cross) {
            Sizing::Fixed(amount) => amount * scale,
            Sizing::Fill(_) => cross_extent,
            Sizing::Auto => {
                if node.align == Align::Stretch {
                    cross_extent
                } else {
                    natural(index)[cross] * scale
                }
            }
        };
        let cross_start = align_within(
            node.align,
            axis_start(content, cross),
            cross_extent,
            cross_size,
        );
        rects.push(compose(
            main,
            position,
            mains[index],
            cross_start,
            cross_size,
        ));
        position += mains[index] + gap + extra_gap;
    }
    rects
}

/// A child of a `Stack`, which fills the box unless it asked for a size.
///
/// One `align` governs both axes here, unlike a row or a column where it governs only the cross one.
/// That is a deliberate simplification: the case a stack exists for is a modal centred over a backdrop,
/// and two independent alignment fields would be four more states to author and test for no case anyone
/// has yet.
fn stacked(child: &Node, natural: [f32; 2], content: Rect, align: Align, scale: f32) -> Rect {
    let size = [0usize, 1].map(|index| match sizing(child, index) {
        Sizing::Fixed(amount) => amount * scale,
        Sizing::Fill(_) => axis(content, index),
        Sizing::Auto => {
            if align == Align::Stretch {
                axis(content, index)
            } else {
                natural[index] * scale
            }
        }
    });
    Rect::new(
        align_within(align, content.x, content.width, size[0]),
        align_within(align, content.y, content.height, size[1]),
        size[0],
        size[1],
    )
}

fn sizing(node: &Node, axis_index: usize) -> Sizing {
    if axis_index == 0 {
        node.width
    } else {
        node.height
    }
}

fn axis(rect: Rect, axis_index: usize) -> f32 {
    if axis_index == 0 {
        rect.width
    } else {
        rect.height
    }
}

fn axis_start(rect: Rect, axis_index: usize) -> f32 {
    if axis_index == 0 { rect.x } else { rect.y }
}

fn compose(
    main: usize,
    main_start: f32,
    main_size: f32,
    cross_start: f32,
    cross_size: f32,
) -> Rect {
    if main == 0 {
        Rect::new(main_start, cross_start, main_size, cross_size)
    } else {
        Rect::new(cross_start, main_start, cross_size, main_size)
    }
}

fn align_within(align: Align, start: f32, extent: f32, size: f32) -> f32 {
    match align {
        Align::Start | Align::Stretch => start,
        Align::Center => start + (extent - size) / 2.0,
        Align::End => start + extent - size,
    }
}

/// Where the pack begins, and how much to add to each gap.
fn pack(justify: Justify, start: f32, slack: f32, count: usize) -> (f32, f32) {
    match justify {
        Justify::Center => (start + slack / 2.0, 0.0),
        Justify::End => (start + slack, 0.0),
        Justify::SpaceBetween if count > 1 => (start, slack / (count - 1) as f32),
        // `Start`, and a lone child — which has no "between" to spread into, so it packs at the start
        // rather than taking a position that depends on a division by zero.
        Justify::Start | Justify::SpaceBetween => (start, 0.0),
    }
}

#[cfg(test)]
mod tests {
    // Every float compared below is an integer-valued pixel coordinate produced by snapping, or an
    // exactly-representable product of one, so exact comparison is the assertion rather than a
    // tolerance nobody chose. A layout that lands half a pixel out is a layout that is wrong.
    #![allow(clippy::float_cmp)]

    use super::{Measure, NoContent, Solved, solve};
    use crate::geometry::{Insets, Rect, Viewport};
    use crate::layout::{Align, Direction, Justify, Layout, Node, Sizing, Widget};
    use crate::{Action, StringTable};

    fn wrap(root: Node) -> Layout {
        Layout {
            format_version: crate::layout::FORMAT_VERSION,
            root,
        }
    }

    /// A root that covers whatever viewport it is given.
    fn full(children: Vec<Node>) -> Node {
        Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children,
            ..Node::default()
        }
    }

    fn fixed(width: f32, height: f32) -> Node {
        Node {
            width: Sizing::Fixed(width),
            height: Sizing::Fixed(height),
            ..Node::default()
        }
    }

    fn at(viewport: (u32, u32, f32), root: Node) -> Solved {
        let layout = wrap(root);
        layout
            .validate()
            .expect("the fixture must be a valid layout");
        let viewport = Viewport::new(viewport.0, viewport.1, viewport.2).expect("valid viewport");
        solve(&layout, viewport, &NoContent)
    }

    /// Reports a fixed logical size for every leaf, so `Auto` has something to be.
    struct Content(f32, f32);

    impl Measure for Content {
        fn measure(&self, _node: &Node, _available: [f32; 2]) -> [f32; 2] {
            [self.0, self.1]
        }
    }

    #[test]
    fn a_filling_root_covers_the_viewport() {
        let solved = at((800, 600, 1.0), full(vec![]));
        assert_eq!(solved.len(), 1);
        assert_eq!(solved.nodes()[0].rect, Rect::new(0.0, 0.0, 800.0, 600.0));
        assert_eq!(solved.nodes()[0].depth, 1);
        assert_eq!(solved.nodes()[0].parent, None);
    }

    #[test]
    fn fills_share_the_main_axis_by_weight() {
        let solved = at(
            (800, 600, 1.0),
            Node {
                direction: Direction::Row,
                ..full(vec![
                    Node {
                        width: Sizing::Fill(1),
                        ..Node::default()
                    },
                    Node {
                        width: Sizing::Fill(3),
                        ..Node::default()
                    },
                ])
            },
        );
        assert_eq!(solved.nodes()[1].rect, Rect::new(0.0, 0.0, 200.0, 600.0));
        assert_eq!(solved.nodes()[2].rect, Rect::new(200.0, 0.0, 600.0, 600.0));
    }

    #[test]
    fn three_equal_fills_still_tile_a_width_that_does_not_divide() {
        // The case edge snapping exists for. A third of 1000 is 333.33, and rounding sizes rather than
        // edges would leave a one-pixel seam between the second column and the third.
        let solved = at(
            (1000, 100, 1.0),
            Node {
                direction: Direction::Row,
                ..full(vec![
                    Node {
                        width: Sizing::Fill(1),
                        ..Node::default()
                    },
                    Node {
                        width: Sizing::Fill(1),
                        ..Node::default()
                    },
                    Node {
                        width: Sizing::Fill(1),
                        ..Node::default()
                    },
                ])
            },
        );
        let columns: Vec<Rect> = solved.nodes()[1..].iter().map(|node| node.rect).collect();
        assert_eq!(columns[0].right(), columns[1].x, "no seam after the first");
        assert_eq!(columns[1].right(), columns[2].x, "no seam after the second");
        assert_eq!(columns[0].x, 0.0);
        assert_eq!(columns[2].right(), 1000.0, "the row still fills the width");
    }

    #[test]
    fn every_authored_measurement_scales_with_the_display() {
        // The charter's requirement stated as a test: the same layout at two scales differs by exactly
        // the factor, so nothing is authored in pixels by accident.
        for (scale, expected) in [(1.0, 100.0), (2.0, 200.0), (1.5, 150.0)] {
            let solved = at(
                (2000, 2000, scale),
                Node {
                    padding: Insets::uniform(10.0),
                    gap: 4.0,
                    ..full(vec![fixed(100.0, 20.0), fixed(100.0, 20.0)])
                },
            );
            let first = solved.nodes()[1].rect;
            let second = solved.nodes()[2].rect;
            assert_eq!(first.width, expected, "at scale {scale}");
            assert_eq!(first.x, 10.0 * scale, "padding scales too");
            assert_eq!(first.y, 10.0 * scale);
            assert_eq!(
                second.y - first.bottom(),
                4.0 * scale,
                "and so does the gap, at scale {scale}"
            );
        }
    }

    #[test]
    fn an_auto_column_is_as_tall_as_its_children_plus_gaps_and_padding() {
        let solved = at(
            (400, 400, 1.0),
            Node {
                width: Sizing::Auto,
                height: Sizing::Auto,
                padding: Insets::uniform(5.0),
                gap: 3.0,
                children: vec![fixed(50.0, 20.0), fixed(50.0, 30.0)],
                ..Node::default()
            },
        );
        // 5 + 20 + 3 + 30 + 5
        assert_eq!(solved.nodes()[0].rect.height, 63.0);
        // The widest child plus padding on both sides.
        assert_eq!(solved.nodes()[0].rect.width, 60.0);
    }

    #[test]
    fn an_auto_leaf_takes_the_size_the_measurer_reports() {
        let layout = wrap(Node {
            width: Sizing::Auto,
            height: Sizing::Auto,
            widget: Widget::Label,
            ..Node::default()
        });
        let viewport = Viewport::new(400, 400, 2.0).expect("valid viewport");
        let solved = solve(&layout, viewport, &Content(30.0, 12.0));
        // Measured in logical units, drawn in physical, so the scale applies once.
        assert_eq!(solved.nodes()[0].rect, Rect::new(0.0, 0.0, 60.0, 24.0));
    }

    #[test]
    fn stretch_fills_the_cross_axis_and_the_other_alignments_do_not() {
        let row = |align: Align| {
            at(
                (400, 200, 1.0),
                Node {
                    direction: Direction::Row,
                    align,
                    ..full(vec![Node {
                        width: Sizing::Fixed(40.0),
                        height: Sizing::Fixed(50.0),
                        ..Node::default()
                    }])
                },
            )
            .nodes()[1]
                .rect
        };
        // A `Fixed` cross size is never stretched: an explicit number outranks the default.
        assert_eq!(row(Align::Stretch).height, 50.0);
        assert_eq!(row(Align::Start).y, 0.0);
        assert_eq!(row(Align::Center).y, 75.0, "centred in 200 at height 50");
        assert_eq!(row(Align::End).y, 150.0);
    }

    #[test]
    fn an_auto_cross_size_stretches_only_when_alignment_asks_it_to() {
        let row = |align: Align| {
            let layout = wrap(Node {
                direction: Direction::Row,
                align,
                ..full(vec![Node {
                    width: Sizing::Fixed(40.0),
                    height: Sizing::Auto,
                    ..Node::default()
                }])
            });
            let viewport = Viewport::new(400, 200, 1.0).expect("valid viewport");
            solve(&layout, viewport, &Content(10.0, 15.0)).nodes()[1].rect
        };
        assert_eq!(row(Align::Stretch).height, 200.0, "stretch fills");
        assert_eq!(row(Align::Start).height, 15.0, "otherwise it measures");
    }

    #[test]
    fn justify_moves_a_pack_that_does_not_fill_its_parent() {
        let column = |justify: Justify| {
            at(
                (100, 400, 1.0),
                Node {
                    justify,
                    ..full(vec![fixed(100.0, 50.0), fixed(100.0, 50.0)])
                },
            )
        };
        let starts = |justify: Justify| {
            let solved = column(justify);
            (solved.nodes()[1].rect.y, solved.nodes()[2].rect.y)
        };
        assert_eq!(starts(Justify::Start), (0.0, 50.0));
        // 400 - 100 of content leaves 300 of slack; centring puts half above.
        assert_eq!(starts(Justify::Center), (150.0, 200.0));
        assert_eq!(starts(Justify::End), (300.0, 350.0));
        // Spread puts the whole slack between the two.
        assert_eq!(starts(Justify::SpaceBetween), (0.0, 350.0));
    }

    #[test]
    fn justify_cannot_move_a_pack_a_fill_already_consumed() {
        // A filling child takes the slack, so there is none left for `justify` to distribute. Applying
        // both would push the pack past its own parent.
        let solved = at(
            (100, 400, 1.0),
            Node {
                justify: Justify::Center,
                ..full(vec![
                    fixed(100.0, 50.0),
                    Node {
                        height: Sizing::Fill(1),
                        ..Node::default()
                    },
                ])
            },
        );
        assert_eq!(solved.nodes()[1].rect.y, 0.0);
        assert_eq!(solved.nodes()[2].rect, Rect::new(0.0, 50.0, 100.0, 350.0));
    }

    #[test]
    fn a_stack_puts_every_child_in_the_same_box() {
        let solved = at(
            (200, 100, 1.0),
            Node {
                direction: Direction::Stack,
                ..full(vec![Node::default(), Node::default()])
            },
        );
        let expected = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(solved.nodes()[1].rect, expected);
        assert_eq!(solved.nodes()[2].rect, expected, "and not stacked in a row");
    }

    #[test]
    fn a_stack_centres_a_sized_child_on_both_axes() {
        // The case a stack exists for: a modal over a backdrop.
        let solved = at(
            (200, 100, 1.0),
            Node {
                direction: Direction::Stack,
                align: Align::Center,
                ..full(vec![
                    Node {
                        width: Sizing::Fill(1),
                        height: Sizing::Fill(1),
                        ..Node::default()
                    },
                    fixed(80.0, 40.0),
                ])
            },
        );
        assert_eq!(solved.nodes()[1].rect, Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(solved.nodes()[2].rect, Rect::new(60.0, 30.0, 80.0, 40.0));
    }

    #[test]
    fn a_nested_tree_gives_every_node_its_own_measurement() {
        // The pass-alignment test. Both passes walk the tree in pre-order and the second finds the
        // first's answers by index, so a subtree miscounted anywhere shifts every later sibling onto a
        // descendant's intrinsic size. A lopsided tree is what makes that visible: the second branch's
        // sizes are only right if the first branch's three nodes were stepped over exactly.
        let solved = at(
            (400, 400, 1.0),
            Node {
                direction: Direction::Row,
                align: Align::Start,
                ..full(vec![
                    Node {
                        width: Sizing::Auto,
                        height: Sizing::Auto,
                        children: vec![fixed(30.0, 10.0), fixed(70.0, 20.0)],
                        ..Node::default()
                    },
                    fixed(11.0, 13.0),
                    Node {
                        width: Sizing::Auto,
                        height: Sizing::Auto,
                        children: vec![fixed(17.0, 19.0)],
                        ..Node::default()
                    },
                ])
            },
        );
        let rect = |index: usize| solved.nodes()[index].rect;
        assert_eq!(solved.len(), 7, "root, three branches, three leaves");
        // First branch: a column of two, so 70 wide and 30 tall.
        assert_eq!(rect(1), Rect::new(0.0, 0.0, 70.0, 30.0));
        assert_eq!(rect(2), Rect::new(0.0, 0.0, 30.0, 10.0));
        assert_eq!(rect(3), Rect::new(0.0, 10.0, 70.0, 20.0));
        // The middle leaf must start where the first branch ended, not where a descendant did.
        assert_eq!(rect(4), Rect::new(70.0, 0.0, 11.0, 13.0));
        // Third branch wraps its single child.
        assert_eq!(rect(5), Rect::new(81.0, 0.0, 17.0, 19.0));
        assert_eq!(rect(6), Rect::new(81.0, 0.0, 17.0, 19.0));
        // Parent links describe the authored tree, not the flattened order.
        assert_eq!(solved.nodes()[2].parent, Some(1));
        assert_eq!(solved.nodes()[4].parent, Some(0));
        assert_eq!(solved.nodes()[6].parent, Some(5));
        assert_eq!(solved.nodes()[6].depth, 3);
    }

    #[test]
    fn children_too_large_for_their_parent_overflow_rather_than_failing() {
        // A cramped screen beats no screen, and the rectangles stay honest about where things went so
        // the drawing layer can clip.
        let solved = at(
            (100, 40, 1.0),
            full(vec![fixed(100.0, 50.0), fixed(100.0, 50.0)]),
        );
        assert_eq!(solved.nodes()[1].rect.height, 50.0);
        assert_eq!(solved.nodes()[2].rect.y, 50.0, "past the parent's bottom");
        assert!(solved.nodes()[2].rect.bottom() > solved.nodes()[0].rect.bottom());
    }

    #[test]
    fn padding_larger_than_its_box_leaves_children_with_nothing() {
        let solved = at(
            (50, 50, 1.0),
            Node {
                padding: Insets::uniform(40.0),
                ..full(vec![Node {
                    width: Sizing::Fill(1),
                    height: Sizing::Fill(1),
                    ..Node::default()
                }])
            },
        );
        assert_eq!(solved.nodes()[1].rect.width, 0.0);
        assert_eq!(solved.nodes()[1].rect.height, 0.0);
    }

    #[test]
    fn a_hit_finds_the_topmost_node_and_an_activatable_hit_skips_the_panel_beneath() {
        let solved = at(
            (200, 100, 1.0),
            full(vec![Node {
                id: Some("ok".to_owned()),
                widget: Widget::Button,
                action: Some(Action::ApplySettings),
                width: Sizing::Fixed(80.0),
                height: Sizing::Fixed(30.0),
                ..Node::default()
            }]),
        );
        // The root panel contains the point too, and returning it would swallow the press.
        assert_eq!(
            solved.hit(10.0, 10.0).and_then(|n| n.id.as_deref()),
            Some("ok")
        );
        assert_eq!(
            solved
                .hit_activatable(10.0, 10.0)
                .and_then(|node| node.action),
            Some(Action::ApplySettings)
        );
        // Outside the button, the only thing under the cursor is the root, which is not activatable.
        assert!(solved.hit(150.0, 80.0).is_some());
        assert!(solved.hit_activatable(150.0, 80.0).is_none());
        // Outside everything.
        assert!(solved.hit(500.0, 500.0).is_none());
    }

    #[test]
    fn a_node_is_findable_by_its_authored_id() {
        let solved = at(
            (200, 100, 1.0),
            full(vec![Node {
                id: Some("play".to_owned()),
                ..Node::default()
            }]),
        );
        assert!(solved.by_id("play").is_some());
        assert!(solved.by_id("absent").is_none());
        assert_eq!(solved.get(99), None);
        assert!(!solved.is_empty());
    }

    #[test]
    fn text_resolves_in_drawing_order_against_the_table() {
        let mut strings = StringTable::new();
        strings.set("menu.title", "Commanders");
        let solved = at(
            (200, 100, 1.0),
            full(vec![
                Node {
                    widget: Widget::Label,
                    text_key: Some("menu.title".to_owned()),
                    ..Node::default()
                },
                Node {
                    widget: Widget::Label,
                    text_key: Some("menu.absent".to_owned()),
                    ..Node::default()
                },
            ]),
        );
        let texts: Vec<&str> = solved
            .texts(&strings)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        // The missing one shows its key rather than a blank label.
        assert_eq!(texts, vec!["Commanders", "menu.absent"]);
    }
}

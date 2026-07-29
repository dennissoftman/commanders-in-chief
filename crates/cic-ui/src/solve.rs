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
use crate::layout::{
    Align, DEFAULT_MAX_LENGTH, Direction, Justify, Layout, Node, Range, Sizing, Style, Widget,
};
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

    /// The logical width of a string drawn at a logical text size.
    ///
    /// Separate from [`Self::measure`] because the two answer different questions. A node's size is
    /// asked once, before anything is positioned; an *advance* is asked about a substring — how far
    /// along the caret sits after three characters — and only the same implementation that will
    /// rasterise the text can answer it. It is what narrows the input-method cursor area from the whole
    /// field to the caret, and what lets a drawing layer place one.
    ///
    /// Zero by default, so a layout with nothing to measure and a stub in a test need not implement it.
    /// A drawing layer that gets zero draws a caret at the start of the field, which is wrong but not
    /// broken — the honest failure for a measurement nobody supplied.
    fn advance(&self, _text: &str, _size: f32) -> f32 {
        0.0
    }
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

/// Supplies each tab strip's chosen tab, so the solver knows which page is showing.
///
/// # Why the solver needs this at all
///
/// Everything else here is a pure function of the layout and the viewport. Tab pages are not: which of a
/// stack of pages is the visible one is *state*, held by [`Interface`](crate::Interface) and keyed by the
/// strip's id — and a page that is not showing must take no clicks, hold no tab stop, and draw nothing. All
/// three read the solved sequence, so marking the hidden nodes here answers them at once, rather than
/// leaving each consumer to remember to ask.
///
/// It also means a tab change is a *relayout*, exactly as a resize is. That is the cost of the decision and
/// it is the right way round: solving is cheap and already happens per frame, whereas a consumer that
/// forgot to filter would hand a click to an invisible control.
pub trait Selections {
    /// The chosen entry for a control, or `None` when nothing has chosen yet.
    fn selection(&self, id: &str) -> Option<usize>;

    /// Whether this control's dropdown is open.
    ///
    /// Only a [`Widget::Combo`] answers true, and at most one at a time. A closed combo's options are not on
    /// screen at all, which is the same mechanism a tab page that is not chosen uses — so both arrive here
    /// rather than one being visibility and the other being a special case in three consumers.
    fn is_open(&self, id: &str) -> bool;
}

/// A [`Selections`] where nothing has been chosen, so every tab strip shows its first page.
///
/// The default for [`solve`], and correct rather than merely convenient: a screen that has never been
/// interacted with shows its first tab.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSelection;

impl Selections for NoSelection {
    fn selection(&self, _id: &str) -> Option<usize> {
        None
    }

    fn is_open(&self, _id: &str) -> bool {
        false
    }
}

/// How tall a combo's option row is when nothing measured it, in logical units.
///
/// A floor rather than a size: an option normally takes the height its text measured to. This exists because
/// a [`Measure`] that reports nothing — [`NoContent`], or a host whose font is not loaded yet — would
/// otherwise produce a list of zero-height rows, which is a dropdown that opens and appears not to.
const DEFAULT_OPTION_HEIGHT: f32 = 24.0;

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
    /// What it is for, when its widget kind does not already say.
    pub style: Option<Style>,
    /// Where its own text sits, which on a childless node is what `align` selects.
    pub align: Align,
    /// Its identifier, when it has one.
    pub id: Option<String>,
    /// Its text key, when it has one.
    pub text_key: Option<String>,
    /// What activating it asks for.
    pub action: Option<Action>,
    /// The span a slider moves over.
    pub range: Option<Range>,
    /// Longest text a text entry accepts, as authored.
    pub max_length: Option<usize>,
    /// The pages container this tab strip switches, when it has one.
    pub pages: Option<String>,
    /// Whether this node is on screen at all.
    ///
    /// False for a tab page that is not the selected one, and for everything inside it. A hidden node is
    /// still *solved* — its rectangle is correct for when its tab is chosen — but every consumer here skips
    /// it, so it takes no clicks, holds no tab stop, and is not drawn.
    pub visible: bool,
    /// How many direct children it has, which is what bounds a list's or a tab strip's selection.
    pub children: usize,
    /// This node plus every descendant.
    ///
    /// The sequence is pre-order, so a node's descendants are exactly the `subtree - 1` entries after
    /// it. That is what lets a scrollable container measure its own content without a second walk.
    pub subtree: usize,
    /// Index of the parent in the same sequence, absent for the root.
    pub parent: Option<usize>,
    /// Nesting level, the root being one.
    pub depth: usize,
}

impl SolvedNode {
    /// The longest text this node accepts, defaulted when the layout did not say.
    #[must_use]
    pub fn text_limit(&self) -> usize {
        self.max_length.unwrap_or(DEFAULT_MAX_LENGTH)
    }
}

/// A whole layout, positioned.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    nodes: Vec<SolvedNode>,
    /// The open combo and its options, as a half-open range of sequence indices.
    ///
    /// # Why an overlay needs to be named rather than inferred
    ///
    /// A dropdown breaks the one assumption the flat pre-order sequence rests on: that a node's position in
    /// the sequence is its position in the stacking order. A combo early in a screen opens a list over
    /// siblings that come *later*, so drawing in sequence order paints the list under them and hit testing in
    /// reverse sequence order hands a click on the list to whatever is behind it.
    ///
    /// Naming the range fixes both from one place: it is drawn last and searched first. The alternative —
    /// having each consumer notice the combo for itself — is three chances to get the ordering wrong, and two
    /// of them are only visible in motion.
    overlay: Option<std::ops::Range<usize>>,
}

impl Solved {
    /// Every node in pre-order, visible or not.
    ///
    /// A renderer wants [`Self::visible_nodes`]; this is for a caller that needs the sequence indices to
    /// line up with what [`Self::get`] returns.
    #[must_use]
    pub fn nodes(&self) -> &[SolvedNode] {
        &self.nodes
    }

    /// Every node that is on screen, in pre-order, which is back-to-front drawing order.
    pub fn visible_nodes(&self) -> impl Iterator<Item = &SolvedNode> {
        self.nodes.iter().filter(|node| node.visible)
    }

    /// The open dropdown's own subtree, drawn last and searched first.
    ///
    /// Empty when no combo is open, which is the ordinary case.
    #[must_use]
    pub fn overlay(&self) -> std::ops::Range<usize> {
        self.overlay.clone().unwrap_or(0..0)
    }

    /// Whether a point lands anywhere the open dropdown covers.
    ///
    /// What a caller dismissing a dropdown on an outside press asks. The control itself counts as covered:
    /// pressing it again is how a user closes the list they just opened.
    #[must_use]
    pub fn covers_overlay(&self, x: f32, y: f32) -> bool {
        self.overlay().any(|index| {
            self.nodes
                .get(index)
                .is_some_and(|node| node.rect.contains(x, y))
        })
    }

    /// Whether an index is part of the open dropdown.
    #[must_use]
    pub fn is_overlay(&self, index: usize) -> bool {
        self.overlay
            .as_ref()
            .is_some_and(|range| range.contains(&index))
    }

    /// Hides every closed combo's options, and names the open one's subtree as the overlay.
    ///
    /// Where the options *are* was decided in the arrange pass, which is the only place their measured
    /// heights exist. What is decided here is whether they are on screen, because that is state — the same
    /// split, and the same flag, that a tab page not currently chosen uses.
    fn resolve_open_combos(&mut self, selections: &impl Selections) {
        let combos: Vec<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.widget == Widget::Combo)
            .map(|(index, _)| index)
            .collect();
        for combo in combos {
            let Some(node) = self.nodes.get(combo) else {
                continue;
            };
            let span = node.subtree;
            let open = node.id.as_deref().is_some_and(|id| selections.is_open(id));
            for index in (combo + 1)..(combo + span).min(self.nodes.len()) {
                self.nodes[index].visible = open;
            }
            // At most one combo is open at a time — [`crate::Interface`] holds one id — so the last one to
            // answer true wins rather than the overlay being a set. A second open dropdown is not a state
            // this can reach.
            if open {
                self.overlay = Some(combo..(combo + span).min(self.nodes.len()));
            }
        }
    }

    /// Marks every tab page except the chosen one — and everything inside it — as not visible.
    ///
    /// The selection is clamped to the pages that exist and defaults to the first, so a stale selection left
    /// behind by a layout that has since lost a tab shows the last page rather than nothing at all. Blanking
    /// the screen would be the alternative, and a blank screen reads as a crash.
    fn hide_unselected_pages(&mut self, selections: &impl Selections) {
        // Collected first because the walk below mutates, and the strips are few.
        let strips: Vec<(String, usize)> = self
            .nodes
            .iter()
            .filter_map(|node| {
                let pages = node.pages.clone()?;
                let id = node.id.as_deref()?;
                Some((pages, selections.selection(id).unwrap_or(0)))
            })
            .collect();
        for (pages, chosen) in strips {
            let Some(container) = self.index_of(&pages) else {
                continue;
            };
            let children = self.children_of(container);
            let chosen = chosen.min(children.len().saturating_sub(1));
            for (index, child) in children.into_iter().enumerate() {
                if index == chosen {
                    continue;
                }
                // The page and its whole subtree, which is exactly the `subtree - 1` entries after it —
                // the sequence being pre-order is what makes hiding a branch a range rather than a walk.
                let span = self.nodes.get(child).map_or(1, |node| node.subtree);
                for hidden in child..(child + span).min(self.nodes.len()) {
                    self.nodes[hidden].visible = false;
                }
            }
        }
    }

    /// The rectangles of a node's direct children, in authored order.
    ///
    /// What a tab strip's headers are: the strip is one control, and which of its children a click landed in
    /// is what says which tab was chosen.
    #[must_use]
    pub fn child_rects(&self, index: usize) -> Vec<Rect> {
        self.children_of(index)
            .into_iter()
            .filter_map(|child| self.nodes.get(child).map(|node| node.rect))
            .collect()
    }

    /// The sequence indices of a node's direct children.
    ///
    /// Stepping over whole subtrees rather than scanning for a matching parent, which is what keeps this
    /// linear in the children rather than in the whole tree.
    fn children_of(&self, index: usize) -> Vec<usize> {
        let Some(node) = self.nodes.get(index) else {
            return Vec::new();
        };
        let end = (index + node.subtree).min(self.nodes.len());
        let mut children = Vec::with_capacity(node.children);
        let mut next = index + 1;
        while next < end {
            children.push(next);
            next += self.nodes.get(next).map_or(1, |child| child.subtree);
        }
        children
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

    /// The topmost visible node containing a physical point, or nothing if the point misses everything.
    ///
    /// Searched in reverse, because pre-order draws parents first and so the *last* match is the one on
    /// top — the one a click has to reach. Searching forward would hand every click to the backdrop.
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Option<&SolvedNode> {
        self.hit_where(x, y, |_| true)
            .and_then(|index| self.nodes.get(index))
    }

    /// The topmost visible activatable node containing a physical point.
    ///
    /// What a click actually wants. The panel beneath a button contains the point too, and reporting it
    /// would swallow the press.
    #[must_use]
    pub fn hit_activatable(&self, x: f32, y: f32) -> Option<&SolvedNode> {
        self.hit_where(x, y, |node| node.widget.activatable())
            .and_then(|index| self.nodes.get(index))
    }

    /// The index of the node carrying an id.
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.id.as_deref() == Some(id))
    }

    /// The index of the topmost focusable node containing a physical point.
    ///
    /// An index rather than a reference, because a caller that has hit something usually needs to walk
    /// its ancestors next — a scroll wheel acts on the enclosing container, not on what is under it.
    #[must_use]
    pub fn hit_focusable(&self, x: f32, y: f32) -> Option<usize> {
        self.hit_where(x, y, |node| node.widget.focusable())
    }

    /// The topmost visible node satisfying a test, the open dropdown searched first.
    ///
    /// Sequence order is stacking order for everything except an overlay, and an open dropdown is drawn over
    /// siblings that come *after* it — so a plain reverse scan would hand a click on the list to whatever is
    /// behind it. Searching the overlay first is the same rule as drawing it last, stated once.
    /// Anything an open dropdown *covers* belongs to the dropdown, even where the point lands on a row that
    /// is not itself a control: a click on an option is a click on the combo that owns it, so the search
    /// falls back to the owner rather than passing the point through to whatever is behind the list.
    fn hit_where(&self, x: f32, y: f32, wanted: impl Fn(&SolvedNode) -> bool) -> Option<usize> {
        let ok = |index: usize| {
            self.nodes
                .get(index)
                .is_some_and(|node| node.visible && wanted(node))
        };
        let inside = |index: usize| {
            self.nodes
                .get(index)
                .is_some_and(|node| node.visible && node.rect.contains(x, y))
        };
        if let Some(range) = self
            .overlay
            .clone()
            .filter(|range| range.clone().any(inside))
        {
            return range
                .clone()
                .rev()
                .find(|index| ok(*index) && inside(*index))
                .or_else(|| Some(range.start).filter(|index| ok(*index)));
        }
        (0..self.nodes.len())
            .rev()
            .find(|index| !self.is_overlay(*index) && ok(*index) && inside(*index))
    }

    /// The index of a node's nth direct child.
    ///
    /// Stepping over whole subtrees rather than counting depths, which is what makes this arithmetic: in
    /// pre-order a child's next sibling is `subtree` entries further on. What a list or a tab strip needs
    /// to find the row its selection names, since a selection is an index among *direct* children and the
    /// flat sequence holds every descendant.
    #[must_use]
    pub fn child(&self, index: usize, nth: usize) -> Option<usize> {
        let parent = self.nodes.get(index)?;
        let end = (index + parent.subtree).min(self.nodes.len());
        let mut at = index + 1;
        let mut counted = 0;
        while at < end {
            if counted == nth {
                return Some(at);
            }
            counted += 1;
            at += self.nodes.get(at).map_or(1, |node| node.subtree.max(1));
        }
        None
    }

    /// The nearest enclosing node of a given kind, starting with the node itself.
    #[must_use]
    pub fn enclosing(&self, index: usize, widget: Widget) -> Option<usize> {
        let mut current = Some(index);
        while let Some(at) = current {
            let node = self.nodes.get(at)?;
            if node.widget == widget {
                return Some(at);
            }
            current = node.parent;
        }
        None
    }

    /// Every visible focusable node's id, in pre-order, which is navigation order.
    ///
    /// Pre-order rather than by position on screen. Reading order is what an author controls and what a
    /// reviewer can see in the file; geometric order would make tab sequence depend on a solved layout,
    /// so a resize could silently reorder it.
    ///
    /// Controls on a tab page that is not showing are absent, which is the whole reason visibility reaches
    /// this far: a keyboard sequence that stopped on an invisible field would look like focus vanishing.
    #[must_use]
    pub fn focus_order(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|node| node.visible && node.widget.focusable())
            .filter_map(|node| node.id.as_deref())
            .collect()
    }

    /// How far a scrollable node can be scrolled before its content runs out.
    ///
    /// Measured from the solved rectangles rather than from an authored figure, because the content's
    /// extent is what the layout worked out and not something an author states. Zero when everything
    /// fits, which is also what a non-scrollable node reports.
    #[must_use]
    pub fn scroll_limit(&self, index: usize) -> f32 {
        let Some(node) = self.nodes.get(index) else {
            return 0.0;
        };
        let descendants = &self.nodes[index + 1..(index + node.subtree).min(self.nodes.len())];
        let content = descendants
            .iter()
            // A hidden tab page inside a scrollable container is not content the user can scroll to, so it
            // must not extend the range: it would leave the container able to scroll past its own end.
            .filter(|child| child.visible)
            .map(|child| child.rect.bottom())
            .fold(node.rect.y, f32::max);
        (content - node.rect.y - node.rect.height).max(0.0)
    }

    /// Resolves every visible node's text against a table, in pre-order, skipping nodes without a key.
    #[must_use]
    pub fn texts<'a>(&'a self, strings: &'a StringTable) -> Vec<(&'a SolvedNode, &'a str)> {
        self.nodes
            .iter()
            .filter(|node| node.visible)
            .filter_map(|node| {
                node.text_key
                    .as_deref()
                    .map(|key| (node, strings.text(key)))
            })
            .collect()
    }
}

/// Positions a layout against a viewport, with every tab strip on its first page.
///
/// The root is solved against the whole viewport, so a root sized `Fill` covers the surface and a root
/// sized `Auto` collapses to its content.
#[must_use]
pub fn solve(layout: &Layout, viewport: Viewport, measure: &impl Measure) -> Solved {
    solve_selected(layout, viewport, measure, &NoSelection)
}

/// Positions a layout against a viewport, showing the tab page each strip's selection names.
///
/// [`Interface`](crate::Interface) implements [`Selections`], so a host passes its own state straight in.
#[must_use]
pub fn solve_selected(
    layout: &Layout,
    viewport: Viewport,
    measure: &impl Measure,
    selections: &impl Selections,
) -> Solved {
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
    let mut solved = Solved {
        nodes: arrange.nodes,
        overlay: None,
    };
    solved.hide_unselected_pages(selections);
    solved.resolve_open_combos(selections);
    solved
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
    // A combo is one row high whatever its options number, because its options are not *in* it — an `Auto`
    // combo that summed them would be as tall as its own open list, which is the one size it must never be.
    // Wide enough for the widest option, though, so opening it does not reveal text the closed control
    // could not have shown.
    if node.widget == Widget::Combo {
        return [largest(0), largest(1)];
    }
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
            style: node.style,
            align: node.align,
            id: node.id.clone(),
            text_key: node.text_key.clone(),
            action: node.action,
            range: node.range,
            max_length: node.max_length,
            pages: node.pages.clone(),
            // Everything is visible until the visibility pass says otherwise, which it can only do once
            // the whole sequence exists: a strip's pages container may be anywhere in the tree.
            visible: true,
            children: node.children.len(),
            // The measure pass counted this already, and it counted it over the same tree in the same
            // order, so taking it from there beats walking the children again.
            subtree: self
                .intrinsics
                .get(slot)
                .map_or(1, |intrinsic| intrinsic.subtree),
            parent,
            depth,
        });

        if node.children.is_empty() {
            return;
        }

        let slots = child_slots(node, self.intrinsics, slot);
        if node.widget == Widget::Combo {
            self.options(node, rect, index, depth, &slots);
            return;
        }

        let content = rect.inset(node.padding.scaled(self.scale));
        let rects = child_rects(node, content, self.scale, self.intrinsics, &slots);
        for ((child, child_slot), child_rect) in node.children.iter().zip(&slots).zip(rects) {
            self.node(child, child_rect, Some(index), depth + 1, *child_slot);
        }
    }

    /// Places a combo's options as a list hanging below the control.
    ///
    /// The one arrangement no author could have written, and the reason a combo is a widget kind rather than
    /// a `Panel` with a convention: the options belong *outside* the box that owns them, at that box's width,
    /// over whatever happens to be beneath. Every other node here is positioned by its parent's arrangement
    /// within its parent's content box, and a dropdown is precisely the case that does not fit.
    ///
    /// Padding is deliberately not applied. On every other container it insets the children; here it would
    /// inset the list from the control it hangs off, so the two edges would not line up — and a dropdown
    /// whose list is narrower than its control reads as a rendering fault.
    fn options(&mut self, node: &Node, rect: Rect, index: usize, depth: usize, slots: &[usize]) {
        let mut top = rect.bottom();
        for (child, child_slot) in node.children.iter().zip(slots) {
            let natural = self
                .intrinsics
                .get(*child_slot)
                .map_or([0.0, 0.0], |entry| entry.size);
            // A row is as tall as it asked to be, or as tall as its text measured, floored so that a
            // measurer with nothing to say cannot produce a list of invisible rows. `Fill` means nothing
            // here — there is no leftover to share, since the list's height *is* the sum of its rows.
            let height = match child.height {
                Sizing::Fixed(amount) => amount * self.scale,
                Sizing::Auto | Sizing::Fill(_) => {
                    (natural[1] * self.scale).max(DEFAULT_OPTION_HEIGHT * self.scale)
                }
            };
            let placed = Rect::new(rect.x, top, rect.width, height);
            top = placed.snapped().bottom();
            self.node(child, placed, Some(index), depth + 1, *child_slot);
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

    use super::{Measure, NoContent, NoSelection, Selections, Solved, solve, solve_selected};
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

    /// A [`Selections`] that answers the same chosen index for everything.
    struct Chosen(usize);

    impl Selections for Chosen {
        fn selection(&self, _id: &str) -> Option<usize> {
            Some(self.0)
        }

        fn is_open(&self, _id: &str) -> bool {
            false
        }
    }

    /// A strip of two headers over a stack of two pages, each page holding one child.
    fn tabbed() -> Layout {
        wrap(Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![
                Node {
                    id: Some("tabs".to_owned()),
                    widget: Widget::Tabs,
                    pages: Some("pages".to_owned()),
                    children: vec![Node::default(), Node::default()],
                    ..Node::default()
                },
                Node {
                    id: Some("pages".to_owned()),
                    direction: Direction::Stack,
                    children: vec![
                        Node {
                            id: Some("first".to_owned()),
                            widget: Widget::Button,
                            children: vec![Node::default()],
                            ..Node::default()
                        },
                        Node {
                            id: Some("second".to_owned()),
                            widget: Widget::Button,
                            children: vec![Node::default()],
                            ..Node::default()
                        },
                    ],
                    ..Node::default()
                },
            ],
            ..Node::default()
        })
    }

    fn tabbed_with(selections: &impl Selections) -> Solved {
        let layout = tabbed();
        layout.validate().expect("the fixture must be valid");
        let viewport = Viewport::new(200, 100, 1.0).expect("valid viewport");
        solve_selected(&layout, viewport, &NoContent, selections)
    }

    /// A [`Selections`] whose one combo is open on the given option.
    struct Opened(usize);

    impl Selections for Opened {
        fn selection(&self, _id: &str) -> Option<usize> {
            Some(self.0)
        }

        fn is_open(&self, _id: &str) -> bool {
            true
        }
    }

    /// A row holding a combo of three options, followed by a panel the list would cover.
    ///
    /// The trailing panel is the point: it comes *after* the combo in the sequence, so it is what a naive
    /// drawing order paints over the list and what a naive hit test hands a click on the list to.
    fn with_combo() -> Layout {
        let option = || Node {
            widget: Widget::Label,
            height: Sizing::Fixed(20.0),
            ..Node::default()
        };
        wrap(Node {
            width: Sizing::Fill(1),
            height: Sizing::Fill(1),
            children: vec![
                Node {
                    id: Some("pick".to_owned()),
                    widget: Widget::Combo,
                    width: Sizing::Fixed(120.0),
                    height: Sizing::Fixed(30.0),
                    children: vec![option(), option(), option()],
                    ..Node::default()
                },
                Node {
                    id: Some("beneath".to_owned()),
                    widget: Widget::Button,
                    width: Sizing::Fill(1),
                    height: Sizing::Fill(1),
                    ..Node::default()
                },
            ],
            ..Node::default()
        })
    }

    fn combo_with(selections: &impl Selections) -> Solved {
        let layout = with_combo();
        layout.validate().expect("the fixture must be valid");
        let viewport = Viewport::new(200, 200, 1.0).expect("valid viewport");
        solve_selected(&layout, viewport, &NoContent, selections)
    }

    #[test]
    fn a_combo_hangs_its_options_below_itself_at_its_own_width() {
        // The arrangement no author could have written, which is the whole reason a dropdown is a widget kind
        // rather than a convention over panels. The control is one row tall whatever its options number —
        // summing them, which is what a column would do, would make an `Auto` combo as tall as its own list.
        let solved = combo_with(&Opened(0));
        let control = solved.by_id("pick").expect("the combo").rect;
        assert_eq!(control, Rect::new(0.0, 0.0, 120.0, 30.0));
        let rows = solved.child_rects(solved.index_of("pick").expect("the combo"));
        assert_eq!(
            rows,
            vec![
                Rect::new(0.0, 30.0, 120.0, 20.0),
                Rect::new(0.0, 50.0, 120.0, 20.0),
                Rect::new(0.0, 70.0, 120.0, 20.0),
            ],
            "the options must stack downward from the control's bottom edge, at its width"
        );
    }

    #[test]
    fn a_closed_combo_has_no_options_on_screen_and_an_open_one_is_the_overlay() {
        let closed = combo_with(&NoSelection);
        assert!(
            closed.visible_nodes().count() == closed.len() - 3,
            "a closed dropdown's three options must not be on screen"
        );
        assert!(closed.overlay().is_empty(), "nothing is open");
        // Its own control still is, and still takes a tab stop: what is hidden is the list, not the widget.
        assert_eq!(closed.focus_order(), vec!["pick", "beneath"]);

        let open = combo_with(&Opened(0));
        assert_eq!(open.visible_nodes().count(), open.len());
        let combo = open.index_of("pick").expect("the combo");
        assert_eq!(
            open.overlay(),
            combo..combo + 4,
            "the combo and its options"
        );
    }

    #[test]
    fn an_open_dropdown_takes_a_click_that_lands_over_a_later_sibling() {
        // The hit-test half of "an overlay is drawn last and searched first". The panel beneath fills the
        // whole viewport and comes after the combo in the sequence, so a plain reverse scan would report it
        // for every point — including the ones the list is covering.
        let open = combo_with(&Opened(0));
        let combo = open.index_of("pick").expect("the combo");

        // The middle option's row, which overlaps the panel.
        assert_eq!(
            open.hit(60.0, 60.0).and_then(|node| node.id.as_deref()),
            None,
            "an option row carries no id of its own"
        );
        assert_eq!(
            open.hit_focusable(60.0, 60.0),
            Some(combo),
            "a click on a row belongs to the combo that owns it, not to the panel behind the list"
        );
        // And a point the list does not cover still reaches what is beneath.
        assert_eq!(
            open.hit_focusable(60.0, 150.0)
                .and_then(|index| open.get(index))
                .and_then(|node| node.id.as_deref()),
            Some("beneath")
        );
        // Closed, the same point over the list's former position reaches the panel.
        let closed = combo_with(&NoSelection);
        assert_eq!(
            closed
                .hit_focusable(60.0, 60.0)
                .and_then(|index| closed.get(index))
                .and_then(|node| node.id.as_deref()),
            Some("beneath")
        );
    }

    #[test]
    fn every_tab_page_but_the_chosen_one_is_hidden_along_with_its_subtree() {
        // A page's *subtree* and not just the page, which is the part worth a test: hiding only the page node
        // would leave every control on it clickable and every label on it drawn, over the top of the page
        // that is actually showing.
        let solved = tabbed_with(&Chosen(1));
        let hidden: Vec<&str> = solved
            .nodes()
            .iter()
            .filter(|node| !node.visible)
            .map(|node| node.id.as_deref().unwrap_or("-"))
            .collect();
        // The first page and the one child beneath it.
        assert_eq!(hidden, vec!["first", "-"]);
        assert_eq!(solved.visible_nodes().count(), solved.len() - 2);
        assert_eq!(solved.focus_order(), vec!["tabs", "second"]);
    }

    #[test]
    fn nothing_chosen_shows_the_first_page_and_a_stale_choice_shows_the_last() {
        // Two ends of the same decision. A screen nobody has touched shows its first tab, which is why
        // `solve` can keep its signature and default to `NoSelection`. And a selection left behind by a
        // layout that has since lost a tab is clamped rather than hiding everything: a blank screen reads as
        // a crash, and the last page reads as a screen.
        assert_eq!(
            tabbed_with(&NoSelection).focus_order(),
            vec!["tabs", "first"]
        );
        assert_eq!(
            tabbed_with(&Chosen(9)).focus_order(),
            vec!["tabs", "second"]
        );
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

//! The ground the simulation walks on: a passability grid derived from the heightfield, and the
//! integer A\* search that finds a way across it.
//!
//! [ADR 3001](../../../docs/adr/3001-pathfinding.md) is this module's specification, and its first
//! four decisions are the shape of everything here: passability is **derived** from the terrain
//! rather than authored, the grid **is** the heightfield's own grid, a cell carries a small-integer
//! **cost class** rather than a bit, objects **stamp** it with a footprint that occludes and a
//! passage that grants, and the search is **A\* in integers throughout**.
//!
//! # The grid is layered, and that is the whole of decision 4's difficulty
//!
//! A stamp is not a write. The derivation is kept whole and untouched underneath everything, each
//! object's rectangles are held beside it, and the classes the search reads are *computed* from
//! the two — derivation, then passage over it, then occlusion over that. The reason is one line of
//! the record: **an object's death restores what the layers beneath it say.** A grid that mutated
//! its classes in place could only restore a demolished building's ground by guessing what was
//! there, and the guess is wrong exactly where it matters — a depot raised on a shoreline, blown
//! up, and leaving walkable water behind it.
//!
//! # Why there is no floating point in the search
//!
//! [ADR 0007](../../../docs/adr/0007-simulation-arithmetic.md) would *permit* `f64` costs — addition
//! and comparison are correctly rounded, so two conforming machines cannot disagree about them. The
//! record declines anyway, and the reason is worth restating where the code is: a search accumulates
//! thousands of additions along thousands of frontier paths, and every one of those accumulation
//! orders is a question somebody investigating a desync would have to rule out. Integers make the
//! entire class of question unaskable. An orthogonal step costs `10 ×` the destination's class and a
//! diagonal `14 ×` — the classic approximation of √2, chosen because path cost is a *ranking* device
//! and consistency is the whole requirement.
//!
//! Floating point appears twice, both at the boundary and both correctly rounded: deriving the grid
//! from the terrain's scales, and converting the cell chain back to world-space waypoints.
//!
//! # Where the algorithms come from
//!
//! Both are published and both are written here from their definitions, per the provenance rule in
//! [LICENSING.md](../../../LICENSING.md): nothing in this file is ported, translated or transcribed
//! from another game.
//!
//! - **A\*** — Hart, Nilsson and Raphael, *A Formal Basis for the Heuristic Determination of
//!   Minimum Cost Paths*, IEEE Transactions on Systems Science and Cybernetics 4(2), 1968. The
//!   octile heuristic and the `10`/`14` integer step pair are the standard grid specialisation of
//!   it.
//! - **Bresenham's line algorithm** — Bresenham, *Algorithm for Computer Control of a Digital
//!   Plotter*, IBM Systems Journal 4(1), 1965. Used by [`Ground::walkable_line`] to enumerate the
//!   cells a straight run passes through, which is what string-pulling rests on.
//!
//! # What is not here, and where the rest of the record lives
//!
//! Local avoidance (decision 10) is a *unit* against a *unit* rather than a unit against the
//! ground, so it lives in [`crate::units`] with the roster it mutates. This module's only part in
//! it is refusing a push that would put somebody where they could not have walked.
//!
//! Stamping has one producer so far. The objects a scenario places are the first and currently the
//! only thing that puts a footprint on the ground; construction, demolition and Concord's grading
//! all land through the same [`Ground::reconcile`], and none of them exists yet.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use cic_assets::templates::TemplateSet;
use cic_assets::terrain::Terrain;

use crate::activation::{FORCES, Forces, Placed};
use crate::hash::StateHasher;
use crate::id::ObjectId;
use crate::subsystem::{Subsystem, TickContext};

/// The name the [`Ground`] subsystem is registered and hashed under.
pub const GROUND: &str = "ground";

/// The cost class of a cell nothing may enter: a cliff, or ground under water.
pub const IMPASSABLE: u8 = 0;

/// Metalled road: the best ground there is, and three times as quick as a field.
///
/// The whole ladder below is [ADR 3001](../../../docs/adr/3001-pathfinding.md)'s **amendment A**,
/// accepted 2026-08-01. The record as first written put plain ground at `1` with `0` reserved for
/// impassable, which left nothing cheaper than a field — so grading could restore mud and never
/// improve past it, and Concord's rising income curve was pothole repair. Plain ground moving to `3`
/// is the entire fix; `10`/`14` stays the octile pair, and no arithmetic changes.
///
/// Three times open ground is aggressive on purpose. A road twenty percent better than a field is
/// not a thing three factions go to war over, and this one is the premise.
pub const METALLED: u8 = 1;

/// Graded road: what Concord's engineering leaves behind, twice the speed of a field.
pub const GRADED: u8 = 2;

/// Plain ground, and what [`GroundRules::plain_class`] starts at.
///
/// The reference rung: a template's authored speed is the speed it makes *here*, and every other
/// class is a ratio against it.
pub const PLAIN: u8 = 3;

/// Mud, and the first rung that costs rather than pays.
pub const MUD: u8 = 4;

/// Rubble: passable, slow, and what a road chokes with as wreckage accumulates on it.
pub const RUBBLE: u8 = 5;

/// Every coefficient the ground layer runs on: what makes a cell impassable, what a step costs, and
/// how sharply a route may turn.
///
/// **These are settings rather than constants**, and they are folded into the subsystem hash, which
/// is the reason they are gathered in one place. A coefficient that changes where a unit may walk or
/// which way it goes changes the simulation — two machines running one match with different grades
/// are two different games, and a desync report should name the tick they disagreed rather than
/// leave somebody comparing configuration files.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundRules {
    /// The steepest ground a unit may cross, as rise over run.
    ///
    /// `1.0` is a rise as long as the run — 45° — which is the line this engine calls a cliff by
    /// default. One mover class for now, per ADR 3001 decision 3: ground a truck cannot climb is
    /// ground nobody climbs, and per-class interpretation waits for a mechanic that wants it.
    pub maximum_grade: f64,
    /// The elevation water stands at, or `None` for a map where water blocks nothing.
    ///
    /// Comes from [`Terrain::water_level`], which presentation also floods to — one level, two
    /// readers, so what the player sees as a lake is what the simulation refuses to walk into.
    pub water_level: Option<f64>,
    /// The cost class the derivation assigns to ground it finds walkable.
    ///
    /// The only class the derivation produces, because the terrain says nothing about roads or
    /// rubble — those are *stamps*, and a stamp arrives with the mechanic that applies it. Defaults
    /// to [`PLAIN`]; a map whose open ground is all metalled is a legitimate thing to declare.
    pub plain_class: u8,
    /// The class a template's authored `speed` is the speed for.
    ///
    /// [ADR 3001](../../../docs/adr/3001-pathfinding.md)'s **amendment B**: a cell's class scales
    /// how fast a unit crosses it and not only which route wins, so something has to say which rung
    /// means "the speed on the tin". Every other class is a ratio against this one — at the default
    /// ladder a metalled road is three times [`PLAIN`] and rubble is three fifths of it.
    ///
    /// Separate from [`Self::plain_class`] because they answer different questions: one is what the
    /// terrain derives to, the other is what a speed means. They are equal by default and a map that
    /// paved its open ground would move the first without touching the second.
    pub reference_class: u8,
    /// What one orthogonal step through a cell of class `1` costs.
    pub orthogonal_step: u32,
    /// What one diagonal step through a cell of class `1` costs: the integer approximation of √2
    /// against [`Self::orthogonal_step`], `14` to `10`.
    pub diagonal_step: u32,
    /// How far back from a corner a route starts turning, in metres. `0.0` leaves corners sharp.
    ///
    /// A route's corners are cell centres, so an unrounded one turns in 45° steps at eight-metre
    /// intervals and a unit walking it reads as clockwork. See [`Ground::route`] for what the
    /// rounding does and what it refuses to do.
    pub corner_radius: f64,
    /// How many segments each rounded corner is drawn in. `1` is a plain chamfer.
    ///
    /// Every extra segment is another waypoint to store, hash and step through, so this buys
    /// smoothness at a real price and four is where it stops being visible.
    pub corner_steps: u8,
}

impl Default for GroundRules {
    fn default() -> Self {
        Self {
            maximum_grade: 1.0,
            water_level: None,
            plain_class: PLAIN,
            reference_class: PLAIN,
            orthogonal_step: 10,
            diagonal_step: 14,
            corner_radius: 6.0,
            corner_steps: 4,
        }
    }
}

impl GroundRules {
    /// The rules a host uses when nothing has authored otherwise: the defaults, plus the terrain's
    /// own derived water table.
    #[must_use]
    pub fn derived(terrain: &Terrain) -> Self {
        Self {
            water_level: Some(f64::from(terrain.water_level())),
            ..Self::default()
        }
    }

    /// The rules with corner rounding switched off, which is what a test asserting on exact
    /// waypoints wants.
    #[must_use]
    pub fn with_sharp_corners(self) -> Self {
        Self {
            corner_radius: 0.0,
            ..self
        }
    }
}

/// A rectangle of whole cells: where a stamp landed, and what an edit touched.
///
/// Always at least one cell in each direction and always inside the grid — the constructors clip,
/// so a structure placed against the map edge stamps the part of itself that is on the map rather
/// than being refused or wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    /// The lowest cell column covered.
    pub x: u32,
    /// The lowest cell row covered.
    pub y: u32,
    /// How many columns.
    pub width: u32,
    /// How many rows.
    pub height: u32,
}

impl CellRect {
    /// The rectangle covering both of two cells, inclusive of each.
    fn spanning((ax, ay): (u32, u32), (bx, by): (u32, u32)) -> Self {
        let (x, y) = (ax.min(bx), ay.min(by));
        Self {
            x,
            y,
            width: ax.max(bx) - x + 1,
            height: ay.max(by) - y + 1,
        }
    }

    /// One past the last column.
    const fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    /// One past the last row.
    const fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }

    /// Whether the two rectangles share a cell.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// The cells both rectangles cover, or `None` when they cover none in common.
    fn intersection(self, other: Self) -> Option<Self> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (right, bottom) = (
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }

    /// The smallest rectangle covering both.
    fn union(self, other: Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    /// Every cell in the rectangle, row-major.
    fn cells(self) -> impl Iterator<Item = (u32, u32)> {
        (self.y..self.bottom()).flat_map(move |y| (self.x..self.right()).map(move |x| (x, y)))
    }
}

/// A template's stamp geometry as authored: extents in cells, before a pose puts them anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Geometry {
    /// The extent the object occupies, along its own axes.
    footprint: Option<[u32; 2]>,
    /// The extent it grants, along its own axes, and the class it grants it at.
    passage: Option<([u32; 2], u8)>,
}

/// What one object does to the grid while it stands where it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    /// The cells it occupies, impassable while it stands.
    footprint: Option<CellRect>,
    /// The cells it grants, and the class it grants them at.
    passage: Option<(CellRect, u8)>,
}

impl Stamp {
    /// The one rectangle covering everything this stamp touches.
    ///
    /// A stamp with neither rectangle never reaches the map — [`Ground::stamp_for`] refuses to
    /// build one — and the empty rectangle it would answer with intersects nothing and iterates
    /// nothing, which is the honest degenerate answer rather than a panic guarding an unreachable
    /// case.
    fn extent(self) -> CellRect {
        match (self.footprint, self.passage.map(|(rect, _)| rect)) {
            (Some(occluded), Some(granted)) => occluded.union(granted),
            (Some(rect), None) | (None, Some(rect)) => rect,
            (None, None) => CellRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
        }
    }
}

/// The passability grid: one cost class per cell, and routes across it.
///
/// One cell per heightfield sample interval, so a grid of `(width − 1) × (height − 1)` cells at
/// `horizontal_scale` spacing. There is deliberately no second resolution to keep registered against
/// the first: a height edit maps to the cells it affects by index arithmetic.
#[derive(Debug, Clone)]
pub struct Ground {
    /// Cells along X. One fewer than the terrain's sample count, and zero for a degenerate terrain.
    width: u32,
    /// Cells along Y.
    height: u32,
    /// The world-space size of one cell, in metres.
    cell_size: f64,
    /// The class the *terrain* gives each cell, row-major, and never written to after derivation.
    ///
    /// This is the bottom layer, and keeping it whole is what makes a demolition able to restore
    /// what a building stood on. A second byte per cell is the price; the alternative is holding
    /// the heightfield — two bytes a sample — and re-deriving, which is dearer and slower.
    derived: Vec<u8>,
    /// The effective cost class per cell, row-major: the derivation with every stamp applied over
    /// it. `0` is impassable, and this is what the search reads.
    classes: Vec<u8>,
    /// The coefficients this grid was derived and is searched under.
    rules: GroundRules,
    /// The stamp geometry of every template that declares one, by identifier.
    ///
    /// Captured at construction, exactly as `Units` captures template speeds: what a placement's
    /// `template:` name resolves to has to be settled once, on this side of the tick loop, rather
    /// than looked up through a peer the grid has no business reaching for.
    geometry: BTreeMap<String, Geometry>,
    /// What each object currently stamps, by identifier. The middle and top layers.
    stamps: BTreeMap<ObjectId, Stamp>,
    /// The rectangles the stamps changed on the tick just computed, in the order they were applied.
    ///
    /// ADR 3001 decision 7's input: `units` reads these to decide who repaths, and they are cleared
    /// at the top of every tick because an edit is news for exactly one of them.
    edits: Vec<CellRect>,
    /// The cheapest class the *derivation* produced, before any stamp.
    derived_cheapest: u32,
    /// The cheapest class the grid may hold, which is what the heuristic may assume and no more.
    ///
    /// Kept rather than recomputed because the heuristic asks for it on every cell the search
    /// touches, and it tracks the stamps because a passage can introduce a class cheaper than
    /// anything the derivation produced — a graded road being the whole point of there being a
    /// ladder at all.
    cheapest_class: u32,
    /// The grid folded to one number, per ADR 3001 decision 8.
    fingerprint: u64,
}

impl Ground {
    /// Derives the grid from a heightfield.
    ///
    /// Two tests, both in the permitted operation set trivially:
    ///
    /// - **Slope.** The largest rise between any two *edge-adjacent* corners of the cell, against
    ///   `maximum_grade × cell_size`. Edge-adjacent rather than corner-to-corner because that is
    ///   what "the slope between adjacent height samples" measures, and because a diagonal spans a
    ///   longer run than the threshold is written against.
    /// - **Water.** The cell's *highest* corner decides. A cell entirely beneath the water line is
    ///   lake bed; one whose high corner breaks the surface is a shoreline a unit can wade at, which
    ///   is the difference between a beach a player can land on and a map ringed by an invisible
    ///   wall.
    #[must_use]
    pub fn derive(terrain: &Terrain, rules: GroundRules) -> Self {
        // `n` samples span `n - 1` intervals, and a terrain one sample across spans none — a
        // legitimate heightfield with no ground to walk on, which every method below handles as an
        // empty grid rather than as an error.
        let width = terrain.width().saturating_sub(1);
        let height = terrain.height().saturating_sub(1);
        let cell_size = f64::from(terrain.horizontal_scale());
        let vertical_scale = f64::from(terrain.vertical_scale());
        let maximum_rise = rules.maximum_grade * cell_size;

        let samples = terrain.elevations();
        let stride = terrain.width() as usize;
        let mut classes = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let corner = |dx: usize, dy: usize| samples[(y + dy) * stride + x + dx];
                let [top_left, top_right, bottom_left, bottom_right] =
                    [corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1)];

                // The four edges of the cell. A rise is a difference in quantisation steps until it
                // is scaled, so the subtraction happens in integers and only the comparison is
                // floating point.
                let rise = [
                    top_left.abs_diff(top_right),
                    bottom_left.abs_diff(bottom_right),
                    top_left.abs_diff(bottom_left),
                    top_right.abs_diff(bottom_right),
                ]
                .into_iter()
                .max()
                .unwrap_or(0);
                let too_steep = f64::from(rise) * vertical_scale > maximum_rise;

                let high = top_left.max(top_right).max(bottom_left).max(bottom_right);
                let submerged = rules
                    .water_level
                    .is_some_and(|level| f64::from(high) * vertical_scale < level);

                classes.push(if too_steep || submerged {
                    IMPASSABLE
                } else {
                    rules.plain_class
                });
            }
        }

        let fingerprint = fingerprint(width, height, cell_size, &classes, rules);
        let derived_cheapest = cheapest(&classes);
        Self {
            width,
            height,
            cell_size,
            derived: classes.clone(),
            classes,
            rules,
            geometry: BTreeMap::new(),
            stamps: BTreeMap::new(),
            edits: Vec::new(),
            derived_cheapest,
            cheapest_class: derived_cheapest,
            fingerprint,
        }
    }

    /// Teaches the grid what each template stamps, so placed objects can occlude and grant.
    ///
    /// Without this a grid is pure terrain and nothing that stands on it changes anything, which
    /// is what every kernel assembled before ADR 3001 decision 4 had — a legitimate state, and the
    /// one the replay fixtures that hold no template set still run in.
    ///
    /// Templates that declare neither a `footprint` nor a `passage` are not recorded, so the map
    /// this holds has one entry per *stamping* template rather than one per template: a scenario
    /// whose scenery is a thousand pines reconciles against the handful of things that occlude.
    #[must_use]
    pub fn with_templates(mut self, templates: &TemplateSet) -> Self {
        self.geometry = templates
            .templates
            .iter()
            .filter(|template| template.kind.stamps())
            .filter_map(|template| {
                let geometry = Geometry {
                    footprint: template.footprint.map(|shape| shape.cells),
                    passage: template.passage.map(|shape| (shape.cells, shape.class)),
                };
                (geometry.footprint.is_some() || geometry.passage.is_some())
                    .then(|| (template.id.clone(), geometry))
            })
            .collect();
        self
    }

    /// The coefficients this grid runs on.
    #[must_use]
    pub const fn rules(&self) -> &GroundRules {
        &self.rules
    }

    /// Cells along X.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Cells along Y.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The world-space size of one cell, in metres.
    #[must_use]
    pub const fn cell_size(&self) -> f64 {
        self.cell_size
    }

    /// The cost class at a cell, or [`IMPASSABLE`] outside the grid.
    ///
    /// Off-grid reads as impassable rather than as an error, because that is what it means: there is
    /// no ground past the edge of the map, and a search that had to handle a `None` at every
    /// neighbour would say the same thing at four times the length.
    #[must_use]
    pub fn class(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return IMPASSABLE;
        }
        self.classes[(y * self.width + x) as usize]
    }

    /// Whether a cell may be entered.
    #[must_use]
    pub fn passable(&self, x: u32, y: u32) -> bool {
        self.class(x, y) != IMPASSABLE
    }

    /// The cell a world position falls in, clamped to the grid, or `None` for an empty grid or a
    /// position that is not a finite number.
    #[must_use]
    pub fn cell_at(&self, world: [f64; 2]) -> Option<(u32, u32)> {
        if self.width == 0 || self.height == 0 || !world[0].is_finite() || !world[1].is_finite() {
            return None;
        }
        let axis = |value: f64, cells: u32| {
            let cell = (value / self.cell_size).floor();
            let clamped = cell.clamp(0.0, f64::from(cells - 1));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped into [0, cells) and floored, so it is a whole number in range"
            )]
            {
                clamped as u32
            }
        };
        Some((axis(world[0], self.width), axis(world[1], self.height)))
    }

    /// How fast a unit crosses the ground here, as a multiple of its authored speed.
    ///
    /// [ADR 3001](../../../docs/adr/3001-pathfinding.md)'s **amendment B**. A cell's class ranks
    /// routes *and* sets the pace on them: without this half, grading the whole map would change
    /// which way a unit went and not how long it took, so Concord's paving would be a routing
    /// preference rather than the income increase its doctrine is built on.
    ///
    /// The ratio is [`GroundRules::reference_class`] over the cell's class — a correctly-rounded
    /// division of two small integers, inside ADR 0007's permitted set. Off the grid, on a grid with
    /// no cells, or standing on ground that has turned impassable underneath it, a unit keeps its
    /// authored speed: there is no sensible ratio in any of those cases, and the last one has to let
    /// the unit walk off rather than divide by zero.
    #[must_use]
    pub fn pace_at(&self, world: [f64; 2]) -> f64 {
        let Some((x, y)) = self.cell_at(world) else {
            return 1.0;
        };
        let class = self.class(x, y);
        if class == IMPASSABLE || self.rules.reference_class == IMPASSABLE {
            return 1.0;
        }
        f64::from(self.rules.reference_class) / f64::from(class)
    }

    /// The world position at the centre of a cell.
    #[must_use]
    pub fn centre_of(&self, x: u32, y: u32) -> [f64; 2] {
        [
            (f64::from(x) + 0.5) * self.cell_size,
            (f64::from(y) + 0.5) * self.cell_size,
        ]
    }

    /// The rectangles the stamps changed on the tick just computed.
    ///
    /// Empty on every tick nothing was built, destroyed or moved, which is nearly all of them.
    #[must_use]
    pub fn edits(&self) -> &[CellRect] {
        &self.edits
    }

    /// Whether a route from `from` through `waypoints` passes over ground stamped this tick.
    ///
    /// ADR 3001 decision 7's intersection test, and deliberately an **over-approximation**: each
    /// leg is tested by the bounding box of the cells its two ends fall in, which contains every
    /// cell the leg actually crosses. So it can report a crossing that is not one — costing a
    /// repath that returns the same route — and it cannot miss one, which is the direction that
    /// matters. The record names "a unit whose next step is blocked" as the fallback for whatever
    /// the intersection test misses; a test that cannot miss leaves it nothing to catch.
    #[must_use]
    pub fn route_crosses_an_edit(&self, from: [f64; 2], waypoints: &[[f64; 2]]) -> bool {
        if self.edits.is_empty() {
            return false;
        }
        let mut previous = from;
        for waypoint in waypoints {
            if let (Some(start), Some(end)) = (self.cell_at(previous), self.cell_at(*waypoint)) {
                let leg = CellRect::spanning(start, end);
                if self.edits.iter().any(|edit| edit.intersects(leg)) {
                    return true;
                }
            }
            previous = *waypoint;
        }
        false
    }

    /// Brings the stamps into line with the objects standing on the map.
    ///
    /// One pass a tick over what a scenario placed, which today never changes after activation —
    /// so the interesting cases are the first tick, when everything lands at once, and the ticks
    /// construction and demolition will bring when they exist. Written as a *reconciliation*
    /// rather than as add and remove calls for exactly that reason: an object appearing,
    /// disappearing and moving are one comparison, and a producer that forgets to announce a
    /// removal cannot leave a stamp behind.
    ///
    /// `pub(crate)` because nothing outside this crate can hold `&mut Ground` — the kernel hands
    /// subsystems out immutably — so this is the tick path plus the tests that drive an edit the
    /// way construction eventually will.
    pub(crate) fn reconcile(&mut self, objects: &BTreeMap<ObjectId, Placed>) {
        // An edit is news for one tick, and a reconcile that changes nothing is not news at all.
        self.edits.clear();
        let mut wanted = BTreeMap::new();
        for (id, placed) in objects {
            if let Some(stamp) = self.stamp_for(placed) {
                wanted.insert(*id, stamp);
            }
        }
        if wanted == self.stamps {
            return;
        }

        // Lifted first, then laid, each in identifier order. A stamp that *changed* is both, and
        // in that order: the ground it left has to be restored before the ground it took is
        // denied, or an object that moved one cell would leave the cell behind it walled off.
        let lifted: Vec<(ObjectId, Stamp)> = self
            .stamps
            .iter()
            .filter(|(id, stamp)| wanted.get(id) != Some(stamp))
            .map(|(id, stamp)| (*id, *stamp))
            .collect();
        let laid: Vec<(ObjectId, Stamp)> = wanted
            .iter()
            .filter(|(id, stamp)| self.stamps.get(id) != Some(stamp))
            .map(|(id, stamp)| (*id, *stamp))
            .collect();
        self.stamps = wanted;

        let mut touched = Vec::with_capacity(lifted.len() + laid.len());
        for (laid_down, (id, stamp)) in lifted
            .into_iter()
            .map(|edit| (false, edit))
            .chain(laid.into_iter().map(|edit| (true, edit)))
        {
            self.fold(laid_down, id, stamp);
            touched.push(stamp.extent());
        }
        // Recomputed after every stamp is in place, so a cell two rectangles share sees both.
        for rect in &touched {
            self.recompute(*rect);
        }
        self.cheapest_class = self.cheapest_now();
        self.edits = touched;
    }

    /// Where a placed object's template geometry lands, or `None` when it lands nowhere.
    fn stamp_for(&self, placed: &Placed) -> Option<Stamp> {
        let geometry = *self.geometry.get(&placed.template)?;
        // The ground plane is X and Y; Z is elevation, which the simulation does not walk in.
        let anchor = self.cell_at([placed.position[0], placed.position[1]])?;
        let quarter = quarter_turn(placed.rotation);
        let stamp = Stamp {
            footprint: geometry
                .footprint
                .and_then(|cells| self.rect_at(anchor, cells, quarter)),
            passage: geometry
                .passage
                .and_then(|(cells, class)| Some((self.rect_at(anchor, cells, quarter)?, class))),
        };
        (stamp.footprint.is_some() || stamp.passage.is_some()).then_some(stamp)
    }

    /// The cells an extent covers when its owner stands in `anchor`, turned by `quarter` right
    /// angles and clipped to the grid.
    ///
    /// **Centred on the cell the object stands in**, with an even extent taking the extra cell on
    /// the high side. That last clause is the half-cell ADR 3001's "placements snap to cells"
    /// leaves open, settled one way here rather than left for whoever reads this next: a rectangle
    /// with an even side has no cell at its centre, so something has to break the tie and the tie
    /// is worth half a cell.
    ///
    /// A quarter or three-quarter turn swaps the axes. A half turn maps an axis-aligned rectangle
    /// onto itself and is therefore no change at all.
    ///
    /// An object at the map edge stamps **the part of itself that is on the map**: the rectangle is
    /// clipped where it leaves the grid rather than slid inward, because sliding would put a
    /// building's footprint somewhere its model is not.
    fn rect_at(&self, (ax, ay): (u32, u32), cells: [u32; 2], quarter: u8) -> Option<CellRect> {
        let [across, along] = if quarter.is_multiple_of(2) {
            cells
        } else {
            [cells[1], cells[0]]
        };
        // Signed, because a rectangle centred on the first column starts left of the map and the
        // clip has to happen against where it *is* rather than against an underflow.
        let span = |anchor: u32, extent: u32, limit: u32| -> Option<(u32, u32)> {
            if extent == 0 {
                return None;
            }
            let low = (i64::from(anchor) - i64::from((extent - 1) / 2)).max(0);
            let high = (i64::from(anchor) - i64::from((extent - 1) / 2) + i64::from(extent))
                .min(i64::from(limit));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clipped into [0, limit], and limit is a u32"
            )]
            (high > low).then(|| (low as u32, (high - low) as u32))
        };
        let (x, width) = span(ax, across, self.width)?;
        let (y, height) = span(ay, along, self.height)?;
        Some(CellRect {
            x,
            y,
            width,
            height,
        })
    }

    /// Rebuilds the effective classes in one rectangle from the layers beneath it.
    ///
    /// The precedence is ADR 3001 decision 4's, in the order the record states it: the
    /// **derivation**, then **passage** over the top of it, then **occlusion** over the top of
    /// that. Passage *replaces* what the terrain said rather than competing with it — that is what
    /// puts a bridge over water — while a footprint beats everything, so a structure raised at a
    /// bridgehead denies the bridgehead.
    fn recompute(&mut self, rect: CellRect) {
        let area = (rect.width as usize) * (rect.height as usize);
        if area == 0 {
            return;
        }
        // "Nothing granted this cell" as an absent value rather than a reserved class: a class is
        // a byte and every non-zero value of it is a legal cost, so there is no spare number.
        let mut granted = vec![None::<u8>; area];
        let offset = |x: u32, y: u32| ((y - rect.y) * rect.width + (x - rect.x)) as usize;
        for stamp in self.stamps.values() {
            let Some((over, class)) = stamp.passage else {
                continue;
            };
            let Some(shared) = over.intersection(rect) else {
                continue;
            };
            for (x, y) in shared.cells() {
                let slot = &mut granted[offset(x, y)];
                // Two passages over one cell — a ramp onto a bridge — give the better of the two.
                // Order-independent on purpose: the iteration order here is deterministic anyway,
                // but a rule that depends on which object was built first is a rule somebody has
                // to reason about, and `min` is not.
                *slot = Some(slot.map_or(class, |held| held.min(class)));
            }
        }
        for (x, y) in rect.cells() {
            let cell = (y * self.width + x) as usize;
            self.classes[cell] = granted[offset(x, y)].unwrap_or(self.derived[cell]);
        }
        for stamp in self.stamps.values() {
            let Some(over) = stamp.footprint else {
                continue;
            };
            let Some(shared) = over.intersection(rect) else {
                continue;
            };
            for (x, y) in shared.cells() {
                self.classes[(y * self.width + x) as usize] = IMPASSABLE;
            }
        }
    }

    /// The cheapest class the grid may hold: the derivation's floor, and anything a passage beat it
    /// with.
    ///
    /// A lower bound rather than a survey of the cells, which is all the heuristic needs — an
    /// estimate that prices a step below what any real step costs stays admissible, and a stamp
    /// buried under a footprint only makes the bound looser. Computed from the stamps rather than
    /// from the grid so an edit costs one pass over the objects instead of one over the map.
    fn cheapest_now(&self) -> u32 {
        let granted = self
            .stamps
            .values()
            .filter_map(|stamp| stamp.passage)
            .map(|(_, class)| u32::from(class))
            .filter(|class| *class != u32::from(IMPASSABLE));
        std::iter::once(self.derived_cheapest)
            .chain(granted)
            .min()
            .unwrap_or(1)
            .max(1)
    }

    /// Folds one applied edit into the running fingerprint, per ADR 3001 decision 8.
    ///
    /// A **chain** rather than a rehash of the grid: the grid is a pure function of the terrain
    /// plus the stamps the command log already records, and rehashing tens of millions of cells on
    /// the tick a structure goes up is a price no other subsystem pays. The consequence is
    /// deliberate and worth stating where somebody will read it — the fingerprint records the
    /// *history* of edits and not only their sum, so lifting a stamp does not restore the number
    /// the grid had before it was laid, even though it restores every cell. That catches strictly
    /// more than a state hash would, and there is nothing here that needs to recognise a grid it
    /// has seen before.
    fn fold(&mut self, laid: bool, id: ObjectId, stamp: Stamp) {
        fn rectangle(hasher: &mut StateHasher, rect: Option<CellRect>, class: u8) {
            match rect {
                Some(rect) => {
                    hasher.write_bytes(&[1, class]);
                    for value in [rect.x, rect.y, rect.width, rect.height] {
                        hasher.write_u64(u64::from(value));
                    }
                }
                None => hasher.write_bytes(&[0, 0]),
            }
        }
        let mut hasher = StateHasher::new();
        hasher.write_u64(self.fingerprint);
        hasher.write_bytes(&[u8::from(laid)]);
        hasher.write_u64(id.0);
        rectangle(&mut hasher, stamp.footprint, IMPASSABLE);
        let (granted, class) = match stamp.passage {
            Some((rect, class)) => (Some(rect), class),
            None => (None, IMPASSABLE),
        };
        rectangle(&mut hasher, granted, class);
        self.fingerprint = hasher.finish();
    }

    /// The waypoints a unit at `from` walks to reach `to`, string-pulled, excluding where it stands.
    ///
    /// An empty result means the unit is already where the route would end. An **unreachable**
    /// destination does not fail: the route ends at the nearest cell the search closed, which is
    /// what a player asking for the far side of a wall means, and it costs nothing because the
    /// search has already done the work (ADR 3001 decision 5).
    ///
    /// The last waypoint is the caller's exact `to` when that point is on the map and its cell was
    /// reached, so an order to a precise position still arrives at that position rather than at the
    /// centre of the cell containing it.
    ///
    /// # Corners are rounded, and never through anything
    ///
    /// A string-pulled route still turns at cell centres, and on an eight-metre grid that means a
    /// unit changing direction by 45° on the spot every few strides — which reads as clockwork
    /// rather than as walking. Each interior corner is therefore cut back by
    /// [`GroundRules::corner_radius`] and interpolated across
    /// [`GroundRules::corner_steps`] segments of a quadratic Bézier, the original corner
    /// serving as the control point.
    ///
    /// **Every segment of a rounded corner is checked against the grid, and the corner stays sharp
    /// if any of them would clip.** That is the whole risk of smoothing a path: the search took care
    /// not to cut a corner past an obstacle, and a rounding pass that ignored the grid would put it
    /// straight back — precisely at the inside of the turns a unit makes to get around things.
    #[must_use]
    pub fn route(&self, from: [f64; 2], to: [f64; 2]) -> Vec<[f64; 2]> {
        let (Some(start), Some(goal)) = (self.cell_at(from), self.cell_at(to)) else {
            // No grid, or a position that is not a number: there is no ground to consult, so the
            // straight line the caller asked for is the whole answer.
            return if to[0].is_finite() && to[1].is_finite() {
                vec![to]
            } else {
                Vec::new()
            };
        };
        let start = self.index(start);
        let goal = self.index(goal);
        if start == goal {
            return vec![self.destination(to, goal, goal)];
        }

        let (reached, came_from) = self.search(start, goal);
        if reached == start {
            return Vec::new();
        }

        let chain = trace(&came_from, start, reached);
        // The corners are rounded over the *whole* polyline, `from` included, because the first turn
        // a unit makes is the one out of the cell it is standing in and it is as sharp as any other.
        // The start is dropped again afterwards: a route says where to go, not where you are.
        let mut corners: Vec<[f64; 2]> = std::iter::once(from)
            .chain(
                self.string_pull(&chain)
                    .into_iter()
                    .skip(1)
                    .map(|cell| self.centre_of(cell % self.width, cell / self.width)),
            )
            .collect();
        if let Some(last) = corners.last_mut() {
            *last = self.destination(to, goal, reached);
        }
        let mut route = self.round_corners(&corners);
        route.remove(0);
        route
    }

    /// Replaces each interior corner with a short interpolated arc, wherever the grid allows one.
    fn round_corners(&self, corners: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let radius = self.rules.corner_radius;
        let steps = u32::from(self.rules.corner_steps);
        // Written positively so a radius that is not a number falls out as "do not round" rather
        // than through a negated comparison, which for a partially ordered type reads as though NaN
        // had been thought about when it had not.
        let rounds = corners.len() >= 3 && radius > 0.0 && steps > 0;
        if !rounds {
            return corners.to_vec();
        }

        let mut rounded = vec![corners[0]];
        for index in 1..corners.len() - 1 {
            let (before, corner, after) = (corners[index - 1], corners[index], corners[index + 1]);
            // Half the leg at most, so two corners sharing a short leg cannot cut past each other
            // and turn the route inside out. The entry point also measures from where the arc
            // *actually* left the previous corner, for the same reason.
            let entered_at = *rounded.last().unwrap_or(&before);
            let start = step_toward(
                corner,
                entered_at,
                radius.min(distance(corner, entered_at) / 2.0),
            );
            let end = step_toward(corner, after, radius.min(distance(corner, after) / 2.0));

            let arc: Vec<[f64; 2]> = (1..=steps)
                .map(|step| {
                    let t = f64::from(step) / f64::from(steps);
                    quadratic(start, corner, end, t)
                })
                .collect();

            // Walkability is checked over the arc as a chain, from where the unit will actually be
            // when it starts turning. A single failure keeps the sharp corner rather than any
            // partial rounding, because half a rounded corner is a shape nobody reasoned about.
            let mut probe = entered_at;
            let clear = std::iter::once(start)
                .chain(arc.iter().copied())
                .all(|point| {
                    let ok = self.walkable_between(probe, point);
                    probe = point;
                    ok
                });
            if clear {
                rounded.push(start);
                rounded.extend(arc);
            } else {
                rounded.push(corner);
            }
        }
        rounded.push(corners[corners.len() - 1]);
        rounded
    }

    /// Whether a straight run between two world positions crosses only passable ground.
    ///
    /// `pub(crate)` for the tests that check a route leg by leg — the assertion that catches a
    /// smoothing or repathing pass putting a unit through something.
    pub(crate) fn walkable_between(&self, from: [f64; 2], to: [f64; 2]) -> bool {
        let (Some(from), Some(to)) = (self.cell_at(from), self.cell_at(to)) else {
            return false;
        };
        self.walkable_line(self.index(from), self.index(to))
    }

    /// The flat index of a cell.
    fn index(&self, (x, y): (u32, u32)) -> u32 {
        y * self.width + x
    }

    /// Where the last waypoint actually goes: the caller's exact point when its cell was reached and
    /// the point itself is on the map, and the cell centre otherwise.
    fn destination(&self, to: [f64; 2], goal: u32, reached: u32) -> [f64; 2] {
        let on_the_map = to[0] >= 0.0
            && to[1] >= 0.0
            && to[0] <= f64::from(self.width) * self.cell_size
            && to[1] <= f64::from(self.height) * self.cell_size;
        if reached == goal && on_the_map {
            to
        } else {
            self.centre_of(reached % self.width, reached / self.width)
        }
    }

    /// A\* from `start`, returning the cell it reached and the predecessor of every closed cell.
    ///
    /// `start` is expanded whatever its own class says: a unit standing on ground that became
    /// impassable underneath it must still be able to walk off, and refusing to search would strand
    /// it there permanently.
    fn search(&self, start: u32, goal: u32) -> (u32, Vec<u32>) {
        // Flat arrays indexed by cell, and a binary heap ordered on `(f, cell index)`. No hashed
        // container appears anywhere in here, per ADR 3001 decision 5 and the determinism invariant
        // it applies: a hash map's iteration order is exactly the kind of thing that is identical on
        // one machine and different on another.
        let cells = self.classes.len();
        let mut cost = vec![u32::MAX; cells];
        let mut came_from = vec![u32::MAX; cells];
        let mut closed = vec![false; cells];
        let mut open = BinaryHeap::new();

        cost[start as usize] = 0;
        open.push(Reverse((self.heuristic(start, goal), start)));

        // The consolation prize, kept as the search runs: the closed cell nearest the goal. Ties go
        // to the lower index, unconditionally, for the same reason the frontier's do.
        let mut nearest = start;
        let mut nearest_distance = self.heuristic(start, goal);

        while let Some(Reverse((_, current))) = open.pop() {
            if closed[current as usize] {
                continue;
            }
            closed[current as usize] = true;

            let distance = self.heuristic(current, goal);
            if distance < nearest_distance {
                nearest = current;
                nearest_distance = distance;
            }
            if current == goal {
                return (goal, came_from);
            }

            let x = current % self.width;
            let y = current / self.width;
            for (dx, dy) in NEIGHBOURS {
                let (Some(nx), Some(ny)) = (offset(x, dx, self.width), offset(y, dy, self.height))
                else {
                    continue;
                };
                let class = self.class(nx, ny);
                if class == IMPASSABLE {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                // A diagonal step may not cut a corner past an impassable cell, or a unit's line
                // clips the building it is walking around.
                if diagonal && !(self.passable(nx, y) && self.passable(x, ny)) {
                    continue;
                }
                let step = if diagonal {
                    self.rules.diagonal_step
                } else {
                    self.rules.orthogonal_step
                }
                .saturating_mul(u32::from(class));
                let neighbour = ny * self.width + nx;
                let candidate = cost[current as usize].saturating_add(step);
                if candidate < cost[neighbour as usize] {
                    cost[neighbour as usize] = candidate;
                    came_from[neighbour as usize] = current;
                    let estimate = candidate.saturating_add(self.heuristic(neighbour, goal));
                    open.push(Reverse((estimate, neighbour)));
                }
            }
        }

        (nearest, came_from)
    }

    /// The octile distance between two cells, in the same integer units as a step cost.
    ///
    /// Admissible because it prices every step at the **cheapest class the grid actually holds**, so
    /// no real path can come in under the estimate whatever the ladder is numbered. That is why the
    /// cheapest class is measured rather than assumed: an estimate priced at class `1` on a grid
    /// whose cheapest cell is class `3` would overestimate, and an overestimating A\* quietly stops
    /// returning shortest paths — a bug that looks like "the routes are a bit odd" rather than like
    /// a failure, and exactly the kind a configurable coefficient would otherwise introduce.
    fn heuristic(&self, from: u32, goal: u32) -> u32 {
        let dx = (from % self.width).abs_diff(goal % self.width);
        let dy = (from / self.width).abs_diff(goal / self.width);
        let (short, long) = (dx.min(dy), dx.max(dy));
        self.rules
            .diagonal_step
            .saturating_mul(short)
            .saturating_add(self.rules.orthogonal_step.saturating_mul(long - short))
            .saturating_mul(self.cheapest_class)
    }

    /// Drops every waypoint the unit can walk past **without paying more for it**, so it crosses
    /// open ground in one straight run instead of staircasing along the cells A\* happened to
    /// close.
    ///
    /// # Why the cost test and not just walkability
    ///
    /// Because a shortcut across passable ground is not free once the ground has classes. A route
    /// that goes four rows out of its way to reach a metalled road is *shorter in cost and longer
    /// in metres*, and a smoothing pass that asked only "can the unit walk this line" would pull it
    /// straight back off the road — leaving A\* to make a decision that the next pass silently
    /// undid, and Concord's paving a routing preference nothing acted on.
    ///
    /// The test is exact and costs nothing extra: the straight line's cells are already being
    /// walked to check they are passable, so they are priced on the same walk, and the chain's own
    /// cost comes from a prefix sum. On ground of one class the two are always **equal** — a
    /// Bresenham line between two cells takes the octile-optimal mix of steps, which is what A\*
    /// found — so every shortcut that used to be taken is still taken, and the only ones now
    /// refused are the ones that would have cost the unit something.
    fn string_pull(&self, chain: &[u32]) -> Vec<u32> {
        let Some((&first, rest)) = chain.split_first() else {
            return Vec::new();
        };
        // What the chain itself costs to walk, cell by cell, as a running total.
        let mut accumulated = Vec::with_capacity(chain.len());
        let mut running = 0u32;
        accumulated.push(running);
        for pair in chain.windows(2) {
            running = running.saturating_add(self.step_cost(pair[0], pair[1]));
            accumulated.push(running);
        }

        let mut kept = vec![first];
        let mut anchor = 0;
        let mut probe = 1;
        while probe < chain.len() {
            let along_the_chain = accumulated[probe] - accumulated[anchor];
            if self
                .line_cost(chain[anchor], chain[probe])
                .is_some_and(|direct| direct <= along_the_chain)
            {
                probe += 1;
                continue;
            }
            // The one before the probe was the furthest visible from the anchor. `max` is a
            // stall guard rather than a case: consecutive cells in an A* chain are always visible to
            // each other, so `probe - 1` is always past the anchor, and if that ever stopped being
            // true this would emit an unsmoothed route rather than loop forever.
            anchor = (probe - 1).max(anchor + 1);
            kept.push(chain[anchor]);
            probe = anchor + 1;
        }
        if let Some(&last) = rest.last()
            && kept.last() != Some(&last)
        {
            kept.push(last);
        }
        kept
    }

    /// What one step between two adjacent cells costs, priced exactly as the search prices it.
    fn step_cost(&self, from: u32, to: u32) -> u32 {
        let diagonal =
            (from % self.width) != (to % self.width) && (from / self.width) != (to / self.width);
        let step = if diagonal {
            self.rules.diagonal_step
        } else {
            self.rules.orthogonal_step
        };
        step.saturating_mul(u32::from(self.classes[to as usize]))
    }

    /// Whether a straight line between two cell centres crosses only passable ground.
    fn walkable_line(&self, from: u32, to: u32) -> bool {
        self.line_cost(from, to).is_some()
    }

    /// What a straight line between two cell centres costs to walk, or `None` when it cannot be.
    ///
    /// The cell the line starts in is *not* charged or tested: that is where the unit is standing,
    /// and a unit on ground that turned impassable underneath it must still be allowed to leave.
    /// Every cell the line enters after that must be passable, and a diagonal crossing must satisfy
    /// the same corner rule the search does — otherwise string-pulling would reintroduce exactly
    /// the clipped corners the search took care to avoid.
    ///
    /// The price is the search's own: `10 ×` the entered cell's class for an orthogonal step and
    /// `14 ×` for a diagonal, in integers, so the number this returns is directly comparable with
    /// an accumulated `g` and there is no rounding to reason about between the two.
    fn line_cost(&self, from: u32, to: u32) -> Option<u32> {
        let (mut x, mut y) = (from % self.width, from / self.width);
        let (goal_x, goal_y) = (to % self.width, to / self.width);
        let (dx, dy) = (goal_x.abs_diff(x), goal_y.abs_diff(y));
        let step_x = if goal_x > x { 1 } else { -1 };
        let step_y = if goal_y > y { 1 } else { -1 };

        // Bresenham over the whole line, with both axes stepping in one iteration where the line
        // passes through a corner. Signed error terms, so the comparison against a negative bound is
        // the ordinary one rather than an underflow to guard.
        let (dx, dy) = (i64::from(dx), i64::from(dy));
        let mut error = dx - dy;
        let mut total = 0u32;
        while (x, y) != (goal_x, goal_y) {
            let doubled = error * 2;
            let along_x = doubled > -dy;
            let along_y = doubled < dx;
            let (next_x, next_y) = (
                if along_x {
                    offset(x, step_x, self.width)
                } else {
                    Some(x)
                },
                if along_y {
                    offset(y, step_y, self.height)
                } else {
                    Some(y)
                },
            );
            let (Some(next_x), Some(next_y)) = (next_x, next_y) else {
                return None;
            };
            if along_x && along_y && !(self.passable(next_x, y) && self.passable(x, next_y)) {
                return None;
            }
            if along_x {
                error -= dy;
            }
            if along_y {
                error += dx;
            }
            (x, y) = (next_x, next_y);
            let class = self.class(x, y);
            if class == IMPASSABLE {
                return None;
            }
            let step = if along_x && along_y {
                self.rules.diagonal_step
            } else {
                self.rules.orthogonal_step
            };
            total = total.saturating_add(step.saturating_mul(u32::from(class)));
        }
        Some(total)
    }
}

impl Subsystem for Ground {
    fn name(&self) -> &'static str {
        GROUND
    }

    fn tick(&mut self, context: &mut TickContext<'_>) {
        // Last tick's edits are not this tick's news. Cleared before anything else, so a tick that
        // returns early below still reports nothing touched.
        self.edits.clear();
        // The objects standing on the map as they are *this* tick: `Forces` is registered ahead of
        // the grid, so it has already advanced. No forces at all is a kernel that was never handed
        // a scenario — the replay fixtures assemble one — and not a fault.
        if let Some(forces) = context.peers.read::<Forces>(FORCES) {
            self.reconcile(forces.objects());
        }
    }

    fn write_state(&self, hasher: &mut StateHasher) {
        // The fingerprint rather than the cells, per ADR 3001 decision 8: rehashing tens of millions
        // of cells every tick is a price no other subsystem pays, and the grid is a pure function of
        // the terrain plus the stamps the command log already records. A machine that stamped
        // differently still diverges on the tick it happened.
        hasher.write_u64(u64::from(self.width));
        hasher.write_u64(u64::from(self.height));
        hasher.write_f64(self.cell_size);
        hasher.write_u64(self.fingerprint);
        // And this tick's edits, because `units` reads them during the tick to decide who repaths.
        // They follow from the stamps the fingerprint already chains, so this is belt and braces —
        // but state a peer consults mid-tick is state a divergence could hide in, and it is four
        // numbers on the rare ticks there are any.
        hasher.write_u64(self.edits.len() as u64);
        for rect in &self.edits {
            for value in [rect.x, rect.y, rect.width, rect.height] {
                hasher.write_u64(u64::from(value));
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The eight neighbours, in a fixed order. The order does not decide anything — ties break on cell
/// index — but it is written out rather than generated so a reader can see that it does not.
const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Which quarter of a revolution a heading rounds to: `0`, `1`, `2` or `3` right angles.
///
/// ADR 3001 decision 4: only the *stamp* quantizes, and the visual rotation stays free. Integer
/// arithmetic throughout, because a heading already is one — a full turn is 2^32 and a quarter is
/// 2^30, so adding half a quarter before shifting rounds to nearest with nothing left to round.
fn quarter_turn(rotation: u32) -> u8 {
    // A hair under a full turn rounds up to a fifth quarter, so the mask is what makes it none.
    let rounded = ((u64::from(rotation) + (1 << 29)) >> 30) & 3;
    rounded as u8
}

/// A coordinate one step along an axis, or `None` when that leaves the grid.
fn offset(value: u32, delta: i32, limit: u32) -> Option<u32> {
    let moved = i64::from(value) + i64::from(delta);
    (moved >= 0 && moved < i64::from(limit)).then_some(
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "just bounded into [0, limit), and limit is a u32"
        )]
        {
            moved as u32
        },
    )
}

/// The distance between two world positions.
///
/// Subtraction, multiplication and `sqrt` — all correctly rounded, so this is inside ADR 0007's
/// permitted set and every machine gets the same number.
fn distance(from: [f64; 2], to: [f64; 2]) -> f64 {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    (dx * dx + dy * dy).sqrt()
}

/// The point `along` metres from `from` in the direction of `toward`.
fn step_toward(from: [f64; 2], toward: [f64; 2], along: f64) -> [f64; 2] {
    let span = distance(from, toward);
    if span > 0.0 {
        let scale = along / span;
        [
            from[0] + (toward[0] - from[0]) * scale,
            from[1] + (toward[1] - from[1]) * scale,
        ]
    } else {
        // Coincident points, or a distance that is not a number. There is no direction to step in,
        // and staying put is the answer that cannot produce a waypoint at infinity.
        from
    }
}

/// A point on the quadratic Bézier through `start` and `end` with `control` pulling the middle.
///
/// Written as the expanded polynomial rather than as nested interpolations: both are correct, and
/// this one is three multiplies and two adds per axis with no intermediate to reason about.
fn quadratic(start: [f64; 2], control: [f64; 2], end: [f64; 2], t: f64) -> [f64; 2] {
    let inverse = 1.0 - t;
    let (a, b, c) = (inverse * inverse, 2.0 * inverse * t, t * t);
    [
        start[0] * a + control[0] * b + end[0] * c,
        start[1] * a + control[1] * b + end[1] * c,
    ]
}

/// Walks the predecessor array back from `reached` to `start`, returning the chain forwards.
fn trace(came_from: &[u32], start: u32, reached: u32) -> Vec<u32> {
    let mut chain = vec![reached];
    let mut current = reached;
    while current != start {
        let previous = came_from[current as usize];
        if previous == u32::MAX {
            break;
        }
        chain.push(previous);
        current = previous;
    }
    chain.reverse();
    chain
}

/// The cheapest class any cell holds, floored at one.
///
/// Floored because a class of zero is impassable rather than free, and a heuristic multiplied by
/// zero is a heuristic of zero — admissible, but it turns A\* into Dijkstra and searches the whole
/// map. An all-impassable grid has no cheapest class at all, and one is the honest answer there
/// because nothing will be searched anyway.
fn cheapest(classes: &[u8]) -> u32 {
    classes
        .iter()
        .copied()
        .filter(|class| *class != IMPASSABLE)
        .min()
        .map_or(1, u32::from)
}

/// The grid folded to one number, coefficients included.
///
/// The rules are in here because they decide what the grid *means*: two machines with the same
/// classes and different step costs will disagree about which route is shortest on the first order
/// that has a choice, and that disagreement should surface as a divergence on the tick it happened
/// rather than as two players watching different units take different roads.
fn fingerprint(width: u32, height: u32, cell_size: f64, classes: &[u8], rules: GroundRules) -> u64 {
    let mut hasher = StateHasher::new();
    hasher.write_u64(u64::from(width));
    hasher.write_u64(u64::from(height));
    hasher.write_f64(cell_size);
    hasher.write_bytes(classes);
    hasher.write_f64(rules.maximum_grade);
    match rules.water_level {
        Some(level) => {
            hasher.write_bytes(&[1]);
            hasher.write_f64(level);
        }
        None => hasher.write_bytes(&[0]),
    }
    hasher.write_bytes(&[rules.plain_class, rules.reference_class, rules.corner_steps]);
    hasher.write_u64(u64::from(rules.orthogonal_step));
    hasher.write_u64(u64::from(rules.diagonal_step));
    hasher.write_f64(rules.corner_radius);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cic_assets::templates::{Footprint, Passage, Template, TemplateKind, TemplateSet};
    use cic_assets::terrain::Terrain;

    use super::{
        CellRect, GRADED, GROUND, Ground, GroundRules, IMPASSABLE, METALLED, PLAIN, quarter_turn,
    };
    use crate::activation::Placed;
    use crate::id::ObjectId;
    use crate::subsystem::Subsystem;

    /// A quarter of a revolution, in the binary turns simulation state stores a heading as.
    const QUARTER: u32 = 1 << 30;

    /// Two stamping templates, sized by the caller: `structure/depot` occupies `denies` cells and
    /// grants nothing, `structure/bridge` grants `grants` cells at `class` and occupies nothing.
    ///
    /// The format holds an extent and a class and nothing about what a thing looks like, so the
    /// bridge doubles as the metalled road further down — same stamp, different numbers.
    fn stamping_templates(denies: [u32; 2], grants: [u32; 2], class: u8) -> TemplateSet {
        let entry = |id: &str, footprint, passage| Template {
            id: id.to_owned(),
            kind: TemplateKind::Structure,
            model: Some("models/thing.glb".to_owned()),
            name: None,
            speed: None,
            radius: None,
            footprint,
            passage,
        };
        let set = TemplateSet {
            format_version: 1,
            templates: vec![
                entry("structure/depot", Some(Footprint { cells: denies }), None),
                entry(
                    "structure/bridge",
                    None,
                    Some(Passage {
                        cells: grants,
                        class,
                    }),
                ),
            ],
        };
        set.validate().expect("the fixture is a legal template set");
        set
    }

    /// One placed object, on the ground plane at `at` and turned by `rotation` binary turns.
    fn placed(template: &str, at: [f64; 2], rotation: u32) -> Placed {
        Placed {
            owner: None,
            template: template.to_owned(),
            position: [at[0], at[1], 0.0],
            rotation,
            scale: 1.0,
        }
    }

    /// The object map a reconcile is driven with.
    fn objects(entries: &[(u64, Placed)]) -> BTreeMap<ObjectId, Placed> {
        entries
            .iter()
            .map(|(id, placed)| (ObjectId(*id), placed.clone()))
            .collect()
    }

    /// Every impassable cell, row-major.
    fn blocked(ground: &Ground) -> Vec<(u32, u32)> {
        (0..ground.height())
            .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
            .filter(|(x, y)| !ground.passable(*x, *y))
            .collect()
    }

    /// A terrain from explicit sample heights, one metre between samples and one metre per step.
    fn terrain(width: u32, height: u32, elevations: Vec<u16>) -> Terrain {
        Terrain::new(width, height, 1.0, 1.0, elevations, Vec::new()).expect("a valid terrain")
    }

    /// Flat ground with a wall of `height` running down column `x`, as a `size × size` sample grid.
    fn walled(size: u32, wall_x: u32, wall_height: u16) -> Terrain {
        let elevations = (0..size * size)
            .map(|index| {
                if index % size == wall_x {
                    wall_height
                } else {
                    0
                }
            })
            .collect();
        terrain(size, size, elevations)
    }

    fn rules() -> GroundRules {
        // Sharp corners: these fixtures are one metre a cell and assert on exact waypoints, so the
        // rounding pass -- which is sized in metres and tested separately -- would only obscure what
        // they are about.
        GroundRules {
            maximum_grade: 1.0,
            water_level: None,
            ..GroundRules::default()
        }
        .with_sharp_corners()
    }

    fn flat(size: u32) -> Ground {
        Ground::derive(
            &terrain(size, size, vec![0; (size * size) as usize]),
            rules(),
        )
    }

    /// The sharpest turn anywhere along a route, as a cosine: `1.0` is dead straight and `-1.0` is a
    /// reversal, so a *larger* number is a smoother path.
    fn sharpest_turn(start: [f64; 2], route: &[[f64; 2]]) -> f64 {
        let points: Vec<[f64; 2]> = std::iter::once(start)
            .chain(route.iter().copied())
            .collect();
        points
            .windows(3)
            .map(|window| {
                let leg = |from: [f64; 2], to: [f64; 2]| {
                    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                    let length = (dx * dx + dy * dy).sqrt();
                    if length > 0.0 {
                        [dx / length, dy / length]
                    } else {
                        [0.0, 0.0]
                    }
                };
                let (a, b) = (leg(window[0], window[1]), leg(window[1], window[2]));
                a[0] * b[0] + a[1] * b[1]
            })
            .fold(1.0_f64, f64::min)
    }

    /// A right-angle detour: flat ground with a bar of raised samples forcing one corner.
    fn cornered() -> Terrain {
        let size = 17;
        let mut elevations = vec![0u16; (size * size) as usize];
        for y in 0..10 {
            elevations[(y * size + 8) as usize] = 50;
        }
        terrain(size, size, elevations)
    }

    #[test]
    fn rounding_a_corner_makes_the_turn_less_sharp() {
        // The same route twice, from the same grid, differing only in the coefficient. Asserting
        // against the *sharp* route rather than against a number is what makes this a statement
        // about rounding instead of a transcription of whatever the code currently emits.
        let start = [3.5, 1.5];
        let target = [12.5, 1.5];
        let sharp = Ground::derive(&cornered(), rules());
        let round = Ground::derive(
            &cornered(),
            GroundRules {
                corner_radius: 2.0,
                corner_steps: 4,
                ..rules()
            },
        );

        let sharp_route = sharp.route(start, target);
        let round_route = round.route(start, target);
        assert!(sharp_route.len() >= 2, "the fixture needs a corner in it");
        assert!(
            round_route.len() > sharp_route.len(),
            "rounding produced no extra waypoints: {round_route:?}"
        );
        assert!(
            sharpest_turn(start, &round_route) > sharpest_turn(start, &sharp_route),
            "the rounded route turns as sharply as the unrounded one"
        );
        assert_eq!(
            round_route.last(),
            sharp_route.last(),
            "rounding moved the destination"
        );
    }

    #[test]
    fn a_corner_stays_sharp_when_rounding_it_would_clip() {
        // A radius far larger than the gap the route squeezes through. The cut has to be refused:
        // the search went out of its way not to clip that corner, and a rounding pass that ignored
        // the grid would put the clip straight back at exactly the turns that exist to avoid things.
        let ground = Ground::derive(
            &cornered(),
            GroundRules {
                corner_radius: 40.0,
                corner_steps: 4,
                ..rules()
            },
        );
        let start = [3.5, 1.5];
        let route = ground.route(start, [12.5, 1.5]);
        assert!(
            route.len() >= 2,
            "a route with no interior corner cannot show a corner being refused"
        );

        let mut previous = start;
        for waypoint in &route {
            assert!(
                ground.walkable_between(previous, *waypoint),
                "leg {previous:?} -> {waypoint:?} crosses impassable ground"
            );
            previous = *waypoint;
        }
    }

    #[test]
    fn a_renumbered_cost_ladder_finds_the_same_route() {
        // Amendment A in a test: plain ground at class 3 rather than 1. Every step costs three
        // times as much and the *ranking* is unchanged, so the route must be identical — which is
        // only true because the heuristic prices itself against the cheapest class the grid holds
        // rather than assuming that class is 1. An overestimating heuristic would quietly stop
        // returning shortest paths, and this is the test that would notice.
        let start = [3.5, 1.5];
        let target = [12.5, 1.5];
        let ladder = |plain_class| {
            Ground::derive(
                &cornered(),
                GroundRules {
                    plain_class,
                    ..rules()
                },
            )
            .route(start, target)
        };
        assert_eq!(ladder(1), ladder(3));
        assert_eq!(ladder(1), ladder(7));
    }

    #[test]
    fn a_coefficient_that_changes_the_game_changes_the_hash() {
        // Two machines with the same terrain and different coefficients are playing two different
        // games, and the point of folding the rules into the fingerprint is that they find that out
        // on tick zero rather than by watching units take different roads.
        let hash = |rules| {
            let mut hasher = crate::hash::StateHasher::new();
            Ground::derive(&cornered(), rules).write_state(&mut hasher);
            hasher.finish()
        };
        let base = rules();
        assert_ne!(
            hash(base),
            hash(GroundRules {
                maximum_grade: 0.1,
                ..base
            })
        );
        assert_ne!(
            hash(base),
            hash(GroundRules {
                diagonal_step: 15,
                ..base
            })
        );
        assert_ne!(
            hash(base),
            hash(GroundRules {
                corner_radius: 2.0,
                ..base
            })
        );
        assert_ne!(
            hash(base),
            hash(GroundRules {
                plain_class: super::GRADED,
                ..base
            })
        );
        assert_ne!(
            hash(base),
            hash(GroundRules {
                reference_class: super::GRADED,
                ..base
            }),
            "the class a speed is measured against decides how fast everything moves"
        );
        assert_eq!(
            hash(base),
            hash(base),
            "the same rules must agree with themselves"
        );
    }

    #[test]
    fn the_grid_is_one_cell_per_sample_interval() {
        let ground = flat(9);
        assert_eq!((ground.width(), ground.height()), (8, 8));
        assert!((ground.cell_size() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_terrain_too_small_to_have_intervals_is_an_empty_grid_not_an_error() {
        // One sample across spans no intervals. Every query has to survive it, because a caller that
        // panics here turns a legal heightfield into a crash.
        let ground = Ground::derive(&terrain(1, 1, vec![7]), rules());
        assert_eq!((ground.width(), ground.height()), (0, 0));
        assert_eq!(ground.class(0, 0), IMPASSABLE);
        assert_eq!(ground.cell_at([0.0, 0.0]), None);
        assert_eq!(ground.route([0.0, 0.0], [5.0, 5.0]), vec![[5.0, 5.0]]);
    }

    #[test]
    fn a_slope_past_the_grade_is_impassable_and_one_under_it_is_not() {
        // One metre of run per cell, so the grade *is* the rise. A step of one is exactly at the
        // threshold and passable; a step of two is over it. The pair is the point: a test that only
        // checked the wall would pass just as well against an implementation that refused every
        // slope.
        let gentle = Ground::derive(&terrain(3, 2, vec![0, 1, 2, 0, 1, 2]), rules());
        assert_eq!(gentle.class(0, 0), PLAIN);
        assert_eq!(gentle.class(1, 0), PLAIN);

        let steep = Ground::derive(&terrain(3, 2, vec![0, 2, 4, 0, 2, 4]), rules());
        assert_eq!(steep.class(0, 0), IMPASSABLE);
        assert_eq!(steep.class(1, 0), IMPASSABLE);
    }

    #[test]
    fn a_cell_wholly_under_water_is_impassable_and_a_shoreline_cell_is_not() {
        // A basin two samples wide against ground at ten, with the water line at five.
        let terrain = terrain(4, 2, vec![0, 0, 10, 10, 0, 0, 10, 10]);
        let ground = Ground::derive(
            &terrain,
            GroundRules {
                maximum_grade: 20.0,
                water_level: Some(5.0),
                ..GroundRules::default()
            },
        );
        assert_eq!(
            ground.class(0, 0),
            IMPASSABLE,
            "both corners are under water"
        );
        assert_eq!(
            ground.class(1, 0),
            PLAIN,
            "a cell whose high corner breaks the surface is a shore, not lake bed"
        );
        assert_eq!(ground.class(2, 0), PLAIN);
    }

    #[test]
    fn a_route_across_open_ground_is_one_straight_run() {
        // String-pulling is what this asserts. Without it the route would be every cell A* closed,
        // which on open ground is a staircase eight waypoints long.
        let ground = flat(9);
        let route = ground.route([0.5, 0.5], [7.5, 7.5]);
        assert_eq!(route, vec![[7.5, 7.5]]);
    }

    #[test]
    fn a_route_around_a_wall_goes_around_it_and_only_over_passable_ground() {
        // A wall down the middle with a gap at the bottom row. The straight line is blocked, so the
        // route has to bend through the gap — and every leg of it has to stay on passable ground,
        // which is the assertion that would catch a string-pull that smoothed back through the wall.
        // A raised *sample* makes the cells on both sides of it steep, so the wall is two cells
        // thick and a gap needs two clear sample rows to appear at all. Getting that wrong is how
        // the first version of this test asserted against a map with no gap in it.
        let size = 9;
        let mut elevations = vec![0u16; (size * size) as usize];
        for y in 0..size - 2 {
            elevations[(y * size + 4) as usize] = 50;
        }
        let ground = Ground::derive(&terrain(size, size, elevations), rules());
        assert!(!ground.passable(3, 0), "the wall is where the test put it");
        assert!(ground.passable(3, 7), "the gap is where the test put it");

        let start = [2.5, 0.5];
        let route = ground.route(start, [5.5, 0.5]);
        assert!(route.len() > 1, "a straight line would have been one leg");
        assert_eq!(route.last().copied(), Some([5.5, 0.5]));

        let mut previous = start;
        for waypoint in route {
            let (from, to) = (
                ground.cell_at(previous).expect("on the grid"),
                ground.cell_at(waypoint).expect("on the grid"),
            );
            let (from, to) = (
                from.1 * ground.width() + from.0,
                to.1 * ground.width() + to.0,
            );
            assert!(
                ground.walkable_line(from, to),
                "leg {previous:?} -> {waypoint:?} crosses impassable ground"
            );
            previous = waypoint;
        }
    }

    #[test]
    fn an_unreachable_target_degrades_to_the_nearest_reachable_cell() {
        // A wall right across the map: the far side cannot be reached at all. The order still does
        // something — best effort toward it — rather than being silently dropped.
        let ground = Ground::derive(&walled(9, 4, 50), rules());
        let route = ground.route([0.5, 0.5], [7.5, 7.5]);
        let last = *route.last().expect("a best-effort route");
        assert!(
            last[0] < 4.0,
            "the route ended past the wall at {last:?}, which nothing can cross"
        );
        let cell = ground.cell_at(last).expect("on the grid");
        assert!(ground.passable(cell.0, cell.1));
    }

    #[test]
    fn a_unit_standing_on_impassable_ground_can_still_walk_off_it() {
        // Ground can turn impassable underneath something. Refusing to search from there is how a
        // unit gets stranded permanently, so the start cell is expanded whatever its class says.
        let ground = Ground::derive(&walled(9, 4, 50), rules());
        assert!(!ground.passable(4, 4));
        let route = ground.route(ground.centre_of(4, 4), [7.5, 4.5]);
        assert_eq!(route.last().copied(), Some([7.5, 4.5]));
    }

    #[test]
    fn a_diagonal_step_may_not_cut_a_corner() {
        // The destination cell is *passable*, which is the whole point: only the corner rule can
        // refuse this step, so the assertion cannot be satisfied by an implementation that merely
        // checks passability. Two raised samples block the cells north and east of the origin while
        // leaving the one diagonally beyond them clear.
        let size = 5;
        let mut elevations = vec![0u16; (size * size) as usize];
        for (x, y) in [(2, 0), (0, 2)] {
            elevations[(y * size + x) as usize] = 50;
        }
        let ground = Ground::derive(&terrain(size, size, elevations), rules());
        assert!(
            ground.passable(0, 0) && ground.passable(1, 1),
            "the corners are clear"
        );
        assert!(
            !ground.passable(1, 0) && !ground.passable(0, 1),
            "the sides are blocked"
        );

        let diagonal = ground.width() + 1;
        assert!(
            !ground.walkable_line(0, diagonal),
            "a line squeezed between two blocked cells clips both of them"
        );
        // And the search obeys the same rule, which is what makes the pocket a pocket: every way out
        // of the origin cell is either blocked or a corner, so a best-effort route is no route.
        assert!(ground.route([0.5, 0.5], ground.centre_of(1, 1)).is_empty());
    }

    #[test]
    fn the_route_between_two_points_in_one_cell_is_the_point_itself() {
        let ground = flat(9);
        assert_eq!(ground.route([0.2, 0.3], [0.7, 0.9]), vec![[0.7, 0.9]]);
    }

    #[test]
    fn a_target_off_the_map_ends_at_the_edge_cell_rather_than_off_it() {
        let ground = flat(9);
        let route = ground.route([0.5, 0.5], [100.0, 0.5]);
        assert_eq!(route.last().copied(), Some(ground.centre_of(7, 0)));
    }

    #[test]
    fn a_footprint_denies_the_ground_it_stands_on() {
        let mut ground = flat(9).with_templates(&stamping_templates([3, 1], [1, 1], GRADED));
        assert!(
            ground.passable(4, 4),
            "the ground was open before anything stood on it"
        );

        ground.reconcile(&objects(&[(
            1,
            placed("structure/depot", ground.centre_of(4, 4), 0),
        )]));
        assert_eq!(
            blocked(&ground),
            vec![(3, 4), (4, 4), (5, 4)],
            "three cells wide, centred on the cell it stands in, and nothing else"
        );
    }

    #[test]
    fn a_passage_grants_ground_the_terrain_refused() {
        // A wall right across the map — the same fixture the unreachable-target test uses, where a
        // route from one side ends before it. A bridge over three of its cells makes the crossing,
        // and the *route* is the assertion rather than the class, because a passage that changed
        // the grid without changing where a unit may go would satisfy a weaker test.
        let mut ground = Ground::derive(&walled(9, 4, 50), rules())
            .with_templates(&stamping_templates([1, 1], [3, 1], GRADED));
        assert!(
            !ground.passable(3, 4) && !ground.passable(4, 4),
            "the wall is two cells thick"
        );
        assert!(ground.route([0.5, 4.5], [7.5, 4.5]).last() != Some(&[7.5, 4.5]));

        ground.reconcile(&objects(&[(
            1,
            placed("structure/bridge", ground.centre_of(4, 4), 0),
        )]));
        assert_eq!(ground.class(3, 4), GRADED);
        assert_eq!(ground.class(4, 4), GRADED);
        assert_eq!(
            ground.class(5, 4),
            GRADED,
            "passage overrides the derivation rather than competing with it, so it re-classes \
             ground that was already walkable"
        );
        assert_eq!(
            ground.route([0.5, 4.5], [7.5, 4.5]).last().copied(),
            Some([7.5, 4.5]),
            "the far side is reachable now"
        );
    }

    #[test]
    fn a_footprint_beats_a_passage_and_the_order_they_landed_in_decides_nothing() {
        // ADR 3001 decision 4's precedence: derivation, then passage, then occlusion — so a
        // structure raised at a bridgehead denies the bridgehead. Asserted both ways round because
        // a "last stamp wins" implementation passes one of the two orders by luck.
        let templates = stamping_templates([1, 1], [3, 1], GRADED);
        let classes = |first: &str, second: &str| {
            let mut ground = Ground::derive(&walled(9, 4, 50), rules()).with_templates(&templates);
            ground.reconcile(&objects(&[
                (1, placed(first, [4.5, 4.5], 0)),
                (2, placed(second, [4.5, 4.5], 0)),
            ]));
            [ground.class(3, 4), ground.class(4, 4), ground.class(5, 4)]
        };
        let expected = [GRADED, IMPASSABLE, GRADED];
        assert_eq!(classes("structure/bridge", "structure/depot"), expected);
        assert_eq!(classes("structure/depot", "structure/bridge"), expected);
    }

    #[test]
    fn lifting_a_stamp_restores_what_is_underneath_and_not_what_is_ordinary() {
        // The one that decides whether the layered model was worth building. A single footprint
        // covers lake bed *and* open ground; when it goes, each cell has to come back as what the
        // terrain says it is. An implementation that wrote the stamp over the classes and restored
        // "plain" on removal gets two of these three cells right and leaves a walkable lake, which
        // is a bug nobody notices until a unit wades across one.
        let shore = terrain(4, 2, vec![0, 0, 10, 10, 0, 0, 10, 10]);
        let mut ground = Ground::derive(
            &shore,
            GroundRules {
                maximum_grade: 20.0,
                water_level: Some(5.0),
                ..GroundRules::default()
            },
        )
        .with_templates(&stamping_templates([3, 1], [1, 1], GRADED));

        let before: Vec<u8> = (0..3).map(|x| ground.class(x, 0)).collect();
        assert_eq!(
            before,
            vec![IMPASSABLE, PLAIN, PLAIN],
            "the fixture needs both kinds of ground under one stamp, or it cannot fail"
        );

        ground.reconcile(&objects(&[(1, placed("structure/depot", [1.5, 0.5], 0))]));
        assert_eq!(blocked(&ground), vec![(0, 0), (1, 0), (2, 0)]);

        ground.reconcile(&BTreeMap::new());
        let after: Vec<u8> = (0..3).map(|x| ground.class(x, 0)).collect();
        assert_eq!(
            after, before,
            "lifting the stamp did not restore what was underneath it"
        );
    }

    #[test]
    fn a_footprint_stamps_in_quarter_turns_and_a_free_angle_snaps_to_one() {
        let templates = stamping_templates([3, 1], [1, 1], GRADED);
        let turned = |rotation| {
            let mut ground = flat(9).with_templates(&templates);
            ground.reconcile(&objects(&[(
                1,
                placed("structure/depot", [4.5, 4.5], rotation),
            )]));
            blocked(&ground)
        };
        assert_eq!(turned(0), vec![(3, 4), (4, 4), (5, 4)]);
        assert_eq!(
            turned(QUARTER),
            vec![(4, 3), (4, 4), (4, 5)],
            "a quarter turn stands the rectangle on end"
        );
        assert_eq!(
            turned(2 * QUARTER),
            turned(0),
            "a half turn maps an axis-aligned rectangle onto itself"
        );
        assert_eq!(turned(3 * QUARTER), turned(QUARTER));

        // Eighty degrees is nearer a right angle than nothing and forty is nearer nothing, so the
        // *visual* rotation stays free while the stamp lands on one of four rectangles.
        assert_eq!(
            turned(954_437_177),
            turned(QUARTER),
            "eighty degrees snaps to ninety"
        );
        assert_eq!(
            turned(477_218_588),
            turned(0),
            "forty degrees snaps to nothing"
        );
        assert_eq!(
            [
                quarter_turn(0),
                quarter_turn(QUARTER),
                quarter_turn(2 * QUARTER),
                quarter_turn(3 * QUARTER),
                quarter_turn(u32::MAX),
            ],
            [0, 1, 2, 3, 0],
            "a heading a hair under a full turn is no turn, not a fifth quarter"
        );
    }

    #[test]
    fn a_stamped_road_is_worth_a_detour() {
        // The class a passage grants has to reach the *search*, not only the grid. It is also the
        // admissibility check with teeth: the heuristic prices itself against the cheapest class
        // the grid holds, and a stamp is how a class cheaper than anything the terrain derived
        // gets there at all — priced against `PLAIN` this search would overestimate and quietly
        // stop returning shortest paths.
        let (start, target) = ([0.5, 12.5], [15.5, 12.5]);
        let open = Ground::derive(&terrain(17, 17, vec![0u16; 17 * 17]), rules());
        assert_eq!(
            open.route(start, target),
            vec![target],
            "over open ground the straight line is the whole route"
        );

        let mut roaded = open.with_templates(&stamping_templates([1, 1], [16, 1], METALLED));
        roaded.reconcile(&objects(&[(1, placed("structure/bridge", [7.5, 8.5], 0))]));
        assert_eq!(
            roaded.class(0, 8),
            METALLED,
            "the road is where the test put it"
        );
        assert_eq!(roaded.class(0, 12), PLAIN, "and the field beside it is not");

        let route = roaded.route(start, target);
        assert!(
            route
                .iter()
                .any(|point| roaded.cell_at(*point).is_some_and(|(_, y)| y == 8)),
            "four rows out of the way is worth a road three times as quick, and the route did not \
             take it: {route:?}"
        );
    }

    #[test]
    fn an_edit_reports_the_cells_it_touched_and_only_on_the_tick_it_touched_them() {
        let templates = stamping_templates([3, 1], [1, 1], GRADED);
        let mut ground = flat(9).with_templates(&templates);
        let depot = objects(&[(1, placed("structure/depot", [4.5, 4.5], 0))]);
        let rect = CellRect {
            x: 3,
            y: 4,
            width: 3,
            height: 1,
        };
        assert!(ground.edits().is_empty(), "nothing has been built yet");

        ground.reconcile(&depot);
        assert_eq!(ground.edits(), [rect]);
        ground.reconcile(&depot);
        assert!(
            ground.edits().is_empty(),
            "a reconcile that changes nothing must report nothing, or every unit repaths every tick"
        );
        ground.reconcile(&BTreeMap::new());
        assert_eq!(
            ground.edits(),
            [rect],
            "lifting touches the same ground laying did"
        );
    }

    #[test]
    fn a_route_is_told_when_the_ground_under_it_moved() {
        let mut ground = flat(9).with_templates(&stamping_templates([3, 1], [1, 1], GRADED));
        ground.reconcile(&objects(&[(1, placed("structure/depot", [4.5, 4.5], 0))]));

        assert!(
            ground.route_crosses_an_edit([0.5, 4.5], &[[7.5, 4.5]]),
            "a leg straight through the new wall"
        );
        assert!(
            ground.route_crosses_an_edit([4.5, 0.5], &[[4.5, 7.5]]),
            "and one across it the other way"
        );
        assert!(
            !ground.route_crosses_an_edit([0.5, 0.5], &[[3.5, 0.5], [3.5, 2.5]]),
            "a route nowhere near it is not disturbed"
        );
        assert!(
            !flat(9).route_crosses_an_edit([0.5, 4.5], &[[7.5, 4.5]]),
            "a grid nothing edited disturbs nobody"
        );
    }

    #[test]
    fn every_stamp_moves_the_fingerprint_and_two_machines_stamp_the_same_one() {
        let templates = stamping_templates([3, 1], [1, 1], GRADED);
        let stamped = |at: [f64; 2]| {
            let mut ground = flat(9).with_templates(&templates);
            ground.reconcile(&objects(&[(1, placed("structure/depot", at, 0))]));
            ground
        };
        let bare = flat(9).with_templates(&templates).fingerprint;
        let ours = stamped([4.5, 4.5]);
        assert_ne!(
            ours.fingerprint, bare,
            "raising a structure changed nothing in the hash"
        );
        assert_eq!(
            stamped([4.5, 4.5]).fingerprint,
            ours.fingerprint,
            "two machines that stamped the same thing must agree"
        );
        assert_ne!(
            stamped([5.5, 4.5]).fingerprint,
            ours.fingerprint,
            "a structure one cell over is a different game"
        );

        // A chain, not a state hash — deliberately, per decision 8. Lifting restores every cell
        // and does not restore the number, because what is folded is the edit as it is applied.
        let mut lifted = stamped([4.5, 4.5]);
        lifted.reconcile(&BTreeMap::new());
        assert_ne!(lifted.fingerprint, ours.fingerprint);
        assert_ne!(lifted.fingerprint, bare);
        assert_eq!(
            blocked(&lifted),
            Vec::new(),
            "and the cells themselves are back, which is the half that has to be exact"
        );
    }

    #[test]
    fn a_grid_told_no_templates_is_pure_terrain() {
        // The state every kernel had before decision 4, and the one the replay fixtures still run
        // in. A placement is not a stamp on its own — something has to have said what the template
        // occupies.
        let mut ground = flat(9);
        ground.reconcile(&objects(&[(1, placed("structure/depot", [4.5, 4.5], 0))]));
        assert_eq!(blocked(&ground), Vec::new());
        assert!(ground.edits().is_empty());
    }

    #[test]
    fn a_structure_at_the_map_edge_stamps_the_part_of_itself_that_is_on_the_map() {
        let mut ground = flat(9).with_templates(&stamping_templates([5, 1], [1, 1], GRADED));
        ground.reconcile(&objects(&[(1, placed("structure/depot", [0.5, 0.5], 0))]));
        assert_eq!(
            blocked(&ground),
            vec![(0, 0), (1, 0), (2, 0)],
            "clipped at the edge rather than refused, and certainly rather than wrapped"
        );
    }

    #[test]
    fn the_same_terrain_derives_the_same_fingerprint_and_a_changed_one_does_not() {
        let hash = |ground: &Ground| {
            let mut hasher = crate::hash::StateHasher::new();
            ground.write_state(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(&flat(9)), hash(&flat(9)));
        assert_ne!(
            hash(&flat(9)),
            hash(&Ground::derive(&walled(9, 4, 50), rules()))
        );
        assert_eq!(flat(9).name(), GROUND);
    }
}

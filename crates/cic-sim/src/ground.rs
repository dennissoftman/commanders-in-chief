//! The ground the simulation walks on: a passability grid derived from the heightfield, and the
//! integer A\* search that finds a way across it.
//!
//! [ADR 3001](../../../docs/adr/3001-pathfinding.md) is this module's specification, and its first
//! four decisions are the shape of everything here: passability is **derived** from the terrain
//! rather than authored, the grid **is** the heightfield's own grid, a cell carries a small-integer
//! **cost class** rather than a bit, and the search is **A\* in integers throughout**.
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
//! # What is not here yet
//!
//! Template `footprint` and `passage` stamps (decision 4), repathing on grid edits (decision 7), and
//! local avoidance (decision 10). Each arrives with the mechanic that produces it — construction and
//! demolition are what edit a grid, and nothing constructs anything yet — which is the same growth
//! rule the template format follows. The grid's fingerprint is folded into the tick hash from the
//! start so that when stamps do arrive, they fold into a number that is already being compared.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use cic_assets::terrain::Terrain;

use crate::hash::StateHasher;
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
    /// Cost class per cell, row-major. `0` is impassable.
    classes: Vec<u8>,
    /// The coefficients this grid was derived and is searched under.
    rules: GroundRules,
    /// The cheapest class any cell holds, which is what the heuristic may assume and no more.
    ///
    /// Kept rather than recomputed because the heuristic asks for it on every cell the search
    /// touches, and derived from the grid rather than from the rules because a stamp can introduce
    /// a class cheaper than anything the derivation produced — a graded road being the whole point
    /// of there being a ladder at all.
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
        Self {
            width,
            height,
            cell_size,
            cheapest_class: cheapest(&classes),
            classes,
            rules,
            fingerprint,
        }
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
    fn walkable_between(&self, from: [f64; 2], to: [f64; 2]) -> bool {
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

    /// Drops every waypoint the unit can walk past, so it crosses open ground in one straight run
    /// instead of staircasing along the cells A\* happened to close.
    fn string_pull(&self, chain: &[u32]) -> Vec<u32> {
        let Some((&first, rest)) = chain.split_first() else {
            return Vec::new();
        };
        let mut kept = vec![first];
        let mut anchor = 0;
        let mut probe = 1;
        while probe < chain.len() {
            if self.walkable_line(chain[anchor], chain[probe]) {
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

    /// Whether a straight line between two cell centres crosses only passable ground.
    ///
    /// The cell the line starts in is *not* tested: that is where the unit is standing, and a unit on
    /// ground that turned impassable underneath it must still be allowed to leave. Every cell the
    /// line enters after that must be passable, and a diagonal crossing must satisfy the same corner
    /// rule the search does — otherwise string-pulling would reintroduce exactly the clipped corners
    /// the search took care to avoid.
    fn walkable_line(&self, from: u32, to: u32) -> bool {
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
                return false;
            };
            if along_x && along_y && !(self.passable(next_x, y) && self.passable(x, next_y)) {
                return false;
            }
            if along_x {
                error -= dy;
            }
            if along_y {
                error += dx;
            }
            (x, y) = (next_x, next_y);
            if !self.passable(x, y) {
                return false;
            }
        }
        true
    }
}

impl Subsystem for Ground {
    fn name(&self) -> &'static str {
        GROUND
    }

    fn tick(&mut self, _context: &mut TickContext<'_>) {
        // Deliberately nothing. A grid changes when something stamps it — a structure raised, a
        // bridge lost, a road graded — and nothing constructs or destroys anything yet. The day one
        // does, this is where an edit lands and where the repath it triggers is decided.
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
    use cic_assets::terrain::Terrain;

    use super::{GROUND, Ground, GroundRules, IMPASSABLE, PLAIN};
    use crate::subsystem::Subsystem;

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

//! Deciding what not to draw: view frusta, and the terrain's chunk decomposition.
//!
//! # Why this exists
//!
//! `TerrainRenderer` submits its whole heightfield in one call, and every shadow cascade submits it
//! again — five full submissions a frame with nothing culled at any of them. On a 257x257 terrain that is
//! about two million vertices, and on a map at the size this project is aimed at it is tens of millions.
//! M3's charter asked for terrain level of detail and this is its first half: a decomposition into chunks
//! with bounds, and the frustum test that rejects them. LOD is the second half and wants the same chunks.
//!
//! # Why it is arithmetic, kept away from the GPU
//!
//! A culling bug is silent in the worst way. Too *little* culling costs performance and looks correct;
//! too much removes geometry that should be there, and on a heightfield that reads as a hole in the world
//! rather than as a missing object. Neither shows up in a frame-time number. So the frustum extraction,
//! the box test and the chunk layout are pure functions over plain numbers with their own tests, in the
//! same spirit as the cascade fitting in [`crate::shadow`] — which is the other piece of this renderer
//! whose failures are geometric and invisible to assertions about pixels.

use cic_assets::Terrain;

/// Cells along one edge of a terrain chunk.
///
/// Thirty-two, so a 257x257 terrain divides into 8x8 chunks of 6144 vertices each. The trade is the usual
/// one: smaller chunks cull more tightly and cost more draw bookkeeping and more partially-filled chunks
/// at the terrain's edge, larger ones the reverse. This is also the granularity level of detail will
/// switch at, which argues against going much larger — a chunk is the smallest area that can change
/// density, and at 32 cells that is 256 world units at the default spacing.
pub const CHUNK_CELLS: u32 = 32;

/// A plane as `a * x + b * y + c * z + d`, positive on the inside.
type Plane = [f32; 4];

/// The six planes of a view frustum, all facing inward.
///
/// Extracted from a view-projection matrix rather than from a camera, so the same type serves the camera
/// and a shadow cascade's orthographic box without knowing which it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    planes: [Plane; 6],
}

impl Frustum {
    /// Extracts the six planes from a view-projection matrix.
    ///
    /// `matrix` is column-major — `matrix[column][row]` — matching [`crate::view`]. The planes come from
    /// sums and differences of its *rows*, because a point is inside when its clip coordinates satisfy
    /// `-w <= x <= w`, `-w <= y <= w` and `0 <= z <= w`; each of those six inequalities rearranges into one
    /// plane. The `z` pair is `0..=w` and not `-w..=w` because this project's projections map the near
    /// plane to zero, which [`crate::view`] has a test pinning — under the OpenGL convention the near
    /// plane would be `row3 + row2` and everything in front of the camera would be culled.
    #[must_use]
    pub fn from_view_projection(matrix: &[[f32; 4]; 4]) -> Self {
        let row = |index: usize| {
            [
                matrix[0][index],
                matrix[1][index],
                matrix[2][index],
                matrix[3][index],
            ]
        };
        let add = |left: Plane, right: Plane| {
            [
                left[0] + right[0],
                left[1] + right[1],
                left[2] + right[2],
                left[3] + right[3],
            ]
        };
        let subtract = |left: Plane, right: Plane| {
            [
                left[0] - right[0],
                left[1] - right[1],
                left[2] - right[2],
                left[3] - right[3],
            ]
        };
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        Self {
            planes: [
                add(w, x),
                subtract(w, x),
                add(w, y),
                subtract(w, y),
                z,
                subtract(w, z),
            ],
        }
    }

    /// Whether an axis-aligned box is at least partly inside.
    ///
    /// Conservative by construction: it tests each plane against the box corner furthest along that
    /// plane's normal, so a box outside *any* single plane is rejected and everything else is kept. That
    /// accepts a box in the region beyond two planes at once which is outside neither individually — a
    /// false positive near a frustum corner, which costs a chunk that need not have been drawn. The
    /// opposite error would remove terrain that should be visible, so the asymmetry is deliberate.
    ///
    /// A non-finite plane — from a singular matrix — rejects nothing, because a frustum nobody can
    /// evaluate should draw the world rather than delete it.
    #[must_use]
    pub fn intersects_box(&self, minimum: [f32; 3], maximum: [f32; 3]) -> bool {
        for plane in &self.planes {
            if !plane.iter().all(|value| value.is_finite()) {
                continue;
            }
            // The corner furthest along the normal: the maximum on each axis the normal points up.
            let furthest = [
                if plane[0] >= 0.0 {
                    maximum[0]
                } else {
                    minimum[0]
                },
                if plane[1] >= 0.0 {
                    maximum[1]
                } else {
                    minimum[1]
                },
                if plane[2] >= 0.0 {
                    maximum[2]
                } else {
                    minimum[2]
                },
            ];
            let distance =
                plane[0] * furthest[0] + plane[1] * furthest[1] + plane[2] * furthest[2] + plane[3];
            if distance < 0.0 {
                return false;
            }
        }
        true
    }
}

/// A terrain divided into fixed-size chunks, each with a world-space bounding box.
///
/// # Why the height bounds are global rather than per chunk
///
/// Every chunk's box spans the *whole* terrain's elevation range, not its own. Tighter per-chunk bounds
/// would cull more, and they would also go stale: heights live in a *writable* texture — that is the
/// design decision terrain deformation and road grading rest on — and `write_height_region` takes `&self`
/// and keeps no CPU copy, so nothing here could learn that a chunk had been raised. A box that is too
/// short is the failure that removes visible ground.
///
/// A shared range cannot go stale that way, and it costs little: what the camera looks at is bounded in Z
/// by the terrain itself, so the side planes do nearly all the work. It does inherit one existing
/// exposure — a write that pushes elevations *above* the range recorded at construction, which already
/// mis-sizes how far a shadow cascade reaches toward the light. That is one bug to fix rather than two.
#[derive(Debug, Clone)]
pub struct ChunkGrid {
    chunks_x: u32,
    chunks_y: u32,
    cells_x: u32,
    cells_y: u32,
    spacing: f32,
    lowest: f32,
    highest: f32,
}

impl ChunkGrid {
    /// Divides a terrain into chunks, taking its elevation range as every chunk's height extent.
    #[must_use]
    pub fn new(terrain: &Terrain) -> Self {
        let cells_x = terrain.width().saturating_sub(1).max(1);
        let cells_y = terrain.height().saturating_sub(1).max(1);
        let scale = terrain.vertical_scale();
        let (lowest, highest) = terrain
            .elevations()
            .iter()
            .fold((u16::MAX, u16::MIN), |(low, high), sample| {
                (low.min(*sample), high.max(*sample))
            });
        // An empty heightfield leaves the fold's identity in place, which would give an inverted range.
        let (lowest, highest) = if lowest > highest {
            (0, 0)
        } else {
            (lowest, highest)
        };
        Self {
            chunks_x: cells_x.div_ceil(CHUNK_CELLS),
            chunks_y: cells_y.div_ceil(CHUNK_CELLS),
            cells_x,
            cells_y,
            spacing: terrain.horizontal_scale(),
            lowest: f32::from(lowest) * scale,
            highest: f32::from(highest) * scale,
        }
    }

    /// Chunks along each axis.
    #[must_use]
    pub const fn counts(&self) -> (u32, u32) {
        (self.chunks_x, self.chunks_y)
    }

    /// How many chunks the terrain divides into.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.chunks_x * self.chunks_y
    }

    /// Whether the grid holds no chunks. Never true for a terrain that passed validation.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The world-space box of one chunk, or `None` when the index is out of range.
    ///
    /// The last chunk on each axis is clamped to the terrain's own extent rather than extending past it,
    /// so a terrain whose cell count is not a multiple of [`CHUNK_CELLS`] does not get a box covering
    /// ground that is not there — which would keep a chunk visible at the map edge for no reason.
    #[must_use]
    pub fn bounds(&self, chunk: u32) -> Option<([f32; 3], [f32; 3])> {
        if chunk >= self.len() {
            return None;
        }
        let (chunk_x, chunk_y) = (chunk % self.chunks_x, chunk / self.chunks_x);
        let start_x = chunk_x * CHUNK_CELLS;
        let start_y = chunk_y * CHUNK_CELLS;
        let end_x = (start_x + CHUNK_CELLS).min(self.cells_x);
        let end_y = (start_y + CHUNK_CELLS).min(self.cells_y);
        // Cell counts are bounded by the terrain's own validated dimensions, far inside exact f32 range.
        #[allow(clippy::cast_precision_loss)]
        let scaled = |cells: u32| cells as f32 * self.spacing;
        Some((
            [scaled(start_x), scaled(start_y), self.lowest],
            [scaled(end_x), scaled(end_y), self.highest],
        ))
    }

    /// Appends the chunks a frustum can see to `visible`, clearing it first.
    ///
    /// Takes the buffer rather than returning one, so a caller culling five frusta a frame reuses one
    /// allocation instead of making five.
    pub fn cull_into(&self, frustum: &Frustum, visible: &mut Vec<u32>) {
        visible.clear();
        for chunk in 0..self.len() {
            if let Some((minimum, maximum)) = self.bounds(chunk)
                && frustum.intersects_box(minimum, maximum)
            {
                visible.push(chunk);
            }
        }
    }
}

/// Collapses a sorted list of chunk indices into runs of adjacent ones, appending to `runs`.
///
/// Each run becomes one instanced draw instead of one per chunk. Worth the dozen lines because the input
/// is never scattered in practice: a camera sees a contiguous patch of map, so a cull of a few dozen
/// chunks usually collapses to a handful of runs — one per row of the patch.
///
/// Assumes ascending input, which is what [`ChunkGrid::cull_into`] produces by construction. Out-of-order
/// input is not misdrawn, merely split into more runs than necessary.
pub fn contiguous_runs(visible: &[u32], runs: &mut Vec<std::ops::Range<u32>>) {
    runs.clear();
    for chunk in visible {
        match runs.last_mut() {
            Some(run) if run.end == *chunk => run.end = chunk + 1,
            _ => runs.push(*chunk..chunk + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    // Every float compared here is an exact product of small integers and powers of two — a cell count
    // times a spacing of 4 or 8, or an elevation step times a vertical scale of 0.5 — so these are
    // assertions on values the arithmetic produces exactly rather than approximations of them. A chunk
    // boundary landing half a unit off is a real fault, not a rounding artifact to tolerate.
    #![allow(clippy::float_cmp)]

    use super::{CHUNK_CELLS, ChunkGrid, Frustum, contiguous_runs};
    use crate::view::{Projection, look_at, multiply, orthographic, perspective};
    use cic_assets::Terrain;

    /// A terrain of the given sample count, flat at one elevation.
    fn flat_terrain(samples: u32, spacing: f32, elevation: u16) -> Terrain {
        Terrain::new(
            samples,
            samples,
            spacing,
            0.5,
            vec![elevation; (samples * samples) as usize],
            Vec::new(),
        )
        .expect("valid terrain")
    }

    fn camera_frustum(eye: [f32; 3], focus: [f32; 3]) -> Frustum {
        let view = look_at(eye, focus, [0.0, 0.0, 1.0]);
        let projection = perspective(Projection::for_viewport(1280, 720));
        Frustum::from_view_projection(&multiply(projection, view))
    }

    #[test]
    fn a_box_in_front_of_the_camera_is_visible_and_one_behind_is_not() {
        // The single most important thing to get right, and the one an inverted near plane breaks: under
        // the OpenGL depth convention the near plane would be `row3 + row2` instead of `row2`, and
        // everything in front of the camera would be culled — a blank frame, not a subtle artifact.
        let frustum = camera_frustum([0.0, -100.0, 50.0], [0.0, 0.0, 0.0]);
        assert!(
            frustum.intersects_box([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]),
            "the box the camera is looking at must be visible"
        );
        assert!(
            !frustum.intersects_box([-10.0, -400.0, -10.0], [10.0, -300.0, 10.0]),
            "a box behind the camera must not be"
        );
    }

    #[test]
    fn a_box_beside_the_view_cone_is_rejected() {
        let frustum = camera_frustum([0.0, -100.0, 50.0], [0.0, 0.0, 0.0]);
        assert!(
            !frustum.intersects_box([5_000.0, -10.0, -10.0], [5_100.0, 10.0, 10.0]),
            "a box far off to the side must be rejected"
        );
        assert!(
            !frustum.intersects_box([-10.0, 5_000.0, -10.0], [10.0, 5_100.0, 10.0]),
            "a box far beyond the focus must be rejected"
        );
    }

    #[test]
    fn a_box_straddling_a_plane_is_kept() {
        // Conservative in the direction that matters: a chunk half inside must draw, or its visible half
        // becomes a hole.
        let frustum = camera_frustum([0.0, -100.0, 50.0], [0.0, 0.0, 0.0]);
        assert!(frustum.intersects_box([-10.0, -110.0, -10.0], [10.0, 10.0, 10.0]));
    }

    #[test]
    fn an_orthographic_cascade_box_culls_on_all_six_sides() {
        // The shadow path, where the frustum is a box rather than a cone. This is where culling pays most:
        // a near cascade covers a small patch of a large map and currently draws every triangle of it.
        let view = look_at([0.0, 0.0, 500.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let projection = orthographic(-100.0, 100.0, -100.0, 100.0, 1.0, 1_000.0);
        let frustum = Frustum::from_view_projection(&multiply(projection, view));
        assert!(frustum.intersects_box([-50.0, -50.0, -50.0], [50.0, 50.0, 50.0]));
        for offset in [[400.0, 0.0], [-400.0, 0.0], [0.0, 400.0], [0.0, -400.0]] {
            let minimum = [offset[0] - 50.0, offset[1] - 50.0, -50.0];
            let maximum = [offset[0] + 50.0, offset[1] + 50.0, 50.0];
            assert!(
                !frustum.intersects_box(minimum, maximum),
                "a box at {offset:?} is outside this cascade and must be culled"
            );
        }
    }

    #[test]
    fn a_singular_frustum_draws_everything_rather_than_nothing() {
        // A collapsed projection produces non-finite planes. Rejecting on those would empty the frame; the
        // renderer already reports a singular camera as an error, and this must not turn it into a blank
        // one first.
        let frustum = Frustum::from_view_projection(&[[f32::NAN; 4]; 4]);
        assert!(frustum.intersects_box([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
    }

    #[test]
    fn the_chunk_grid_covers_the_terrain_exactly() {
        // 257 samples is 256 cells, which divides into 8 chunks of 32.
        let grid = ChunkGrid::new(&flat_terrain(257, 8.0, 100));
        assert_eq!(grid.counts(), (8, 8));
        assert_eq!(grid.len(), 64);
        let (first_min, _) = grid.bounds(0).expect("first chunk");
        assert_eq!([first_min[0], first_min[1]], [0.0, 0.0]);
        // The far corner of the last chunk is the terrain's own extent: 256 cells at 8 units.
        let (_, last_max) = grid.bounds(63).expect("last chunk");
        assert_eq!([last_max[0], last_max[1]], [2_048.0, 2_048.0]);
    }

    #[test]
    fn a_partial_chunk_stops_at_the_terrain_edge() {
        // 100 samples is 99 cells: three chunks of 32 and a fourth holding three. A fourth chunk boxed as
        // though it were full would stay visible past the map edge, and under level of detail it would
        // also claim a density for ground that is not there.
        let grid = ChunkGrid::new(&flat_terrain(100, 4.0, 10));
        assert_eq!(grid.counts(), (4, 4));
        let (minimum, maximum) = grid.bounds(3).expect("last chunk of the first row");
        assert_eq!(CHUNK_CELLS, 32, "the figures below are worked from this");
        assert_eq!(
            minimum[0], 384.0,
            "three whole chunks of 32 cells at 4 units"
        );
        assert_eq!(maximum[0], 396.0, "the box must stop at the 99th cell");
    }

    #[test]
    fn every_chunk_box_spans_the_terrain_elevation_range() {
        // Global rather than per chunk, so a height write cannot leave a box too short. See the type note.
        let mut elevations = vec![200_u16; 64 * 64];
        elevations[0] = 40;
        elevations[1] = 900;
        let terrain =
            Terrain::new(64, 64, 8.0, 0.5, elevations, Vec::new()).expect("valid terrain");
        let grid = ChunkGrid::new(&terrain);
        for chunk in 0..grid.len() {
            let (minimum, maximum) = grid.bounds(chunk).expect("chunk in range");
            assert_eq!(minimum[2], 20.0, "40 steps at a 0.5 vertical scale");
            assert_eq!(maximum[2], 450.0, "900 steps at a 0.5 vertical scale");
        }
    }

    #[test]
    fn an_index_past_the_end_has_no_bounds() {
        let grid = ChunkGrid::new(&flat_terrain(64, 8.0, 100));
        assert!(grid.bounds(grid.len()).is_none());
        assert!(!grid.is_empty());
    }

    #[test]
    fn culling_keeps_the_chunks_under_the_camera_and_drops_the_rest() {
        // The claim the whole module exists for, stated as a proportion rather than a list: a camera
        // looking at one corner of a large terrain must not be drawing all of it, and must still be
        // drawing something.
        let terrain = flat_terrain(257, 8.0, 100);
        let grid = ChunkGrid::new(&terrain);
        let frustum = camera_frustum([100.0, -200.0, 300.0], [200.0, 200.0, 50.0]);
        let mut visible = Vec::new();
        grid.cull_into(&frustum, &mut visible);
        assert!(
            !visible.is_empty(),
            "the terrain under the camera must draw"
        );
        assert!(
            visible.len() < grid.len() as usize,
            "a camera looking at one corner drew all {} chunks",
            grid.len()
        );
        // Every index has to be addressable, since it is about to become an instance index on the GPU.
        for chunk in &visible {
            assert!(
                grid.bounds(*chunk).is_some(),
                "chunk {chunk} is out of range"
            );
        }
    }

    #[test]
    fn a_camera_above_the_middle_looking_down_sees_a_contiguous_patch() {
        // A sanity check on the *shape* of the answer, not just its size. A top-down camera over the
        // centre must keep chunks near the centre and reject the far corners; an answer that kept a corner
        // and dropped the middle would pass a proportion test.
        let terrain = flat_terrain(257, 8.0, 100);
        let grid = ChunkGrid::new(&terrain);
        let centre = 2_048.0 * 0.5;
        let frustum = camera_frustum([centre, centre - 1.0, 400.0], [centre, centre, 50.0]);
        let mut visible = Vec::new();
        grid.cull_into(&frustum, &mut visible);

        let (chunks_x, _) = grid.counts();
        let centre_chunk = (chunks_x / 2) + (chunks_x / 2) * chunks_x;
        assert!(
            visible.contains(&centre_chunk),
            "the chunk directly under the camera must be visible"
        );
        assert!(!visible.contains(&0), "the far corner must not be");
    }

    #[test]
    fn adjacent_chunks_collapse_into_one_run() {
        let mut runs = Vec::new();
        contiguous_runs(&[3, 4, 5, 9, 10, 20], &mut runs);
        assert_eq!(runs, vec![3..6, 9..11, 20..21]);
    }

    #[test]
    fn runs_cover_exactly_the_chunks_they_were_given() {
        // The property that matters: collapsing must not add or drop a chunk. Drawing one extra costs a
        // chunk of fill; dropping one leaves a hole in the terrain.
        let visible: Vec<u32> = vec![0, 1, 2, 7, 8, 15, 16, 17, 18, 40];
        let mut runs = Vec::new();
        contiguous_runs(&visible, &mut runs);
        let expanded: Vec<u32> = runs.iter().flat_map(std::ops::Range::clone).collect();
        assert_eq!(expanded, visible);
    }

    #[test]
    fn an_empty_cull_draws_nothing() {
        // A camera pointed at the sky. Zero runs means zero draw calls, which is the correct answer rather
        // than a degenerate one.
        // Seeded with last frame's contents, so this also checks they are cleared rather than kept.
        let mut runs = Vec::new();
        runs.push(0..99);
        contiguous_runs(&[], &mut runs);
        assert!(runs.is_empty());
    }

    #[test]
    fn a_whole_terrain_collapses_to_a_single_run() {
        // The common case at a strategic zoom, and the one that shows culling costs nothing when it finds
        // nothing to cull: 64 chunks become one instanced draw of 64.
        let grid = ChunkGrid::new(&flat_terrain(257, 8.0, 100));
        let visible: Vec<u32> = (0..grid.len()).collect();
        let mut runs = Vec::new();
        contiguous_runs(&visible, &mut runs);
        assert_eq!(runs, vec![0..64]);
    }

    #[test]
    fn culling_reuses_the_buffer_it_is_given() {
        let grid = ChunkGrid::new(&flat_terrain(65, 8.0, 100));
        let mut visible = vec![9_999; 40];
        let frustum = camera_frustum([0.0, -100.0, 100.0], [200.0, 200.0, 0.0]);
        grid.cull_into(&frustum, &mut visible);
        assert!(
            !visible.contains(&9_999),
            "the previous frame's contents must be cleared, not appended to"
        );
    }
}

// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Deterministic CPU residency bookkeeping for GPU-composed terrain pages.

use std::collections::BTreeSet;

use crate::terrain::TerrainDetailRequest;

pub(crate) const VIRTUAL_PAGE_INTERIOR: u32 = 256;
pub(crate) const VIRTUAL_PAGE_BORDER: u32 = 4;
pub(crate) const VIRTUAL_PAGE_EXTENT: u32 = VIRTUAL_PAGE_INTERIOR + 2 * VIRTUAL_PAGE_BORDER;
pub(crate) const VIRTUAL_PAGE_LAYERS: usize = 256;
pub(crate) const VIRTUAL_PAGE_MIPS: u32 = 9;

#[derive(Debug, Clone, Copy)]
pub(crate) struct VirtualPageView {
    position: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    terrain_minimum: [f32; 3],
    terrain_maximum: [f32; 3],
    tangent: f32,
    aspect: f32,
    cell_world_size: f32,
}

impl VirtualPageView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        position: [f32; 3],
        forward: [f32; 3],
        right: [f32; 3],
        up: [f32; 3],
        terrain_bounds: ([f32; 3], [f32; 3]),
        tangent: f32,
        aspect: f32,
        cell_world_size: f32,
    ) -> Self {
        Self {
            position,
            forward,
            right,
            up,
            terrain_minimum: terrain_bounds.0,
            terrain_maximum: terrain_bounds.1,
            tangent,
            aspect,
            cell_world_size,
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn project_page(self, key: VirtualPageKey) -> PageProjection {
        let cells_per_page = key.cells_per_page();
        let cell_minimum = [key.x * cells_per_page, key.y * cells_per_page];
        let cell_maximum = [
            cell_minimum[0].saturating_add(cells_per_page),
            cell_minimum[1].saturating_add(cells_per_page),
        ];
        let world_minimum = [
            self.terrain_minimum[0] + cell_minimum[0] as f32 * self.cell_world_size,
            self.terrain_minimum[1] + cell_minimum[1] as f32 * self.cell_world_size,
            self.terrain_minimum[2],
        ];
        let world_maximum = [
            (self.terrain_minimum[0] + cell_maximum[0] as f32 * self.cell_world_size)
                .min(self.terrain_maximum[0]),
            (self.terrain_minimum[1] + cell_maximum[1] as f32 * self.cell_world_size)
                .min(self.terrain_maximum[1]),
            self.terrain_maximum[2],
        ];
        let mut ndc_minimum = [f32::INFINITY; 2];
        let mut ndc_maximum = [f32::NEG_INFINITY; 2];
        let mut nearest_depth = f32::INFINITY;
        let mut front_corners = 0_u8;
        for x in [world_minimum[0], world_maximum[0]] {
            for y in [world_minimum[1], world_maximum[1]] {
                for z in [world_minimum[2], world_maximum[2]] {
                    let delta = [
                        x - self.position[0],
                        y - self.position[1],
                        z - self.position[2],
                    ];
                    let depth = dot(delta, self.forward);
                    if !depth.is_finite() || depth <= 1.0 {
                        continue;
                    }
                    front_corners = front_corners.saturating_add(1);
                    nearest_depth = nearest_depth.min(depth);
                    let ndc = [
                        dot(delta, self.right) / (depth * self.tangent * self.aspect),
                        dot(delta, self.up) / (depth * self.tangent),
                    ];
                    for axis in 0..2 {
                        ndc_minimum[axis] = ndc_minimum[axis].min(ndc[axis]);
                        ndc_maximum[axis] = ndc_maximum[axis].max(ndc[axis]);
                    }
                }
            }
        }
        let visible = front_corners != 0
            && ndc_minimum[0] <= 1.0
            && ndc_maximum[0] >= -1.0
            && ndc_minimum[1] <= 1.0
            && ndc_maximum[1] >= -1.0;
        let screen_offset = [
            if ndc_maximum[0] < 0.0 {
                -ndc_maximum[0]
            } else if ndc_minimum[0] > 0.0 {
                ndc_minimum[0]
            } else {
                0.0
            },
            if ndc_maximum[1] < 0.0 {
                -ndc_maximum[1]
            } else if ndc_minimum[1] > 0.0 {
                ndc_minimum[1]
            } else {
                0.0
            },
        ];
        PageProjection {
            visible,
            depth: if nearest_depth.is_finite() {
                (nearest_depth * 1_024.0).clamp(0.0, u32::MAX as f32) as u32
            } else {
                u32::MAX
            },
            screen_distance: ((screen_offset[0] * screen_offset[0]
                + screen_offset[1] * screen_offset[1])
                * 1_048_576.0)
                .clamp(0.0, u32::MAX as f32) as u32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PageProjection {
    visible: bool,
    depth: u32,
    screen_distance: u32,
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct VirtualPageKey {
    level: u8,
    x: u32,
    y: u32,
}

impl VirtualPageKey {
    const fn cells_per_page(self) -> u32 {
        if self.level == 0 { 8 } else { 16 }
    }

    const fn pixels_per_cell(self) -> u32 {
        if self.level == 0 { 32 } else { 16 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PhysicalPage {
    key: VirtualPageKey,
    last_used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VirtualPageJob {
    pub(crate) origin: [u32; 2],
    pub(crate) cells_per_page: u32,
    pub(crate) physical_layer: u32,
    pub(crate) pixels_per_cell: u32,
}

impl VirtualPageJob {
    pub(crate) fn write_bytes(self, bytes: &mut Vec<u8>) {
        for value in [
            self.origin[0],
            self.origin[1],
            self.cells_per_page,
            self.physical_layer,
            self.pixels_per_cell,
            0,
            0,
            0,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub(crate) struct VirtualPageUpdate {
    pub(crate) jobs: Vec<VirtualPageJob>,
    pub(crate) tables_changed: bool,
}

pub(crate) struct VirtualPageCache {
    table_sizes: [[u32; 2]; 2],
    tables: [Vec<u32>; 2],
    physical: Vec<Option<PhysicalPage>>,
    clock: u64,
}

impl VirtualPageCache {
    pub(crate) fn new(cell_size: [u32; 2]) -> Self {
        let table_sizes = [
            [cell_size[0].div_ceil(8), cell_size[1].div_ceil(8)],
            [cell_size[0].div_ceil(16), cell_size[1].div_ceil(16)],
        ];
        let table = |size: [u32; 2]| vec![0; size[0] as usize * size[1] as usize];
        Self {
            table_sizes,
            tables: [table(table_sizes[0]), table(table_sizes[1])],
            physical: vec![None; VIRTUAL_PAGE_LAYERS],
            clock: 0,
        }
    }

    pub(crate) const fn table_size(&self, level: usize) -> [u32; 2] {
        self.table_sizes[level]
    }

    pub(crate) fn table(&self, level: usize) -> &[u32] {
        &self.tables[level]
    }

    pub(crate) fn update(
        &mut self,
        requests: &[TerrainDetailRequest],
        view: VirtualPageView,
    ) -> VirtualPageUpdate {
        self.clock = self.clock.saturating_add(1);
        let mut ranked = Vec::new();
        for request in requests {
            let level = match request.density() {
                32 => 0_u8,
                16 => 1_u8,
                _ => continue,
            };
            let cells_per_page = if level == 0 { 8 } else { 16 };
            let minimum = request.minimum();
            let maximum = request.maximum();
            let visible_minimum = request.visible_minimum();
            let visible_maximum = request.visible_maximum();
            let start = [minimum[0] / cells_per_page, minimum[1] / cells_per_page];
            let end = [
                maximum[0].div_ceil(cells_per_page),
                maximum[1].div_ceil(cells_per_page),
            ];
            let table_size = self.table_sizes[usize::from(level)];
            for y in start[1]..end[1].min(table_size[1]) {
                for x in start[0]..end[0].min(table_size[0]) {
                    let key = VirtualPageKey { level, x, y };
                    let page_minimum = [x * cells_per_page, y * cells_per_page];
                    let page_maximum = [
                        page_minimum[0].saturating_add(cells_per_page),
                        page_minimum[1].saturating_add(cells_per_page),
                    ];
                    let intersects_visible = page_minimum[0] < visible_maximum[0]
                        && page_maximum[0] > visible_minimum[0]
                        && page_minimum[1] < visible_maximum[1]
                        && page_maximum[1] > visible_minimum[1];
                    let projection = view.project_page(key);
                    let group = match (projection.visible, intersects_visible, level) {
                        (true, true, 1) => 0_u8,
                        (true, true, 0) => 1,
                        (true, false, 1) => 2,
                        (true, false, 0) => 3,
                        (false, true, 1) => 4,
                        (false, true, 0) => 5,
                        (false, false, 1) => 6,
                        (false, false, 0) => 7,
                        _ => unreachable!("virtual terrain has two page levels"),
                    };
                    ranked.push((group, projection.depth, projection.screen_distance, key));
                }
            }
        }
        ranked.sort_unstable();
        ranked.dedup_by_key(|entry| entry.3);
        ranked.truncate(VIRTUAL_PAGE_LAYERS);
        let desired = ranked.iter().map(|entry| entry.3).collect::<BTreeSet<_>>();

        for page in self.physical.iter_mut().flatten() {
            if desired.contains(&page.key) {
                page.last_used = self.clock;
            }
        }

        let mut jobs = Vec::new();
        let mut tables_changed = false;
        for (_, _, _, key) in ranked {
            if self.physical.iter().flatten().any(|page| page.key == key) {
                continue;
            }
            let slot = self.physical.iter().position(Option::is_none).or_else(|| {
                self.physical
                    .iter()
                    .enumerate()
                    .filter_map(|(index, page)| {
                        page.filter(|page| !desired.contains(&page.key))
                            .map(|page| (page.last_used, index))
                    })
                    .min()
                    .map(|(_, index)| index)
            });
            let Some(slot) = slot else {
                break;
            };
            if let Some(previous) = self.physical[slot] {
                self.set_table(previous.key, 0);
            }
            self.physical[slot] = Some(PhysicalPage {
                key,
                last_used: self.clock,
            });
            self.set_table(key, u32::try_from(slot).unwrap_or(u32::MAX) + 1);
            tables_changed = true;
            let cells_per_page = key.cells_per_page();
            jobs.push(VirtualPageJob {
                origin: [key.x * cells_per_page, key.y * cells_per_page],
                cells_per_page,
                physical_layer: u32::try_from(slot).unwrap_or(u32::MAX),
                pixels_per_cell: key.pixels_per_cell(),
            });
        }
        VirtualPageUpdate {
            jobs,
            tables_changed,
        }
    }

    fn set_table(&mut self, key: VirtualPageKey, value: u32) {
        let level = usize::from(key.level);
        let width = self.table_sizes[level][0];
        let index = key.y as usize * width as usize + key.x as usize;
        if let Some(entry) = self.tables[level].get_mut(index) {
            *entry = value;
        }
    }

    #[cfg(test)]
    fn resident_count(&self) -> usize {
        self.physical.iter().flatten().count()
    }
}

#[cfg(test)]
mod tests {
    use super::{VirtualPageCache, VirtualPageKey, VirtualPageView};
    use crate::terrain::TerrainDetailRequest;

    fn request(min: [u32; 2], max: [u32; 2], density: u32) -> TerrainDetailRequest {
        TerrainDetailRequest::for_test(min, max, density)
    }

    fn test_view(position: [f32; 3], forward: [f32; 3]) -> VirtualPageView {
        let right = [1.0, 0.0, 0.0];
        let up = [0.0, -forward[2], forward[1]];
        VirtualPageView::new(
            position,
            forward,
            right,
            up,
            ([0.0, 0.0, 0.0], [2_560.0, 2_560.0, 100.0]),
            (std::f32::consts::PI / 6.0).tan(),
            16.0 / 9.0,
            10.0,
        )
    }

    #[test]
    fn resident_pages_are_reused_without_new_jobs() {
        let mut cache = VirtualPageCache::new([128, 128]);
        let requests = [request([0, 0], [16, 16], 32)];
        let view = test_view([80.0, -100.0, 100.0], [0.0, 0.8, -0.6]);
        let first = cache.update(&requests, view);
        assert_eq!(first.jobs.len(), 4);
        let second = cache.update(&requests, view);
        assert!(second.jobs.is_empty());
        assert!(!second.tables_changed);
        assert_eq!(cache.resident_count(), 4);
    }

    #[test]
    fn cache_is_bounded_and_keeps_valid_page_table_entries() {
        let mut cache = VirtualPageCache::new([512, 512]);
        let update = cache.update(
            &[request([0, 0], [512, 512], 32)],
            test_view([1_280.0, -100.0, 300.0], [0.0, 0.8, -0.6]),
        );
        assert_eq!(update.jobs.len(), super::VIRTUAL_PAGE_LAYERS);
        assert_eq!(cache.resident_count(), super::VIRTUAL_PAGE_LAYERS);
        assert_eq!(
            cache.table(0).iter().filter(|entry| **entry != 0).count(),
            super::VIRTUAL_PAGE_LAYERS
        );
    }

    #[test]
    fn projected_page_visibility_follows_the_camera_angle() {
        let view = test_view([640.0, -100.0, 200.0], [0.0, 0.8, -0.6]);
        let centered = view.project_page(VirtualPageKey {
            level: 0,
            x: 8,
            y: 4,
        });
        let off_axis = view.project_page(VirtualPageKey {
            level: 0,
            x: 24,
            y: 4,
        });
        assert!(centered.visible);
        assert!(!off_axis.visible);
    }

    #[test]
    fn visible_coarse_coverage_precedes_fine_page_upgrades() {
        let mut cache = VirtualPageCache::new([128, 128]);
        let update = cache.update(
            &[
                request([0, 0], [128, 128], 16),
                request([0, 0], [128, 128], 32),
            ],
            test_view([640.0, -100.0, 200.0], [0.0, 0.8, -0.6]),
        );
        assert_eq!(update.jobs.first().map(|job| job.pixels_per_cell), Some(16));
        assert!(update.jobs.iter().any(|job| job.pixels_per_cell == 32));
    }
}

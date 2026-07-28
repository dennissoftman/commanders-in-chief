//! Water surfaces: a bounded plane with procedural waves, shaded transmissively.
//!
//! # Where water lives, and why it is not in the terrain container
//!
//! A water body is renderer state, supplied per frame the way [`crate::model::ModelBatch`] supplies
//! its instances, and a `.cict` says nothing about it. That follows the split the terrain format
//! already draws: the container carries *where* something is and *how much* of it there is, and the
//! renderer resolves what it looks like. Tint, wave scale, and the depth over which shallow becomes
//! deep are all things an artist adjusts without touching a map, so putting any of them in the
//! heightfield would mean a format version bump for each, and would force every tool that reads a
//! heightfield to understand a surface it has no use for.
//!
//! # Provenance
//!
//! Every constant here, and in the water half of `terrain_deferred.wgsl`, was authored in this
//! module. The predecessor's standing-water shader took its texture scale, tint, alpha, and
//! depth-feather policy from another game's code; that file was deleted rather than carried across,
//! and it must not be consulted to write this one. The values below come from the reasoning stated
//! beside them. See `LICENSING.md`.
//!
//! # The wave model
//!
//! Five directional waves summed, related the way deep-water gravity waves are: phase speed goes as
//! the square root of wavelength, so the long swell outruns the short chop instead of the whole sum
//! sliding rigidly across the map. Amplitude scales with wavelength, which holds steepness roughly
//! constant across the five, and the amplitudes are normalised so `wave_height` is the crest height it
//! claims to be.
//!
//! Two choices exist purely to stop the sum reading as a tiled texture, and both were arrived at by
//! looking at the capture rather than by reasoning ahead:
//!
//! - **Wavelengths step by about 0.61, not by halves.** Near-harmonic ratios reinforce at regular
//!   intervals, and that interference is the visible lattice. The first attempt used 1, ½, ⅓, ⅙ and
//!   produced an unmistakable diamond grid.
//! - **Directions advance by the golden angle.** Any rational fraction of a turn eventually puts two
//!   waves on one axis, and a shared axis is what a lattice is built from.
//!
//! Normals are analytic — the derivative of that same sum — rather than sampled from a normal map or
//! taken by finite difference. That costs five cosines the height already needed and buys a normal
//! exact at any tessellation, which matters because the grid density is chosen from the wavelength
//! rather than fixed.

use crate::RenderError;
use crate::deferred::{buffer_entry, push_vec4, uniform_buffer, uniform_entry};

/// Bytes in the `Water` uniform block: five `vec4`s.
const WATER_UNIFORM_BYTES: usize = 5 * 16;

/// Grid cells spanning one dominant wavelength.
///
/// Six, because the geometric displacement only has to be smooth enough that the silhouette against
/// the shore reads as a wave. The *shading* normal is analytic and exact per fragment however coarse
/// the grid is, so tessellation buys shape, not lighting.
const CELLS_PER_WAVELENGTH: f32 = 6.0;

/// Most cells along one axis of a water grid.
///
/// A cost bound rather than a quality one. The grid covers the body's whole bounding rectangle, so
/// every cell over dry land is a vertex that the fragment stage then discards; a real map wants the
/// grid clipped to the water itself, which is follow-up work rather than something this ceiling
/// fixes. At 128 a body costs about 98,000 vertices per frame.
const MAX_CELLS: u32 = 128;

/// How a water surface looks.
///
/// Split from [`WaterSurface`]'s geometry so several bodies on one map can share an appearance, the
/// way terrain layers share a palette entry.
#[derive(Debug, Clone, Copy)]
pub struct WaterMaterial {
    /// Transmitted colour where the water is shallow.
    pub shallow: [f32; 3],
    /// Transmitted colour where it is deep.
    pub deep: [f32; 3],
    /// World units of depth over which `shallow` reaches `deep`.
    pub depth_scale: f32,
    /// World units of depth over which the shoreline reaches full opacity.
    pub edge_feather: f32,
    /// Amplitude of the dominant wave, in world units.
    pub wave_height: f32,
    /// Wavelength of the dominant wave, in world units.
    pub wave_length: f32,
    /// Travel speed of the dominant wave, in world units per second.
    pub wave_speed: f32,
    /// Surface roughness in `0..=1`, which sets how broad the sun's glitter is.
    pub roughness: f32,
}

impl Default for WaterMaterial {
    /// An inland lake at the scale this renderer's maps use.
    ///
    /// The tints are a desaturated green-blue shading to near-black rather than a saturated blue:
    /// fresh water takes most of its apparent colour from what is suspended in it and from the sky it
    /// reflects, and a strongly blue body colour double-counts the reflection the shader already adds.
    /// `depth_scale` is deliberately short — a dozen units — because the ramp exists to read the
    /// *shore*, and a ramp measured in hundreds of units leaves every playable lake uniformly pale.
    fn default() -> Self {
        Self {
            shallow: [0.16, 0.34, 0.36],
            deep: [0.02, 0.07, 0.13],
            depth_scale: 12.0,
            // Shorter than `depth_scale`: opacity should arrive well before the tint finishes
            // ramping, or the shallows read as fog over the bed instead of as clear water above it.
            edge_feather: 3.0,
            wave_height: 0.35,
            wave_length: 24.0,
            wave_speed: 3.0,
            // Low, so the glitter is a tight highlight. Water is among the smoothest surfaces a map
            // contains, and a broad highlight on it reads as wet plastic.
            roughness: 0.06,
        }
    }
}

/// One body of water: a rectangle of the map, a surface elevation, and a material.
///
/// Axis-aligned rather than a polygon. A rectangle plus the shader's own depth test against the
/// terrain already produces an arbitrarily shaped shoreline, because the water is clipped wherever
/// the bed rises through it — so the authored shape only has to *contain* the water, not describe it.
#[derive(Debug, Clone, Copy)]
pub struct WaterSurface {
    /// World-space extent as `[min_x, min_y, max_x, max_y]`.
    pub bounds: [f32; 4],
    /// Mean surface elevation in world units. Waves displace about this.
    pub elevation: f32,
    /// How the surface looks.
    pub material: WaterMaterial,
}

impl WaterSurface {
    /// A body covering `bounds` at `elevation`, with the default material.
    #[must_use]
    pub fn new(bounds: [f32; 4], elevation: f32) -> Self {
        Self {
            bounds,
            elevation,
            material: WaterMaterial::default(),
        }
    }

    /// Replaces the material, for chaining onto [`Self::new`].
    #[must_use]
    pub const fn with_material(mut self, material: WaterMaterial) -> Self {
        self.material = material;
        self
    }

    /// Returns the extent this body spans as `[x, y]`.
    #[must_use]
    pub fn span(&self) -> [f32; 2] {
        [
            self.bounds[2] - self.bounds[0],
            self.bounds[3] - self.bounds[1],
        ]
    }

    /// Checks every figure the shader divides by or indexes with.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidWater`] for a non-finite value, an empty or inverted rectangle,
    /// a non-positive depth ramp, feather, or wavelength, or a roughness outside `0..=1`.
    pub fn validate(&self) -> Result<(), RenderError> {
        let material = &self.material;
        let finite = self.bounds.iter().all(|value| value.is_finite())
            && self.elevation.is_finite()
            && material.shallow.iter().all(|value| value.is_finite())
            && material.deep.iter().all(|value| value.is_finite())
            && material.wave_height.is_finite()
            && material.wave_speed.is_finite();
        if !finite {
            return Err(RenderError::InvalidWater);
        }
        let [span_x, span_y] = self.span();
        // Positive, not just non-negative: a zero-area body would divide by its own span when the
        // vertex shader maps the grid across it.
        if span_x <= 0.0 || span_y <= 0.0 {
            return Err(RenderError::InvalidWater);
        }
        if material.depth_scale <= 0.0
            || material.edge_feather <= 0.0
            || material.wave_length <= 0.0
            || material.wave_height < 0.0
        {
            return Err(RenderError::InvalidWater);
        }
        if !(0.0..=1.0).contains(&material.roughness) {
            return Err(RenderError::InvalidWater);
        }
        Ok(())
    }

    /// Grid cells along each axis, sized so the dominant wavelength spans
    /// [`CELLS_PER_WAVELENGTH`] of them.
    ///
    /// Derived rather than authored, so a body with a long swell does not pay for a grid fine enough
    /// for chop, and one with short chop is not tessellated too coarsely to show it.
    #[must_use]
    pub fn grid_cells(&self) -> [u32; 2] {
        let cell = self.material.wave_length / CELLS_PER_WAVELENGTH;
        self.span().map(|span| {
            if cell <= 0.0 || !cell.is_finite() {
                return 1;
            }
            // `span / cell` is finite and non-negative here, so the saturating cast lands in range;
            // the clamp is what bounds it, not the cast.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cells = (span / cell).ceil() as u32;
            cells.clamp(1, MAX_CELLS)
        })
    }

    /// Vertices the grid draws: six per cell, as an unindexed triangle list.
    #[must_use]
    pub fn vertex_count(&self) -> u32 {
        let [cells_x, cells_y] = self.grid_cells();
        // Bounded by `MAX_CELLS` squared times six, which is far inside `u32`.
        cells_x * cells_y * 6
    }
}

/// One water body uploaded to the GPU.
#[derive(Debug)]
pub struct WaterBody {
    uniform: wgpu::Buffer,
    group: wgpu::BindGroup,
    surface: WaterSurface,
    vertex_count: u32,
}

impl WaterBody {
    /// Returns the bind group layout a body's uniform is bound through.
    ///
    /// Built once by the renderer and shared, so the pipeline and every body's bind group are created
    /// against the same layout and cannot drift apart — the same arrangement
    /// [`crate::model::ModelBatch`] uses for its materials.
    #[must_use]
    pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render water layout"),
            // Visible to both stages: the vertex shader displaces by the wave sum and the fragment
            // shader shades by the same one, so both read this block.
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        })
    }

    /// Uploads a body and its material.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidWater`] when the surface does not [validate](WaterSurface::validate).
    pub fn new(
        context: &crate::GpuContext,
        surface: WaterSurface,
        layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, RenderError> {
        surface.validate()?;
        let device = context.device();
        let uniform = uniform_buffer(device, "cic-render water", WATER_UNIFORM_BYTES);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render water bindings"),
            layout,
            entries: &[buffer_entry(0, &uniform)],
        });
        let body = Self {
            uniform,
            group,
            surface,
            vertex_count: surface.vertex_count(),
        };
        // Uploaded at construction as well as per frame, so a body drawn before any `set_frame` shows
        // a still surface rather than reading an uninitialised buffer.
        body.set_time(context, 0.0);
        Ok(body)
    }

    /// Uploads the block with `time` as the wave phase, in seconds.
    ///
    /// Time is a parameter rather than a clock reading. That is what lets a headless capture pin it:
    /// an animated surface sampled from the wall clock would make every reference image
    /// irreproducible, and the visual regression harness depends on the opposite.
    pub fn set_time(&self, context: &crate::GpuContext, time: f32) {
        let material = &self.surface.material;
        let [cells_x, cells_y] = self.surface.grid_cells();
        // Cell counts are bounded by `MAX_CELLS`, so both convert exactly.
        #[allow(clippy::cast_precision_loss)]
        let (grid_x, grid_y) = (cells_x as f32, cells_y as f32);

        let mut bytes = Vec::with_capacity(WATER_UNIFORM_BYTES);
        push_vec4(&mut bytes, self.surface.bounds);
        push_vec4(
            &mut bytes,
            [
                self.surface.elevation,
                if time.is_finite() { time } else { 0.0 },
                grid_x,
                grid_y,
            ],
        );
        let [shallow_r, shallow_g, shallow_b] = material.shallow;
        push_vec4(
            &mut bytes,
            [shallow_r, shallow_g, shallow_b, material.depth_scale],
        );
        let [deep_r, deep_g, deep_b] = material.deep;
        push_vec4(&mut bytes, [deep_r, deep_g, deep_b, material.edge_feather]);
        push_vec4(
            &mut bytes,
            [
                material.wave_height,
                material.wave_length,
                material.wave_speed,
                material.roughness,
            ],
        );
        debug_assert_eq!(bytes.len(), WATER_UNIFORM_BYTES, "water uniform drifted");
        context.queue().write_buffer(&self.uniform, 0, &bytes);
    }

    /// Returns the surface this body was built from.
    #[must_use]
    pub const fn surface(&self) -> &WaterSurface {
        &self.surface
    }

    /// Returns how many vertices the grid draws.
    #[must_use]
    pub const fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    /// Records a draw. The caller has already set the pipeline and the lighting bind group.
    ///
    /// Group 2, not 1: the shader shares a module with the composite, which owns group 1, and one
    /// module cannot bind two different resources to the same slot.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(2, &self.group, &[]);
        pass.draw(0..self.vertex_count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_CELLS, WATER_UNIFORM_BYTES, WaterMaterial, WaterSurface};
    use crate::RenderError;

    fn surface() -> WaterSurface {
        WaterSurface::new([0.0, 0.0, 240.0, 240.0], 5.0)
    }

    #[test]
    fn the_uniform_block_matches_the_shader_declaration() {
        // Five `vec4`s in the WGSL `Water` struct. A mismatch does not fail validation — it silently
        // misaligns every field past the drift — so it is asserted here as well as at upload.
        assert_eq!(WATER_UNIFORM_BYTES, 80);
    }

    #[test]
    fn a_valid_body_is_accepted() {
        surface().validate().expect("a plain lake should validate");
    }

    #[test]
    fn an_empty_or_inverted_rectangle_is_refused() {
        // Both would divide by their own span when the vertex shader maps the grid across them.
        for bounds in [
            [0.0, 0.0, 0.0, 100.0],
            [0.0, 0.0, 100.0, 0.0],
            [100.0, 0.0, 0.0, 100.0],
        ] {
            assert!(
                matches!(
                    WaterSurface::new(bounds, 0.0).validate(),
                    Err(RenderError::InvalidWater)
                ),
                "{bounds:?} should be refused"
            );
        }
    }

    #[test]
    fn non_finite_and_out_of_range_material_values_are_refused() {
        let cases = [
            WaterMaterial {
                depth_scale: 0.0,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                edge_feather: -1.0,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                wave_length: 0.0,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                roughness: 1.5,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                wave_height: f32::NAN,
                ..WaterMaterial::default()
            },
        ];
        for material in cases {
            assert!(
                matches!(
                    surface().with_material(material).validate(),
                    Err(RenderError::InvalidWater)
                ),
                "{material:?} should be refused"
            );
        }
    }

    #[test]
    fn the_grid_is_sized_from_the_wavelength_not_the_body() {
        // The property that matters: halving the wavelength doubles the tessellation, so a body with
        // short chop is not drawn too coarsely to show it.
        let coarse = surface().with_material(WaterMaterial {
            wave_length: 48.0,
            ..WaterMaterial::default()
        });
        let fine = surface().with_material(WaterMaterial {
            wave_length: 24.0,
            ..WaterMaterial::default()
        });
        let [coarse_x, _] = coarse.grid_cells();
        let [fine_x, _] = fine.grid_cells();
        assert_eq!(fine_x, coarse_x * 2, "{fine_x} against {coarse_x}");
    }

    #[test]
    fn the_grid_is_bounded_however_large_the_body() {
        // A map-sized body with fine chop would otherwise ask for millions of vertices.
        let huge =
            WaterSurface::new([0.0, 0.0, 100_000.0, 100_000.0], 0.0).with_material(WaterMaterial {
                wave_length: 0.5,
                ..WaterMaterial::default()
            });
        assert_eq!(huge.grid_cells(), [MAX_CELLS, MAX_CELLS]);
        assert_eq!(huge.vertex_count(), MAX_CELLS * MAX_CELLS * 6);
    }

    #[test]
    fn a_body_narrower_than_one_cell_still_draws_a_quad() {
        // Clamped to one cell rather than zero, which would draw nothing and read as missing water.
        let sliver = WaterSurface::new([0.0, 0.0, 1.0, 1.0], 0.0);
        assert_eq!(sliver.grid_cells(), [1, 1]);
        assert_eq!(sliver.vertex_count(), 6);
    }
}

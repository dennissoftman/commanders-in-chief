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
//! See [ADR 4002](../../../docs/adr/4002-water-kinds.md) for why the wave train has the shape it has
//! and why the shading normal is damped by the pixel footprint, and
//! [ADR 4003](../../../docs/adr/4003-reflection-providers.md) for why absorption decides opacity and
//! what a reflection provider is.
//!
//! # Provenance
//!
//! Every constant here, and in `water.wgsl`, was authored in this
//! module. The predecessor's standing-water shader took its texture scale, tint, alpha, and
//! depth-feather policy from another game's code; that file was deleted rather than carried across,
//! and it must not be consulted to write this one. The values below come from the reasoning stated
//! beside them. See `LICENSING.md`.
//!
//! Three published techniques are reimplemented here and named where they are used: the second-order
//! Stokes wave (Stokes, 1847) for the peaked crests, the golden-ratio low-discrepancy sequence
//! (Roberts' generalisation of the Kronecker sequence) for the direction and phase spreads, and
//! normal-map filtering (Toksvig, 2005; Olano and Baker, LEAN mapping, 2010) for the reasoning behind
//! damping a normal a pixel cannot resolve. None of them came from another game's source.
//!
//! # The three kinds
//!
//! [`WaterKind`] names the three bodies a map has: a [lake](WaterKind::Lake), a
//! [river](WaterKind::River) and an [ocean](WaterKind::Ocean). Each resolves to a whole
//! [`WaterMaterial`] rather than to a flag the shader branches on, so a map can name a kind and an
//! artist can still move any one figure off the preset without the two disagreeing about what a river
//! is. What actually separates them, in descending order of how much it shows at playing distance:
//!
//! - **Tint and depth ramp.** A river is silt over a bed a metre or two down and its ramp is measured
//!   in single units; an ocean is near-black past a sandbar and its ramp is measured in tens.
//! - **Directional spread.** A river's train is collimated down the channel, an ocean's fans out about
//!   the wind, and a lake's is isotropic. This is the figure that makes a river look like moving water
//!   from directly above, where nothing else does.
//! - **Foam.** Surf at the shore is most of what reads as an ocean, and whitecaps breaking on the
//!   tallest crests are most of what stops open water reading as a smooth gradient. A lake gets a
//!   faint lap of the first and none of the second.
//! - **Wave scale, current and crest peaking.**
//!
//! A river is one body per straight reach. A channel that bends is a chain of them, each with the flow
//! of its own tangent — which is what a spline-authored river resolves to, and why the flow is a vector
//! rather than a heading and a speed.
//!
//! # The wave model
//!
//! Twelve directional waves summed, related the way deep-water gravity waves are: phase speed goes as
//! the square root of wavelength, so the long swell outruns the short chop instead of the whole sum
//! sliding rigidly across the map. Amplitude scales with wavelength, which holds steepness constant
//! across the twelve, and the amplitudes are normalised so `wave_height` is the crest height it claims
//! to be. Each is a second-order Stokes wave rather than a sine, so crests sharpen and troughs flatten
//! as the peaking rises.
//!
//! Four choices exist purely to stop the sum reading as manufactured, and all four were arrived at by
//! looking at the capture rather than by reasoning ahead:
//!
//! - **Twelve components, not five.** Five have a short enough beat to band a still lake diagonally
//!   across its whole width, which the reference capture showed plainly. The last three of the twelve
//!   are the fine ripple a near camera resolves, and are why no detail normal map is wanted: the same
//!   detail, from the same generator, with nothing to author and no tiling to hide.
//! - **Wavelengths in irrational powers of the golden ratio.** Near-harmonic ratios reinforce at
//!   regular intervals, and that interference is the visible lattice. The first attempt used 1, ½, ⅓, ⅙
//!   and produced an unmistakable diamond grid; the second stepped by a flat 0.61, which left the pair
//!   1 and 0.372 near enough to 8:3 to beat across a map.
//! - **Directions spread by a low-discrepancy fraction of the arc, not by a fixed step around it.** Any
//!   fixed step eventually puts two waves on one axis, and a shared axis is what a lattice is built
//!   from.
//! - **Every component carries a group envelope**, running mostly across its own travel, so a crest
//!   has a finite length and fades in and out along its ridge. This is the one that separates "water"
//!   from "corduroy", and no number of components substitutes for it: a sum of infinite plane waves is
//!   exactly as strong everywhere however many there are, and real water arrives in sets.
//!
//! Normals are analytic — the derivative of that same sum, envelope included by the product rule —
//! rather than sampled from a normal map or taken by finite difference. That buys a normal exact at any
//! tessellation, which matters because the grid density is chosen from the wavelength rather than
//! fixed. The fragment stage damps each component of that derivative by how much of it one pixel
//! covers; see `wave_detail` in `water.wgsl` for why an undamped analytic normal makes a distant
//! surface sparkle.

use crate::RenderError;
use crate::deferred::{buffer_entry, push_vec4, uniform_buffer, uniform_entry};

/// Bytes in the `Water` uniform block: eight `vec4`s.
const WATER_UNIFORM_BYTES: usize = 8 * 16;

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

/// Which of the three bodies a surface is.
///
/// A selector rather than stored state: it resolves to a whole [`WaterMaterial`], and nothing
/// downstream branches on it. That is deliberate. A kind carried alongside the numbers would be a
/// second source of truth that the numbers could drift away from — a `River` whose spread had been
/// widened to a lake's is not a river, and no assertion could tell which of the two fields was meant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaterKind {
    /// Still inland water. Waves run in every direction, and the surface goes nowhere.
    Lake,
    /// A channel with a current, in world units per second along the flow.
    ///
    /// The current carries the whole surface, not only the waves: chop travels *over* water that is
    /// itself moving, and the two speeds are what make a river read as flowing rather than as a lake
    /// with a texture scrolling over it.
    River {
        /// Velocity of the water, in world units per second. Its direction is the channel's heading,
        /// so a spline-authored river is one body per reach carrying that reach's tangent.
        flow: [f32; 2],
    },
    /// Open sea: a long swell with steep crests, near-black in deep water, and surf where it shoals.
    Ocean {
        /// Which way the swell runs. Only the direction is used.
        wind: [f32; 2],
    },
}

impl WaterKind {
    /// The material this kind resolves to.
    #[must_use]
    pub fn material(self) -> WaterMaterial {
        match self {
            Self::Lake => WaterMaterial::lake(),
            Self::River { flow } => WaterMaterial::river(flow),
            Self::Ocean { wind } => WaterMaterial::ocean(wind),
        }
    }
}

/// How a water surface looks.
///
/// Split from [`WaterSurface`]'s geometry so several bodies on one map can share an appearance, the
/// way terrain layers share a palette entry. Build one from a [`WaterKind`] and adjust from there;
/// every field is public because the presets are a starting point rather than a policy.
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
    /// Which way the train runs. Normalised at upload; a zero vector falls back to `+x`.
    pub heading: [f32; 2],
    /// Half-angle the component directions occupy about `heading`, in radians.
    ///
    /// [`std::f32::consts::PI`] is isotropic and near zero is collimated. This is the figure that
    /// distinguishes the three kinds most cheaply.
    pub spread: f32,
    /// How far the crests are peaked, in `0..=1`.
    ///
    /// Zero is a plain sine. One is the steepest second-order Stokes wave that still has a flat
    /// trough rather than a dimpled one.
    pub peaking: f32,
    /// Velocity the whole surface is carried at, in world units per second.
    pub current: [f32; 2],
    /// World units of depth over which shore foam fades out.
    pub foam_depth: f32,
    /// How much foam there is at the waterline, in `0..=1`.
    pub foam_strength: f32,
    /// How much whitecap breaks on the tallest crests in open water, in `0..=1`.
    ///
    /// Separate from [`Self::foam_strength`] rather than a fraction of it: a lake laps at its edge
    /// and never breaks in the middle, and an ocean does both.
    pub whitecap: f32,
    /// How far a wave face displaces the bed seen through it, in world units at full depth.
    ///
    /// This is refraction, and it is what makes a stone under a ripple wobble. Scaled down as the
    /// water shallows, so a bed a hand's breadth under the surface is not slid out from under its own
    /// shoreline. Zero renders the bed undisplaced, which is what every frame before this did.
    pub refraction: f32,
}

impl WaterMaterial {
    /// An inland lake at the scale this renderer's maps use.
    ///
    /// The tints are a desaturated green-blue shading to near-black rather than a saturated blue:
    /// fresh water takes most of its apparent colour from what is suspended in it and from the sky it
    /// reflects, and a strongly blue body colour double-counts the reflection the shader already adds.
    /// `depth_scale` is deliberately short — a dozen units — because the ramp exists to read the
    /// *shore*, and a ramp measured in hundreds of units leaves every playable lake uniformly pale.
    #[must_use]
    pub fn lake() -> Self {
        Self {
            shallow: [0.16, 0.34, 0.36],
            // Not quite the near-black it was. A lake is shallow enough that its bed still returns
            // something from the middle, and a deep tint that bottoms out leaves the whole body one
            // dead navy with nothing in it to catch a wave.
            deep: [0.03, 0.09, 0.15],
            // Longer than it was, so the ramp is still doing something out in the middle rather than
            // reaching its far end a few units from the bank and leaving the rest flat.
            depth_scale: 6.0,
            // Shorter than `depth_scale`: opacity should arrive well before the tint finishes
            // ramping, or the shallows read as fog over the bed instead of as clear water above it.
            edge_feather: 3.0,
            // Small and short and slow, because a lake is *almost still*. The previous single set of
            // figures was a compromise across all three kinds and gave every pond an ocean's swell.
            // Almost still is not glassy, though: at half this steepness the surface came out as a
            // sheet of coloured plastic, because the only variation a lake has to offer is the sky
            // its wave faces catch.
            wave_height: 0.28,
            wave_length: 10.0,
            wave_speed: 1.2,
            // Low, so the glitter is a tight highlight. Water is among the smoothest surfaces a map
            // contains, and a broad highlight on it reads as wet plastic.
            roughness: 0.06,
            // Isotropic. Wind does cross a lake, but not from one quarter for long enough or with
            // enough fetch to collimate the train, and a lake with a visible grain reads as a river.
            heading: [1.0, 0.0],
            spread: std::f32::consts::PI,
            peaking: 0.0,
            current: [0.0, 0.0],
            // A faint lap rather than surf. Present at all because a shoreline with no brightening
            // whatever reads as water cut out with a cookie cutter.
            foam_depth: 0.6,
            foam_strength: 0.10,
            // None. Inland water with no fetch does not break, and a lake with whitecaps on it reads
            // as a photograph of a sea.
            whitecap: 0.0,
            // A pond is clear and still enough to see a bed wobble through, which is most of
            // what says the surface is moving when the waves are as small as a lake's.
            refraction: 1.5,
        }
    }

    /// A river reach carrying `flow`, in world units per second.
    ///
    /// Silt-green rather than blue, and with a ramp measured in single units: a river is shallow, you
    /// see its bed, and that is most of what makes one read as a river from above. The chop is fine
    /// and quick and nearly collimated down the channel.
    #[must_use]
    pub fn river(flow: [f32; 2]) -> Self {
        Self {
            // Silt rather than blue. A river takes its colour from what it is carrying and from the
            // bed a metre under it, and both are the ground's colour rather than the sky's.
            shallow: [0.24, 0.29, 0.17],
            deep: [0.06, 0.10, 0.07],
            depth_scale: 2.5,
            edge_feather: 1.2,
            // Chop rather than swell, but not so fine that it falls under the normal
            // level-of-detail at playing distance: a six-unit wave is under a pixel across a channel
            // seen from a hundred units up, and a river whose every component has been damped flat
            // renders as poured glass. This is the shortest train that still resolves there.
            //
            // The steepest of the three, at about one part in twelve crest to trough — under the one
            // in seven a gravity wave breaks at, and four times the swell's. A current shears its own
            // surface continuously and a river genuinely is the choppiest water on a map, but the
            // reason the figure had to move this far is Fresnel: a river's tint saturates a couple of
            // units down, so nothing about its *body* varies across the channel, and the only thing
            // left to make a surface out of is how much sky each wave face turns toward the camera.
            // At the first, gentler figure the whole reach rendered as a flat green ribbon.
            wave_height: 0.45,
            wave_length: 11.0,
            wave_speed: 2.6,
            // Rougher than a lake. A current shears its own surface continuously, and a river that
            // mirrors as sharply as a pond looks frozen.
            roughness: 0.14,
            heading: flow,
            // Narrow. Everything on a river runs downstream; this is the figure that carries that,
            // and it is what makes the surface read as streaked along the channel rather than dappled.
            spread: 0.30,
            peaking: 0.20,
            current: flow,
            // Water piles against a bank rather than breaking on it, so more foam than a lake and far
            // less than surf, over a band as shallow as the river itself.
            foam_depth: 1.2,
            foam_strength: 0.40,
            // A little, for the standing water that breaks over a shallow. Well under an ocean's:
            // this is riffle rather than surf.
            whitecap: 0.25,
            // The strongest of the three, because a river is the one kind whose bed is reliably
            // close enough to the surface to be seen at all.
            refraction: 2.2,
        }
    }

    /// Open sea, with the swell running along `wind`.
    ///
    /// The one preset whose figures are all large: a hundred-unit swell standing well over a metre,
    /// a ramp measured in tens of units so a sandbar reads turquoise against near-black open water,
    /// and enough foam that a beach is read from its surf. Only the direction of `wind` is used —
    /// the sea does not translate the way a river does, its waves simply travel.
    #[must_use]
    pub fn ocean(wind: [f32; 2]) -> Self {
        Self {
            shallow: [0.09, 0.28, 0.36],
            // Very nearly black. Deep sea water absorbs almost everything that is not reflected, and
            // a deep tint that is still recognisably blue leaves an ocean looking like a swimming
            // pool — the blue of the sea in a photograph is very largely the sky in it.
            deep: [0.004, 0.02, 0.05],
            depth_scale: 8.0,
            edge_feather: 2.5,
            wave_height: 1.6,
            wave_length: 110.0,
            // Slower than the deep-water relation would give for a swell this long, which is about 13
            // units a second. A game reads its own scale from how fast things move, and a swell
            // crossing the frame at true speed reads as a fast-forward.
            wave_speed: 9.0,
            roughness: 0.10,
            heading: wind,
            // A fetch this long collimates the swell, but not to a river's degree: the sea has a
            // grain, not a direction.
            spread: 0.60,
            // Steep. This is what gives an ocean its silhouette of sharp crests over flat troughs.
            peaking: 0.85,
            current: [0.0, 0.0],
            // Full strength, and the only preset at it. Surf really is white — it is froth rather
            // than water, and it hides the sea under it outright — so anything less leaves a share of
            // the body colour showing through and the band comes out as a pale blue rim instead. The
            // shader still gates it on depth and on the crest, so this is the strength of the foam
            // where there *is* foam and not a decision to whiten the whole shore.
            foam_depth: 2.5,
            foam_strength: 1.0,
            // The figure that stops open water reading as an airbrushed gradient from turquoise to
            // navy. Whitecaps are the only high-frequency detail deep sea has at this distance —
            // everything else out there is the depth ramp, which is smooth by construction.
            whitecap: 0.95,
            // Large in absolute terms and rarely visible: the swell is what displaces it, and
            // the ramp has gone to near-black long before this could show in deep water. It reads on
            // a sandbar, which is exactly where it should.
            refraction: 2.5,
        }
    }
}

impl Default for WaterMaterial {
    /// A lake, which is the body a map is likeliest to have and the one whose figures are least
    /// startling if a caller has not thought about it.
    fn default() -> Self {
        Self::lake()
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

    /// A body covering `bounds` at `elevation`, looking like `kind`.
    #[must_use]
    pub fn of_kind(bounds: [f32; 4], elevation: f32, kind: WaterKind) -> Self {
        Self {
            bounds,
            elevation,
            material: kind.material(),
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
    /// a non-positive depth ramp, feather, wavelength or foam depth, a negative spread, or a
    /// roughness, peaking or foam strength outside `0..=1`.
    pub fn validate(&self) -> Result<(), RenderError> {
        let material = &self.material;
        let finite = self.bounds.iter().all(|value| value.is_finite())
            && self.elevation.is_finite()
            && material.shallow.iter().all(|value| value.is_finite())
            && material.deep.iter().all(|value| value.is_finite())
            && material.wave_height.is_finite()
            && material.wave_speed.is_finite()
            && material.heading.iter().all(|value| value.is_finite())
            && material.current.iter().all(|value| value.is_finite())
            && material.spread.is_finite();
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
            || material.foam_depth <= 0.0
            || material.spread < 0.0
        {
            return Err(RenderError::InvalidWater);
        }
        // Each of the three is a share the shader mixes with, and each is clamped there as well —
        // but a value outside the range means the caller meant something the material cannot express,
        // and silently clamping it is how a river ends up shaded as a lake with nobody told.
        if !(0.0..=1.0).contains(&material.roughness)
            || !(0.0..=1.0).contains(&material.peaking)
            || !(0.0..=1.0).contains(&material.foam_strength)
            || !(0.0..=1.0).contains(&material.whitecap)
        {
            return Err(RenderError::InvalidWater);
        }
        // Not a share, so bounded differently: a displacement in world units, which must be finite and
        // may not pull the bed toward the camera.
        if !material.refraction.is_finite() || material.refraction < 0.0 {
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

/// The heading as a unit vector, falling back to `+x` for one too short to have a direction.
///
/// Normalised here rather than in the shader because the shader rotates *nine* directions by it every
/// time it samples, and a heading that is not unit length would scale each of them — which is a wave
/// train that quietly changes wavelength when a river's flow is given in units per minute rather than
/// per second. The fallback matters for the two kinds that have no direction of their own: a lake's
/// heading is arbitrary, so a caller zeroing it is asking for "any", not for a division by zero.
fn unit_heading(heading: [f32; 2]) -> [f32; 2] {
    let [x, y] = heading;
    let length = x.hypot(y);
    if length > 1.0e-6 && length.is_finite() {
        [x / length, y / length]
    } else {
        [1.0, 0.0]
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
        let [heading_x, heading_y] = unit_heading(material.heading);
        push_vec4(
            &mut bytes,
            [heading_x, heading_y, material.spread, material.peaking],
        );
        let [current_x, current_y] = material.current;
        push_vec4(
            &mut bytes,
            [
                current_x,
                current_y,
                material.foam_depth,
                material.foam_strength,
            ],
        );
        push_vec4(
            &mut bytes,
            [material.whitecap, material.refraction, 0.0, 0.0],
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
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(1, &self.group, &[]);
        pass.draw(0..self.vertex_count, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::unit_heading;
    use super::{MAX_CELLS, WATER_UNIFORM_BYTES, WaterKind, WaterMaterial, WaterSurface};
    use crate::RenderError;

    fn surface() -> WaterSurface {
        WaterSurface::new([0.0, 0.0, 240.0, 240.0], 5.0)
    }

    #[test]
    fn the_uniform_block_matches_the_shader_declaration() {
        // Eight `vec4`s in the WGSL `Water` struct. A mismatch does not fail validation — it silently
        // misaligns every field past the drift — so it is asserted here as well as at upload.
        assert_eq!(WATER_UNIFORM_BYTES, 128);
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
            WaterMaterial {
                peaking: 1.5,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                foam_strength: -0.1,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                foam_depth: 0.0,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                spread: -0.2,
                ..WaterMaterial::default()
            },
            WaterMaterial {
                current: [f32::INFINITY, 0.0],
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
    fn every_kind_resolves_to_a_material_the_renderer_accepts() {
        // The presets are the one set of figures nothing else checks: a caller naming a kind never
        // sees the numbers, so a typo in one of them would surface as a body that silently refuses to
        // upload rather than as anything readable.
        for kind in [
            WaterKind::Lake,
            WaterKind::River { flow: [3.0, -1.0] },
            WaterKind::Ocean { wind: [0.0, 1.0] },
        ] {
            WaterSurface::of_kind([0.0, 0.0, 240.0, 240.0], 5.0, kind)
                .validate()
                .unwrap_or_else(|error| panic!("{kind:?} should validate: {error}"));
        }
    }

    #[test]
    fn the_three_kinds_differ_in_what_makes_them_recognisable() {
        // Not a check that the numbers are the numbers — that is what the presets *are*, and asserting
        // them back would only restate the source. These are the four properties the module claims
        // separate the kinds, and each is a thing a careless retune could quietly erase.
        let lake = WaterKind::Lake.material();
        let river = WaterKind::River { flow: [4.0, 0.0] }.material();
        let ocean = WaterKind::Ocean { wind: [1.0, 0.0] }.material();

        // A river runs one way, a lake runs every way, and the sea is in between.
        assert!(
            river.spread < ocean.spread && ocean.spread < lake.spread,
            "spread: river {} ocean {} lake {}",
            river.spread,
            ocean.spread,
            lake.spread
        );
        // Only a river carries its surface with it. Compared as speeds rather than as float arrays,
        // which is both what the property is and what the workspace's lint set will accept.
        let speed = |[x, y]: [f32; 2]| x.hypot(y);
        assert!(
            (speed(river.current) - 4.0).abs() < 1.0e-6,
            "a river's current is {:?}, which is not the flow it was built from",
            river.current
        );
        assert!(
            speed(lake.current) < 1.0e-6 && speed(ocean.current) < 1.0e-6,
            "still water is moving: lake {:?}, ocean {:?}",
            lake.current,
            ocean.current
        );
        // Surf, a lap, and something between the two.
        assert!(
            lake.foam_strength < river.foam_strength && river.foam_strength < ocean.foam_strength,
            "foam: lake {} river {} ocean {}",
            lake.foam_strength,
            river.foam_strength,
            ocean.foam_strength
        );
        // And only open water breaks in the middle. A lake with whitecaps on it is a photograph of a
        // sea, which is the one way this figure can be wrong that still looks like water.
        assert!(
            lake.whitecap == 0.0 && river.whitecap < ocean.whitecap,
            "whitecaps: lake {} river {} ocean {}",
            lake.whitecap,
            river.whitecap,
            ocean.whitecap
        );
        // You see a river's bed, you do not see the sea's.
        assert!(
            river.depth_scale < lake.depth_scale && lake.depth_scale < ocean.depth_scale,
            "depth ramp: river {} lake {} ocean {}",
            river.depth_scale,
            lake.depth_scale,
            ocean.depth_scale
        );
        // And the swell is the big one.
        assert!(
            ocean.wave_length > lake.wave_length * 4.0
                && ocean.wave_height > lake.wave_height * 4.0,
            "swell: {} by {}",
            ocean.wave_length,
            ocean.wave_height
        );
    }

    #[test]
    fn a_heading_is_normalised_and_a_zero_one_falls_back() {
        let close = |actual: [f32; 2], expected: [f32; 2]| {
            (actual[0] - expected[0]).abs() < 1.0e-6 && (actual[1] - expected[1]).abs() < 1.0e-6
        };
        let along_minus_y = unit_heading([0.0, -7.0]);
        assert!(
            close(along_minus_y, [0.0, -1.0]),
            "a seven-unit flow came out as {along_minus_y:?} rather than a unit vector"
        );
        // The two kinds with no direction of their own, and a caller who scaled a flow to zero. Any
        // heading will do; dividing by the length must not be what decides that.
        assert!(close(unit_heading([0.0, 0.0]), [1.0, 0.0]));
        assert!(close(unit_heading([f32::NAN, 0.0]), [1.0, 0.0]));
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

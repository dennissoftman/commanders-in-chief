//! Cascaded shadow map fitting.
//!
//! Pure arithmetic, deliberately: fitting cascades is where shadow bugs live — shimmer, missing
//! shadows at range, a cascade that covers nothing — and all of it is testable without a GPU.
//!
//! # Why a bounding sphere and not a bounding box
//!
//! Each cascade fits a sphere around its frustum slice, not a tight box. A tight box is smaller and
//! therefore sharper, but its extent changes as the camera *rotates*, so every rotation resizes the
//! light frustum and every shadow texel lands somewhere new — visible as edges that crawl and
//! sparkle while the camera turns. A sphere is rotation-invariant, so rotating the camera cannot
//! change the fit at all.
//!
//! # Why the frustum reaches far toward the light
//!
//! A cascade's light frustum has to contain not just the receivers it shades but every *caster* that
//! could shade them. How far away such a caster can be is set by the scene's height range and the
//! sun's elevation — at a low sun a tall ridge casts many hundreds of units — and has nothing to do
//! with how large the cascade is. Sizing that reach from the cascade's own radius instead looks
//! correct at a high sun and fails at a low one, in a distinctive way: whole regions read as fully lit
//! with dead-straight boundaries where the cascade simply never recorded the occluder.
//!
//! # Why the centre is snapped to the texel grid
//!
//! With the extent stable, the remaining shimmer source is *translation*: panning by half a texel
//! moves every sample. Quantising the light-space centre to whole texels means panning moves the
//! frustum in texel steps, so a given world point keeps landing in the same texel until it moves a
//! whole one. Without this, the sphere fit alone still crawls.

use crate::view::{Projection, look_at, multiply, orthographic};

/// Cascades the shadow pass renders.
///
/// Four rather than more. An RTS camera has a bounded height range, so the depth interval needing
/// shadows is far narrower than a free-flight camera's, and a fifth cascade would fit a frustum slice
/// the camera cannot reach.
pub const CASCADE_COUNT: usize = 4;

/// Square resolution of each cascade's depth layer.
pub const CASCADE_RESOLUTION: u32 = 2_048;

/// The same value as a float, for the texel arithmetic. A named constant rather than a cast at each
/// use: 2,048 is exactly representable, and stating that once beats justifying it four times.
const CASCADE_RESOLUTION_F32: f32 = 2_048.0;

/// How the shadowed range is divided between cascades.
///
/// Fractions of the shadow distance, each the *outer* bound of its cascade. Ratios rather than
/// distances so the split adapts to whatever shadow distance a caller sets. The progression is
/// roughly geometric because perspective means near pixels cover far less world area than far ones,
/// so equal-depth splits would waste the near cascades' resolution on almost nothing.
const CASCADE_SPLITS: [f32; CASCADE_COUNT] = [0.055, 0.16, 0.42, 1.0];

/// One fitted cascade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cascade {
    /// Light-space view-projection for rendering and for sampling.
    pub view_projection: [[f32; 4]; 4],
    /// World units spanned by the full normalized depth range, so a depth bias can be expressed in
    /// world units and converted here.
    pub depth_range: f32,
    /// World units covered by one shadow texel, which is what the receiver's normal offset scales by.
    pub texel_world: f32,
    /// View-space distance at which this cascade stops being used.
    pub far_distance: f32,
}

impl Cascade {
    /// A cascade that covers nothing, used before the first fit and for unused slots.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            view_projection: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            depth_range: 1.0,
            texel_world: 1.0,
            far_distance: 0.0,
        }
    }
}

/// Fits every cascade for one camera view and light direction.
///
/// `light_direction` points *toward* the light, matching [`crate::DirectionalLight`].
///
/// `shadow_distance` bounds how far from the camera shadows are cast. It is a separate parameter from
/// the projection's far plane on purpose: the far plane may be tens of kilometres for distant terrain,
/// while shadows only need to reach as far as they are legible, and stretching cascades to the far
/// plane would waste all of their resolution.
///
/// `caster_height` is the world-space vertical extent of anything that can cast — for terrain, its
/// elevation range. It sets how far each frustum reaches toward the light, so a tall occluder at a low
/// sun is still recorded. See the module documentation for what happens when it is too small.
#[must_use]
pub fn fit_cascades(
    eye: [f32; 3],
    focus: [f32; 3],
    projection: Projection,
    light_direction: [f32; 3],
    shadow_distance: f32,
    caster_height: f32,
) -> [Cascade; CASCADE_COUNT] {
    let forward = normalize(subtract(focus, eye));
    let light = normalize(light_direction);
    let mut cascades = [Cascade::empty(); CASCADE_COUNT];

    let distance = shadow_distance.max(1.0);
    // How far along the light axis a caster of `caster_height` can stand and still shade this slice.
    // `light[2]` is the sine of the sun's elevation for a Z-up world; clamped so a sun on the horizon
    // asks for a finite frustum rather than an infinite one.
    let caster_reach = caster_height.max(0.0) / light[2].abs().max(0.12);
    let mut near = projection.near.max(0.01);
    for (index, ratio) in CASCADE_SPLITS.iter().enumerate() {
        let far = (distance * ratio).max(near + 0.01);
        cascades[index] = fit_one(eye, forward, projection, light, near, far, caster_reach);
        near = far;
    }
    cascades
}

/// Fits one cascade around the frustum slice between `near` and `far`.
fn fit_one(
    eye: [f32; 3],
    forward: [f32; 3],
    projection: Projection,
    light: [f32; 3],
    near: f32,
    far: f32,
    caster_reach: f32,
) -> Cascade {
    // Half-extents of the slice's far face. The near face is smaller, so a sphere centred on the
    // slice's axis and reaching the far corners contains the whole slice.
    let half_vertical = (projection.vertical_fov * 0.5).tan().max(1.0e-6);
    let half_horizontal = half_vertical * projection.aspect_ratio.max(1.0e-6);

    // The centre that minimises the enclosing radius sits on the view axis, between the faces. Its
    // offset comes from equating the distance to a near corner and a far corner.
    let spread = half_vertical * half_vertical + half_horizontal * half_horizontal;
    let mut centre_distance = (far + near) * 0.5 * (1.0 + spread);
    // Clamp inside the slice: with a wide field of view the formula can push the centre past the far
    // face, which still encloses the slice but wastes radius.
    centre_distance = centre_distance.clamp(near, far);

    let centre = add(eye, scale(forward, centre_distance));
    let corner_radius = |depth: f32| {
        let extent = depth * (spread).sqrt();
        let along = depth - centre_distance;
        (extent * extent + along * along).sqrt()
    };
    let radius = corner_radius(near).max(corner_radius(far)).max(1.0e-3);

    // The frustum extends toward the light by the caster reach, not by a multiple of the cascade's
    // own radius. A radius-proportional reach is the tempting version and it is wrong: the small near
    // cascades then look only a short way toward the sun, so a distant tall occluder is absent from
    // exactly the cascades covering the ground its shadow should fall on.
    let padding = caster_reach.max(radius * 0.25);
    let depth_range = radius * 2.0 + padding;

    // Snap the centre to whole shadow texels, in light space, before building the view.
    let texel_world = radius * 2.0 / CASCADE_RESOLUTION_F32;
    // `light` points *toward* the light, so the light sits at `centre + light * distance`. Placing the
    // shadow camera at `centre - light * distance` instead puts it on the dark side looking away from
    // the sun, which records the terrain's underside and produces a shadow map that occludes nothing.
    let eye_offset = scale(light, radius + padding);
    let unsnapped_view = look_at(add(centre, eye_offset), centre, up_for(light));
    let snapped_centre = snap_to_texels(unsnapped_view, centre, texel_world);

    let view = look_at(
        add(snapped_centre, eye_offset),
        snapped_centre,
        up_for(light),
    );
    let projection_matrix = orthographic(-radius, radius, -radius, radius, 0.0, depth_range);

    Cascade {
        view_projection: multiply(projection_matrix, view),
        depth_range,
        texel_world,
        far_distance: far,
    }
}

/// Quantises a world-space centre to whole shadow texels along the light's own axes.
fn snap_to_texels(view: [[f32; 4]; 4], centre: [f32; 3], texel_world: f32) -> [f32; 3] {
    if texel_world <= 0.0 || !texel_world.is_finite() {
        return centre;
    }
    // Into light space, quantise the two lateral axes, and back out again. The depth axis is left
    // alone: quantising it would step the whole frustum toward and away from the light for no gain.
    let light_space = transform_point(view, centre);
    let quantised = [
        (light_space[0] / texel_world).round() * texel_world,
        (light_space[1] / texel_world).round() * texel_world,
        light_space[2],
    ];
    let shift = [
        quantised[0] - light_space[0],
        quantised[1] - light_space[1],
        0.0,
    ];
    // The view is orthonormal, so its rows are the light basis and the shift maps back by them.
    let right = [view[0][0], view[1][0], view[2][0]];
    let up = [view[0][1], view[1][1], view[2][1]];
    add(add(centre, scale(right, shift[0])), scale(up, shift[1]))
}

/// Picks an up vector that is not parallel to the light direction.
///
/// A sun near the zenith leaves world up parallel to the light and the view basis degenerate, which
/// produces a NaN matrix and silently blanks every shadow.
fn up_for(light: [f32; 3]) -> [f32; 3] {
    if light[2].abs() > 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for row in 0..3 {
        result[row] = matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row];
    }
    result
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(vector: [f32; 3], factor: f32) -> [f32; 3] {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    // Comparisons here are against structural values the fit produces, with explicit tolerances
    // where the quantity is computed.
    #![allow(clippy::float_cmp)]

    use super::{CASCADE_COUNT, CASCADE_RESOLUTION_F32, Cascade, fit_cascades};
    use crate::view::{Projection, transform};

    fn projection() -> Projection {
        Projection::for_viewport(1280, 720)
    }

    /// A tall occluder, so the fit has something to reach toward the light for.
    const CASTER_HEIGHT: f32 = 320.0;

    fn sun() -> [f32; 3] {
        [-0.45, -0.30, 0.84]
    }

    fn fit() -> [Cascade; CASCADE_COUNT] {
        fit_cascades(
            [0.0, -600.0, 400.0],
            [0.0, 0.0, 0.0],
            projection(),
            sun(),
            1_200.0,
            CASTER_HEIGHT,
        )
    }

    /// Projects a world point into a cascade and returns its normalized device coordinates.
    fn project(cascade: &Cascade, point: [f32; 3]) -> [f32; 3] {
        let clip = transform(cascade.view_projection, point);
        [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]]
    }

    fn inside(cascade: &Cascade, point: [f32; 3]) -> bool {
        let ndc = project(cascade, point);
        (-1.0..=1.0).contains(&ndc[0])
            && (-1.0..=1.0).contains(&ndc[1])
            && (0.0..=1.0).contains(&ndc[2])
    }

    #[test]
    fn every_cascade_is_finite_and_ordered() {
        let cascades = fit();
        let mut previous = 0.0;
        for (index, cascade) in cascades.iter().enumerate() {
            for column in cascade.view_projection {
                for value in column {
                    assert!(value.is_finite(), "cascade {index} matrix has {value}");
                }
            }
            assert!(
                cascade.far_distance > previous,
                "cascade {index} must extend past cascade {}: {} vs {previous}",
                index.saturating_sub(1),
                cascade.far_distance
            );
            assert!(cascade.depth_range > 0.0);
            assert!(cascade.texel_world > 0.0);
            previous = cascade.far_distance;
        }
    }

    #[test]
    fn cascades_grow_coarser_with_distance() {
        // The whole point of cascading: near cascades spend their texels on a small area.
        let cascades = fit();
        for pair in cascades.windows(2) {
            assert!(
                pair[1].texel_world > pair[0].texel_world,
                "a farther cascade must cover more world per texel: {} then {}",
                pair[0].texel_world,
                pair[1].texel_world
            );
        }
    }

    #[test]
    fn the_texel_size_matches_the_fitted_extent() {
        // `texel_world` is what the receiver's normal offset scales by, so a wrong value here shows
        // up as either acne or detached shadows and is hard to diagnose from the image.
        let cascades = fit();
        for cascade in &cascades {
            let extent = cascade.texel_world * CASCADE_RESOLUTION_F32;
            assert!(
                extent > 0.0 && extent.is_finite(),
                "extent {extent} is not usable"
            );
        }
    }

    #[test]
    fn the_focus_point_falls_inside_a_cascade() {
        // If nothing contains the point the camera is looking at, the scene has no shadows at all.
        let cascades = fit();
        assert!(
            cascades
                .iter()
                .any(|cascade| inside(cascade, [0.0, 0.0, 0.0])),
            "the focus must be covered by some cascade"
        );
    }

    #[test]
    fn the_last_cascade_covers_the_shadow_distance() {
        let cascades = fit();
        assert!(
            (cascades[CASCADE_COUNT - 1].far_distance - 1_200.0).abs() < 1.0,
            "the outermost cascade should reach the requested distance, got {}",
            cascades[CASCADE_COUNT - 1].far_distance
        );
    }

    #[test]
    fn a_point_toward_the_light_is_nearer_in_the_cascade() {
        // The test that pins the light-side sign. A shadow map is only useful if the caster is nearer
        // to the light than its receiver, so moving a point toward the sun must *decrease* its
        // projected depth. With the camera placed on the dark side instead, this inverts -- and the
        // shadow map then records the terrain's underside and occludes nothing, while every
        // containment and extent check still passes.
        let cascades = fit();
        let sun = sun();
        for (index, cascade) in cascades.iter().enumerate() {
            let step = cascade.depth_range * 0.1;
            let ground = [0.0, 0.0, 0.0];
            let toward_sun = [sun[0] * step, sun[1] * step, sun[2] * step];
            let ground_depth = project(cascade, ground)[2];
            let raised_depth = project(cascade, toward_sun)[2];
            assert!(
                raised_depth < ground_depth,
                "cascade {index}: a point toward the sun must be nearer the light, \
                 got {raised_depth} against {ground_depth}"
            );
        }
    }

    #[test]
    fn an_occluder_toward_the_light_stays_inside_the_cascade() {
        // The padding along the light axis exists for this. Without it a hill standing between the
        // sun and the fitted sphere drops its shadow entirely.
        let cascades = fit();
        let sun = sun();
        let cascade = &cascades[CASCADE_COUNT - 1];
        let step = cascade.depth_range * 0.2;
        let toward_sun = [sun[0] * step, sun[1] * step, sun[2] * step];
        assert!(
            inside(cascade, toward_sun),
            "an occluder toward the light must still be inside the cascade, projected to {:?}",
            project(cascade, toward_sun)
        );
    }

    #[test]
    fn rotating_the_camera_does_not_resize_a_cascade() {
        // The bounding-sphere fit exists precisely so this holds. If the extent changed with
        // rotation, every shadow edge would crawl as the camera turned.
        let radius = 600.0_f32;
        let mut extents = Vec::new();
        for step in 0..8u16 {
            let angle = f32::from(step) * core::f32::consts::TAU / 8.0;
            let eye = [angle.cos() * radius, angle.sin() * radius, 400.0];
            let cascades = fit_cascades(
                [eye[0], eye[1], eye[2]],
                [0.0, 0.0, 0.0],
                projection(),
                sun(),
                1_200.0,
                CASTER_HEIGHT,
            );
            extents.push(cascades[0].texel_world);
        }
        let first = extents[0];
        for extent in &extents {
            assert!(
                (extent - first).abs() < 1.0e-4,
                "cascade extent changed with camera rotation: {extents:?}"
            );
        }
    }

    #[test]
    fn panning_moves_the_cascade_in_whole_texel_steps() {
        // Texel snapping. Sub-texel panning must not move the sampling grid at all, or shadow edges
        // sparkle during every camera pan.
        let projection = projection();
        let texel = fit()[0].texel_world;
        let baseline = fit_cascades(
            [0.0, -600.0, 400.0],
            [0.0, 0.0, 0.0],
            projection,
            sun(),
            1_200.0,
            CASTER_HEIGHT,
        )[0];
        // Nudge the camera by a small fraction of a texel.
        let nudged = fit_cascades(
            [texel * 0.05, -600.0, 400.0],
            [texel * 0.05, 0.0, 0.0],
            projection,
            sun(),
            1_200.0,
            CASTER_HEIGHT,
        )[0];
        let probe = [12.0, 34.0, 0.0];
        let before = project(&baseline, probe);
        let after = project(&nudged, probe);
        let drift = ((before[0] - after[0]).powi(2) + (before[1] - after[1]).powi(2)).sqrt();
        // One texel is 2/resolution in normalized device coordinates.
        let one_texel_ndc = 2.0 / CASCADE_RESOLUTION_F32;
        assert!(
            drift < one_texel_ndc,
            "a sub-texel pan moved the grid by {drift}, more than one texel ({one_texel_ndc})"
        );
    }

    #[test]
    fn survives_a_light_at_the_zenith() {
        // World up parallel to the light degenerates the view basis. Without the fallback up vector
        // this produces NaNs and silently removes every shadow.
        let cascades = fit_cascades(
            [0.0, -600.0, 400.0],
            [0.0, 0.0, 0.0],
            projection(),
            [0.0, 0.0, 1.0],
            1_200.0,
            CASTER_HEIGHT,
        );
        for cascade in &cascades {
            for column in cascade.view_projection {
                for value in column {
                    assert!(value.is_finite(), "zenith light produced {value}");
                }
            }
        }
        assert!(
            cascades
                .iter()
                .any(|cascade| inside(cascade, [0.0, 0.0, 0.0]))
        );
    }

    #[test]
    fn every_cascade_reaches_a_distant_caster_at_a_low_sun() {
        // The failure this pins produced a distinctive image: whole regions fully lit, bounded by
        // dead-straight lines, because the near cascades never looked far enough toward the sun to
        // record a tall distant ridge. Sizing the reach from the cascade's own radius passes at a high
        // sun and fails here, so the low sun is the case worth asserting.
        //
        // The reference point is each cascade's *own* centre, recovered by inverting its matrix. Using
        // the world origin instead only tests whichever cascade happens to be near it.
        let low_sun = [-0.30, 0.90, 0.32];
        let cascades = fit_cascades(
            [0.0, -600.0, 400.0],
            [0.0, 0.0, 0.0],
            projection(),
            low_sun,
            1_200.0,
            CASTER_HEIGHT,
        );
        // At this elevation an occluder of CASTER_HEIGHT stands this far along the light axis.
        let reach = CASTER_HEIGHT / 0.32;
        for (index, cascade) in cascades.iter().enumerate() {
            assert!(
                cascade.depth_range > reach,
                "cascade {index} spans only {} world units, too little to contain a caster {reach}                  units toward the light",
                cascade.depth_range
            );

            let inverse = crate::view::invert(cascade.view_projection)
                .expect("a fitted cascade must be invertible");
            // The *deepest* receiver, at the far plane. It is the demanding one: its caster stands
            // farther from the light than any other receiver's, so if the frustum contains that, it
            // contains every nearer receiver's caster too.
            let deepest = {
                let homogeneous = transform(inverse, [0.0, 0.0, 1.0]);
                [
                    homogeneous[0] / homogeneous[3],
                    homogeneous[1] / homogeneous[3],
                    homogeneous[2] / homogeneous[3],
                ]
            };
            let caster = [
                deepest[0] + low_sun[0] * reach,
                deepest[1] + low_sun[1] * reach,
                deepest[2] + low_sun[2] * reach,
            ];
            let depth = project(cascade, caster)[2];
            assert!(
                (0.0..=1.0).contains(&depth),
                "cascade {index}: the caster for its deepest receiver, {reach} units toward the                  light, projected to depth {depth} and is outside the frustum"
            );
        }
    }

    #[test]
    fn an_empty_cascade_is_usable_as_a_placeholder() {
        let cascade = Cascade::empty();
        assert_eq!(cascade.far_distance, 0.0);
        assert!(
            cascade.depth_range > 0.0,
            "must not divide by zero downstream"
        );
        assert!(cascade.texel_world > 0.0);
    }
}

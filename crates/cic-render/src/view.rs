//! View and projection matrices.
//!
//! These live in the renderer rather than in `cic-camera` on purpose: a projection depends on the
//! viewport and on the graphics API's clip-space convention, neither of which the camera model has
//! any business knowing. The camera decides *where it is*; this decides how that becomes clip space.

use cic_camera::CameraPose;

/// World up. The world is Z-up, which is the convention the terrain container and the camera share.
const WORLD_UP: [f32; 3] = [0.0, 0.0, 1.0];

/// Perspective parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    /// Vertical field of view in radians.
    pub vertical_fov: f32,
    /// Viewport width divided by height.
    pub aspect_ratio: f32,
    /// Near plane distance, which must be positive.
    pub near: f32,
    /// Far plane distance, which must exceed `near`.
    pub far: f32,
}

impl Projection {
    /// Builds a projection for a viewport, with defaults suited to an RTS camera's working distance.
    #[must_use]
    pub fn for_viewport(width: u32, height: u32) -> Self {
        // Both are bounded by the capture limits at 8,192, well inside exact f32 range.
        #[allow(clippy::cast_precision_loss)]
        let aspect_ratio = if height == 0 {
            1.0
        } else {
            width as f32 / height as f32
        };
        Self {
            vertical_fov: 50.0_f32.to_radians(),
            aspect_ratio,
            near: 1.0,
            far: 8_000.0,
        }
    }
}

/// Returns a right-handed perspective matrix mapping depth to `0..=1`, which is what this graphics
/// API expects — an OpenGL-style `-1..=1` matrix here would clip half the scene away.
///
/// Column-major: `matrix[column][row]`.
#[must_use]
pub fn perspective(projection: Projection) -> [[f32; 4]; 4] {
    let half = (projection.vertical_fov * 0.5).tan().max(1.0e-6);
    let vertical = 1.0 / half;
    let horizontal = vertical / projection.aspect_ratio.max(1.0e-6);
    let range = projection.far / (projection.near - projection.far);
    [
        [horizontal, 0.0, 0.0, 0.0],
        [0.0, vertical, 0.0, 0.0],
        [0.0, 0.0, range, -1.0],
        [0.0, 0.0, range * projection.near, 0.0],
    ]
}

/// Returns a right-handed view matrix looking from `eye` toward `target`.
///
/// Column-major: `matrix[column][row]`.
#[must_use]
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
    let forward = normalize(subtract(target, eye));
    // A camera looking straight down leaves `forward` parallel to world up and the cross product
    // degenerate. Substituting a different up keeps the basis well-formed rather than producing NaNs
    // that silently blank the frame.
    let chosen_up = if cross(forward, up) == [0.0, 0.0, 0.0] {
        [0.0, 1.0, 0.0]
    } else {
        up
    };
    let right = normalize(cross(forward, chosen_up));
    let true_up = cross(right, forward);
    [
        [right[0], true_up[0], -forward[0], 0.0],
        [right[1], true_up[1], -forward[1], 0.0],
        [right[2], true_up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(true_up, eye), dot(forward, eye), 1.0],
    ]
}

/// Returns `projection * view` for a camera pose and viewport.
#[must_use]
pub fn view_projection(pose: CameraPose, projection: Projection) -> [[f32; 4]; 4] {
    multiply(
        perspective(projection),
        look_at(pose.eye, pose.focus, WORLD_UP),
    )
}

/// Multiplies two column-major matrices.
#[must_use]
pub fn multiply(left: [[f32; 4]; 4], right: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut result = [[0.0f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for step in 0..4 {
                sum += left[step][row] * right[column][step];
            }
            result[column][row] = sum;
        }
    }
    result
}

/// Transforms a point by a column-major matrix, returning the clip-space result including `w`.
#[must_use]
pub fn transform(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 4] {
    let mut result = [0.0f32; 4];
    for row in 0..4 {
        result[row] = matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row];
    }
    result
}

fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 && length.is_finite() {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    // Exact comparisons below are against values the matrices produce structurally (zeros, ones,
    // and midpoints), not against measured quantities.
    #![allow(clippy::float_cmp)]

    use super::{Projection, look_at, multiply, perspective, transform};

    fn identity() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    #[test]
    fn multiplication_respects_identity() {
        let matrix = perspective(Projection::for_viewport(800, 600));
        assert_eq!(multiply(identity(), matrix), matrix);
        assert_eq!(multiply(matrix, identity()), matrix);
    }

    #[test]
    fn projection_maps_the_near_and_far_planes_to_zero_and_one() {
        // This is the assertion that catches an OpenGL-convention matrix, which would put the near
        // plane at -1 and clip away half the scene.
        let projection = Projection {
            vertical_fov: 60.0_f32.to_radians(),
            aspect_ratio: 1.0,
            near: 1.0,
            far: 100.0,
        };
        let matrix = perspective(projection);

        // A right-handed view looks down -Z, so a point in front has negative Z.
        let near = transform(matrix, [0.0, 0.0, -projection.near]);
        assert!(near[3] > 0.0, "w must be positive in front of the camera");
        assert!(
            (near[2] / near[3]).abs() < 1.0e-4,
            "near plane should land at depth 0, got {}",
            near[2] / near[3]
        );

        let far = transform(matrix, [0.0, 0.0, -projection.far]);
        assert!(
            ((far[2] / far[3]) - 1.0).abs() < 1.0e-4,
            "far plane should land at depth 1, got {}",
            far[2] / far[3]
        );
    }

    #[test]
    fn projection_keeps_a_centred_point_centred() {
        let matrix = perspective(Projection::for_viewport(1920, 1080));
        let point = transform(matrix, [0.0, 0.0, -10.0]);
        assert!((point[0] / point[3]).abs() < 1.0e-6);
        assert!((point[1] / point[3]).abs() < 1.0e-6);
    }

    #[test]
    fn aspect_ratio_compresses_the_wider_axis() {
        // A wide viewport must scale X less than Y, or the scene stretches horizontally.
        let wide = perspective(Projection::for_viewport(1920, 1080));
        assert!(
            wide[0][0] < wide[1][1],
            "horizontal scale {} should be below vertical {}",
            wide[0][0],
            wide[1][1]
        );
        let square = perspective(Projection::for_viewport(512, 512));
        assert!((square[0][0] - square[1][1]).abs() < 1.0e-6);
    }

    #[test]
    fn view_places_the_eye_at_the_origin_looking_down_negative_z() {
        let eye = [10.0, 20.0, 30.0];
        let matrix = look_at(eye, [10.0, 25.0, 30.0], [0.0, 0.0, 1.0]);
        let at_eye = transform(matrix, eye);
        for axis in 0..3 {
            assert!(
                at_eye[axis].abs() < 1.0e-4,
                "the eye should map to the origin, got {at_eye:?}"
            );
        }
        // The target is 5 units ahead, so it sits at -5 on the view axis.
        let at_target = transform(matrix, [10.0, 25.0, 30.0]);
        assert!(
            (at_target[2] + 5.0).abs() < 1.0e-4,
            "target should sit 5 units down -Z, got {at_target:?}"
        );
    }

    #[test]
    fn view_survives_a_camera_looking_straight_down() {
        // `forward` parallel to world up degenerates the cross product. Without the fallback this
        // produces NaNs and a blank frame rather than an error.
        let matrix = look_at([0.0, 0.0, 100.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        for column in matrix {
            for value in column {
                assert!(value.is_finite(), "matrix must stay finite: {matrix:?}");
            }
        }
        let below = transform(matrix, [0.0, 0.0, 0.0]);
        assert!(
            (below[2] + 100.0).abs() < 1.0e-3,
            "the ground should sit 100 units ahead, got {below:?}"
        );
    }
}

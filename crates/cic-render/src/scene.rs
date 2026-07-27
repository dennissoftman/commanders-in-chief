//! Whole-frame composition: set up a pass, draw what is in it, resolve a capture.
//!
//! One function rather than a builder, because there is currently one pass. When the deferred chain
//! lands this becomes the place that sequences G-buffer, shadow, ambient-occlusion, and lighting
//! passes — so it exists now to keep that sequencing out of the terrain module.

use cic_assets::Terrain;
use cic_camera::CameraPose;

use crate::RenderError;
use crate::gpu::{Capture, CaptureTarget, GpuContext};
use crate::terrain::{DirectionalLight, LayerColour, TerrainRenderer};
use crate::view::{Projection, view_projection};

/// Sky colour the frame is cleared to.
///
/// Not black. A black clear makes an unlit or mis-projected terrain indistinguishable from an empty
/// frame, which is exactly the failure a capture is supposed to reveal.
pub const CLEAR_COLOUR: wgpu::Color = wgpu::Color {
    r: 0.055,
    g: 0.075,
    b: 0.110,
    a: 1.0,
};

/// Everything one terrain frame needs.
#[derive(Debug, Clone, Copy)]
pub struct TerrainFrame {
    /// Where the camera is.
    pub pose: CameraPose,
    /// How the viewport projects.
    pub projection: Projection,
    /// The directional light.
    pub light: DirectionalLight,
}

impl TerrainFrame {
    /// Builds a frame that looks at the centre of a terrain from a sensible standoff.
    ///
    /// Convenience for captures and for a first look at a map: the distance is derived from the
    /// terrain's own extent, so it frames a small test grid and a large map equally well.
    #[must_use]
    pub fn overview(terrain: &Terrain, width: u32, height: u32) -> Self {
        let [extent_x, extent_y] = terrain.world_extent();
        let centre = [extent_x * 0.5, extent_y * 0.5];
        let span = extent_x.max(extent_y).max(1.0);
        // Stand back along -Y and up, looking down at roughly the camera's default tilt. The 0.9
        // factor frames the terrain with a small margin rather than exactly filling the viewport.
        let distance = span * 0.9;
        let pose = CameraPose {
            eye: [centre[0], centre[1] - distance * 0.72, distance * 0.62],
            focus: [centre[0], centre[1], 0.0],
            forward: [0.0, 0.72, -0.62],
        };
        Self {
            pose,
            projection: Projection::for_viewport(width, height),
            light: DirectionalLight::default(),
        }
    }
}

/// Renders one terrain headlessly and returns the resolved capture.
///
/// # Errors
///
/// Returns a structured [`RenderError`] when the capture target cannot be allocated, the terrain
/// declares more layers than the pass blends, or readback fails.
pub fn capture_terrain(
    context: &GpuContext,
    terrain: &Terrain,
    palette: &[LayerColour],
    frame: TerrainFrame,
    width: u32,
    height: u32,
) -> Result<Capture, RenderError> {
    let target = CaptureTarget::new(context, width, height)?;
    let renderer = TerrainRenderer::new(context, terrain, palette)?;
    renderer.set_frame(
        context,
        &view_projection(frame.pose, frame.projection),
        frame.pose.eye,
        frame.light,
    );
    render_terrain_into(context, &target, &renderer)?;
    target.resolve(
        context,
        context
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cic-render terrain resolve"),
            }),
    )
}

/// Records and submits the terrain pass into a target.
///
/// Public so a caller can render the *same* renderer more than once — which is what verifying a
/// runtime terrain or layer edit requires: draw, write a texture region, draw again, compare.
///
/// # Errors
///
/// Currently infallible, but returns `Result` because the deferred chain's passes will not be.
pub fn render_terrain_into(
    context: &GpuContext,
    target: &CaptureTarget,
    renderer: &TerrainRenderer,
) -> Result<(), RenderError> {
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cic-render terrain pass"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cic-render terrain forward"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.colour_view(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(CLEAR_COLOUR),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target.depth_view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            multiview_mask: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        renderer.draw(&mut pass);
    }
    context.queue().submit([encoder.finish()]);
    Ok(())
}

//! Model geometry on the GPU: instanced, with per-instance tint.
//!
//! # One draw per model, not per primitive
//!
//! A glTF model arrives as several primitives, each with its own material. The obvious mapping is one
//! draw call per primitive with the material bound as state — which means a bind group change between
//! every one, and a model of twenty primitives costs twenty state transitions per frame.
//!
//! Instead the material *index* is a per-vertex attribute. Every vertex of a primitive shares the same
//! index, so all of a model's primitives concatenate into one vertex and index buffer, materials live
//! in one storage buffer bound once, and the whole model draws in a single instanced call. The cost is
//! four bytes per vertex.
//!
//! # Per-instance tint
//!
//! Instances carry a colour multiplier as well as a transform. Recovered equipment that keeps its
//! original silhouette under different markings is a shared mesh with a per-instance colour, so the
//! channel exists from the start rather than being retrofitted into the vertex format later.

use cic_assets::Model;

use crate::RenderError;

/// Bytes per model vertex: position, normal, texture coordinates, material index.
const VERTEX_STRIDE: usize = 3 * 4 + 3 * 4 + 2 * 4 + 4;

/// Bytes per instance: a column-major transform and a tint.
const INSTANCE_STRIDE: usize = 64 + 16;

/// Bytes per material: base colour, then metallic and roughness with padding to a 16-byte boundary.
const MATERIAL_STRIDE: usize = 16 + 16;

/// One placement of a model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInstance {
    /// Column-major world transform: `transform[column][row]`.
    pub transform: [[f32; 4]; 4],
    /// Multiplied into the material's base colour. `[1.0; 4]` leaves the material as authored.
    pub tint: [f32; 4],
}

impl Default for ModelInstance {
    fn default() -> Self {
        Self {
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            tint: [1.0; 4],
        }
    }
}

impl ModelInstance {
    /// A placement at a world position, rotated about Z and uniformly scaled.
    ///
    /// Uniform scale specifically: the shader transforms normals by the transform's upper 3x3 and
    /// renormalizes, which is exact for rotation and uniform scale and wrong under shear or
    /// non-uniform scale. Offering only what is correct beats offering a general transform that
    /// silently mis-lights.
    #[must_use]
    pub fn placed(position: [f32; 3], rotation_radians: f32, scale: f32) -> Self {
        let (sin, cos) = rotation_radians.sin_cos();
        Self {
            transform: [
                [cos * scale, sin * scale, 0.0, 0.0],
                [-sin * scale, cos * scale, 0.0, 0.0],
                [0.0, 0.0, scale, 0.0],
                [position[0], position[1], position[2], 1.0],
            ],
            tint: [1.0; 4],
        }
    }

    /// Returns the instance with a tint applied.
    #[must_use]
    pub const fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }
}

/// One model uploaded to the GPU with its instances.
#[derive(Debug)]
pub struct ModelBatch {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    material_group: wgpu::BindGroup,
    index_count: u32,
    instance_count: u32,
    top: f32,
}

impl ModelBatch {
    /// Returns the bind group layout the material storage buffer is bound through.
    ///
    /// Built once by the renderer and shared: the pipelines are created against it and every batch's
    /// bind group uses it, so the two cannot drift apart.
    #[must_use]
    pub fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cic-render model material layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    // Storage rather than uniform: a uniform array would have to be a fixed size, and
                    // a model's material count is whatever its author gave it.
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    /// Uploads a model and its instances.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyModel`] when the model has no geometry, or
    /// [`RenderError::ModelTooLarge`] when its vertex or index count exceeds the addressable range.
    pub fn new(
        context: &crate::GpuContext,
        model: &Model,
        instances: &[ModelInstance],
        material_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, RenderError> {
        if model.primitives.is_empty() || model.vertex_count() == 0 {
            return Err(RenderError::EmptyModel);
        }

        let (vertices, indices, index_count) = pack_geometry(model)?;

        // Slot 0 is a neutral default for primitives that declare no material; the model's own
        // materials follow it, which is why the index above is shifted by one.
        let mut materials: Vec<u8> =
            Vec::with_capacity((model.materials.len() + 1) * MATERIAL_STRIDE);
        push_material(&mut materials, [0.8, 0.8, 0.8, 1.0], 0.0, 0.7);
        for material in &model.materials {
            push_material(
                &mut materials,
                material.base_color,
                material.metallic,
                material.roughness,
            );
        }

        let mut instance_bytes: Vec<u8> = Vec::with_capacity(instances.len() * INSTANCE_STRIDE);
        for instance in instances {
            for column in instance.transform {
                for value in column {
                    instance_bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            for value in instance.tint {
                instance_bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        // An empty batch is legal and draws nothing, but a zero-sized buffer is not, so one instance
        // worth of zeroes is allocated and the draw is skipped by `instance_count`.
        if instance_bytes.is_empty() {
            instance_bytes.resize(INSTANCE_STRIDE, 0);
        }

        let device = context.device();
        let vertex_buffer = buffer(
            device,
            "cic-render model vertices",
            &vertices,
            wgpu::BufferUsages::VERTEX,
        );
        let index_buffer = buffer(
            device,
            "cic-render model indices",
            &indices,
            wgpu::BufferUsages::INDEX,
        );
        let instance_buffer = buffer(
            device,
            "cic-render model instances",
            &instance_bytes,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let material_buffer = buffer(
            device,
            "cic-render model materials",
            &materials,
            wgpu::BufferUsages::STORAGE,
        );
        context.queue().write_buffer(&vertex_buffer, 0, &vertices);
        context.queue().write_buffer(&index_buffer, 0, &indices);
        context
            .queue()
            .write_buffer(&instance_buffer, 0, &instance_bytes);
        context
            .queue()
            .write_buffer(&material_buffer, 0, &materials);

        let material_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cic-render model materials"),
            layout: material_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: material_buffer.as_entire_binding(),
            }],
        });

        Ok(Self {
            vertex_buffer,
            index_buffer,
            instance_buffer,
            material_group,
            index_count,
            instance_count: u32::try_from(instances.len())
                .map_err(|_| RenderError::ModelTooLarge)?,
            top: world_top(model, instances),
        })
    }

    /// Overwrites the instance data, keeping the geometry.
    ///
    /// The instance count may only shrink or stay the same, because the buffer was sized at upload.
    /// Growing it needs a new batch — which is the honest constraint rather than a silent truncation.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::ModelTooLarge`] when more instances are supplied than the batch was
    /// built with.
    pub fn set_instances(
        &mut self,
        context: &crate::GpuContext,
        instances: &[ModelInstance],
    ) -> Result<(), RenderError> {
        let count = u32::try_from(instances.len()).map_err(|_| RenderError::ModelTooLarge)?;
        if u64::from(count) * INSTANCE_STRIDE as u64 > self.instance_buffer.size() {
            return Err(RenderError::ModelTooLarge);
        }
        let mut bytes = Vec::with_capacity(instances.len() * INSTANCE_STRIDE);
        for instance in instances {
            for column in instance.transform {
                for value in column {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            for value in instance.tint {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        if !bytes.is_empty() {
            context
                .queue()
                .write_buffer(&self.instance_buffer, 0, &bytes);
        }
        self.instance_count = count;
        // The tallest instance may have changed, and the cascades are sized from it.
        self.top = instances
            .iter()
            .map(|instance| instance.transform[3][2])
            .fold(f32::NEG_INFINITY, f32::max)
            .max(self.top);
        Ok(())
    }

    /// Returns the highest world-space Z any instance of this batch reaches.
    ///
    /// A shadow cascade sizes how far it looks toward the light from the tallest thing that can cast,
    /// and a model standing on terrain is taller than the terrain alone. Without this a tall model at a
    /// low sun casts no shadow, because the cascade never looked far enough to record it.
    #[must_use]
    pub const fn world_top(&self) -> f32 {
        self.top
    }

    /// Returns how many instances will be drawn.
    #[must_use]
    pub const fn instance_count(&self) -> u32 {
        self.instance_count
    }

    /// Returns how many triangles one instance contains.
    #[must_use]
    pub const fn triangle_count(&self) -> u32 {
        self.index_count / 3
    }

    /// Records a draw. The caller has already set the pipeline and any other bind groups.
    ///
    /// `material_group_index` is `Some(2)` for the G-buffer pass and `None` for the shadow pass, which
    /// needs no materials at all. The caller states it rather than this guessing, because the two
    /// passes genuinely bind different groups.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, material_group_index: Option<u32>) {
        if self.instance_count == 0 || self.index_count == 0 {
            return;
        }
        if let Some(index) = material_group_index {
            pass.set_bind_group(index, &self.material_group, &[]);
        }
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
    }
}

/// The vertex and instance buffer layouts, in the order the pipelines expect them.
///
/// Wrapped in `Some` because a pipeline may leave a vertex buffer slot empty, the same way it may
/// leave a bind group slot empty.
#[must_use]
pub fn buffer_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 24,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 32,
                    shader_location: 3,
                },
            ],
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: INSTANCE_STRIDE as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A 4x4 transform arrives as four separate vec4 attributes; there is no matrix
                // vertex format.
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 8,
                },
            ],
        }),
    ]
}

/// Concatenates a model into one vertex and index buffer, with each vertex carrying its material.
///
/// Returns the packed bytes and the total index count. Indices are rebased per primitive, which is
/// what allows the whole model to draw in a single call.
///
/// # Errors
///
/// Returns [`RenderError::ModelTooLarge`] when a count or a rebased index leaves the addressable range.
fn pack_geometry(model: &Model) -> Result<(Vec<u8>, Vec<u8>, u32), RenderError> {
    let mut vertices: Vec<u8> = Vec::new();
    let mut indices: Vec<u8> = Vec::new();
    let mut base_vertex = 0u32;
    let mut index_count = 0u32;

    for primitive in &model.primitives {
        // A primitive with no material takes slot 0 and gets the default below, so a fragment
        // never indexes past the storage buffer.
        let material = u32::try_from(primitive.material.map_or(0, |index| index + 1))
            .map_err(|_| RenderError::ModelTooLarge)?;
        for vertex in &primitive.vertices {
            for value in vertex.position {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.normal {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            for value in vertex.uv {
                vertices.extend_from_slice(&value.to_le_bytes());
            }
            vertices.extend_from_slice(&material.to_le_bytes());
        }
        // Indices are primitive-local, so each primitive's are offset by the vertices already
        // written. That is what allows one draw call for the whole model.
        for index in &primitive.indices {
            let shifted = index
                .checked_add(base_vertex)
                .ok_or(RenderError::ModelTooLarge)?;
            indices.extend_from_slice(&shifted.to_le_bytes());
        }
        index_count = index_count
            .checked_add(
                u32::try_from(primitive.indices.len()).map_err(|_| RenderError::ModelTooLarge)?,
            )
            .ok_or(RenderError::ModelTooLarge)?;
        base_vertex = base_vertex
            .checked_add(
                u32::try_from(primitive.vertices.len()).map_err(|_| RenderError::ModelTooLarge)?,
            )
            .ok_or(RenderError::ModelTooLarge)?;
    }
    Ok((vertices, indices, index_count))
}

/// Highest world-space Z reached by any instance of a model.
fn world_top(model: &Model, instances: &[ModelInstance]) -> f32 {
    let Some((_, maximum)) = model.bounds() else {
        return 0.0;
    };
    let mut top = f32::NEG_INFINITY;
    for instance in instances {
        // Only the Z row of the transform matters, applied to the bound's upper corner. The upper
        // corner suffices because the transform allows no shear: rotation about Z leaves Z unchanged
        // and uniform scale cannot make a lower corner the taller one.
        let z = instance.transform[0][2] * maximum[0]
            + instance.transform[1][2] * maximum[1]
            + instance.transform[2][2] * maximum[2]
            + instance.transform[3][2];
        top = top.max(z);
    }
    if top.is_finite() { top } else { 0.0 }
}

fn push_material(bytes: &mut Vec<u8>, base_color: [f32; 4], metallic: f32, roughness: f32) {
    for value in base_color {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [metallic, roughness, 0.0, 0.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        // Buffer sizes must be a multiple of four, and a zero-sized buffer is invalid.
        size: (contents.len().max(4) as u64).next_multiple_of(4),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    // Comparisons are against transform entries the constructor writes directly.
    #![allow(clippy::float_cmp)]

    use super::{INSTANCE_STRIDE, MATERIAL_STRIDE, ModelInstance, VERTEX_STRIDE, buffer_layouts};

    #[test]
    fn strides_match_the_declared_attributes() {
        // A stride disagreeing with its attributes does not fail validation in any obvious way; it
        // silently reads each vertex from the wrong offset, which looks like corrupt geometry.
        let [vertex, instance] = buffer_layouts();
        let vertex = vertex.expect("a vertex layout");
        let instance = instance.expect("an instance layout");
        assert_eq!(vertex.array_stride, VERTEX_STRIDE as u64);
        assert_eq!(instance.array_stride, INSTANCE_STRIDE as u64);

        let last = vertex.attributes.last().expect("vertex attributes");
        assert!(
            last.offset + 4 <= VERTEX_STRIDE as u64,
            "the last vertex attribute must fit inside the stride"
        );
        let last = instance.attributes.last().expect("instance attributes");
        assert!(
            last.offset + 16 <= INSTANCE_STRIDE as u64,
            "the last instance attribute must fit inside the stride"
        );
    }

    #[test]
    fn shader_locations_are_unique_and_contiguous() {
        // The two buffers share one location namespace, so an overlap binds the wrong data rather
        // than failing to build.
        let [vertex, instance] = buffer_layouts();
        let vertex = vertex.expect("a vertex layout");
        let instance = instance.expect("an instance layout");
        let mut locations: Vec<u32> = vertex
            .attributes
            .iter()
            .chain(instance.attributes.iter())
            .map(|attribute| attribute.shader_location)
            .collect();
        locations.sort_unstable();
        let expected: Vec<u32> = (0..9).collect();
        assert_eq!(locations, expected, "locations 0..9, each used once");
    }

    #[test]
    fn a_placement_positions_rotates_and_scales() {
        let instance = ModelInstance::placed([10.0, 20.0, 30.0], 0.0, 2.0);
        // Translation sits in the fourth column.
        assert_eq!(instance.transform[3], [10.0, 20.0, 30.0, 1.0]);
        // Uniform scale on the diagonal, no rotation.
        assert_eq!(instance.transform[0][0], 2.0);
        assert_eq!(instance.transform[1][1], 2.0);
        assert_eq!(instance.transform[2][2], 2.0);
        assert_eq!(instance.tint, [1.0; 4]);
    }

    #[test]
    fn a_quarter_turn_maps_x_onto_y() {
        let instance = ModelInstance::placed([0.0; 3], core::f32::consts::FRAC_PI_2, 1.0);
        // Column 0 is where the model's +X axis ends up.
        let x_axis = instance.transform[0];
        assert!(x_axis[0].abs() < 1.0e-6, "{x_axis:?}");
        assert!((x_axis[1] - 1.0).abs() < 1.0e-6, "{x_axis:?}");
    }

    #[test]
    fn a_tint_replaces_only_the_colour() {
        let instance =
            ModelInstance::placed([1.0, 2.0, 3.0], 0.5, 1.5).with_tint([0.4, 0.5, 0.6, 1.0]);
        assert_eq!(instance.tint, [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(instance.transform[3], [1.0, 2.0, 3.0, 1.0]);
    }

    #[test]
    fn the_material_stride_is_sixteen_byte_aligned() {
        // A storage buffer's array stride must satisfy the element's alignment, and a vec4 needs 16.
        assert_eq!(MATERIAL_STRIDE % 16, 0);
    }
}

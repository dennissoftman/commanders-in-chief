//! Drawing the interface: paint primitives into vertices, and text measurement for the solver.
//!
//! # What this module is and is not
//!
//! Everything that decides *how the interface looks* is in `cic_ui::paint` — which colour a focused
//! button takes, where a checkbox's indicator sits, how far along its track a slider's knob is. This
//! module only turns the resulting list into triangles, and rasterises the strings in it. That split is
//! the same one the rest of the crate keeps between arithmetic and device calls: the decisions are
//! testable without a GPU, and what is left here is a vertex buffer and a draw call.
//!
//! # Why the two halves of a solve meet here
//!
//! [`UiMetrics`] closes a loop the layout solver deliberately left open. `cic-ui` cannot know how wide a
//! label is — that depends on a font, a size and a shaping pass — so it takes the answer through a trait.
//! This is the implementation of that trait, and it is the only place in the engine that knows both the
//! theme's sizes and the font's advances. An `Auto`-sized label is therefore as wide as its text, and the
//! caret in a text entry lands on the character it belongs to.
//!
//! # Why glyph quads are snapped and text runs are not
//!
//! A glyph is rasterised at whole-pixel offsets, so drawing it at a fractional position resamples it and
//! the text goes soft — the single most visible thing that can go wrong with interface text. Each glyph's
//! quad is therefore rounded to whole pixels and given the atlas rectangle's exact extent, which makes
//! the mapping one texel per pixel.
//!
//! The pen *between* glyphs stays fractional, because advances are. Rounding it would accumulate up to a
//! pixel of error per character and make a long string measurably shorter than the caret says it is.
//! Rounding only at the quad keeps each letter crisp and each position within half a pixel of true.

use cic_ui::layout::{Node, Widget};
use cic_ui::paint::{Content, Primitive, TextAlign, Theme};
use cic_ui::solve::Measure;
use cic_ui::{Rect, StringTable, Style};

use crate::text::{Font, GlyphAtlas};

/// Bytes one vertex occupies: two floats of position, two of texture coordinate, four of colour, and a
/// `u32` flag.
const VERTEX_BYTES: usize = (2 + 2 + 4) * 4 + 4;

/// Vertices a fresh renderer's buffer holds before it has to grow.
///
/// Enough for about six hundred quads, which is more than any shell screen produces, so the common case
/// never reallocates. Growth is still implemented because a list is data and a bound that cannot be
/// exceeded is a bound somebody eventually exceeds.
const INITIAL_VERTICES: usize = 4_096;

/// The pixel sizes an atlas has to hold for a theme at a display scale.
///
/// Derived rather than configured, because the theme already states every size the interface draws at and
/// a second list would be one that drifts. A host passes this straight to [`GlyphAtlas::new`].
#[must_use]
pub fn atlas_sizes(theme: &Theme, scale: f32) -> [f32; 3] {
    [
        theme.text_size * scale,
        theme.title_size * scale,
        theme.caption_size * scale,
    ]
}

/// Text measurement for the layout solver, and the only place the theme and the font meet.
#[derive(Debug, Clone, Copy)]
pub struct UiMetrics<'a> {
    font: Font,
    theme: &'a Theme,
    strings: &'a StringTable,
    scale: f32,
}

impl<'a> UiMetrics<'a> {
    /// Builds a measurer for one theme, string table and display scale.
    #[must_use]
    pub const fn new(theme: &'a Theme, strings: &'a StringTable, scale: f32) -> Self {
        Self {
            font: Font::new(),
            theme,
            strings,
            scale,
        }
    }

    /// The typeface in use.
    #[must_use]
    pub const fn font(&self) -> Font {
        self.font
    }

    /// The logical text size a node draws at.
    ///
    /// A label's role decides it; everything else takes the body size, because how a button's label looks
    /// is not a per-node decision.
    #[must_use]
    pub fn size_of(&self, node: &Node) -> f32 {
        if node.widget == Widget::Label {
            self.theme.text_role(node.style).1
        } else {
            self.theme.text_size
        }
    }
}

impl Measure for UiMetrics<'_> {
    fn measure(&self, node: &Node, _available: [f32; 2]) -> [f32; 2] {
        let Some(key) = node.text_key.as_deref() else {
            return [0.0, 0.0];
        };
        let size = self.size_of(node);
        // Resolved against the string table rather than against whatever the host stored on this node:
        // a solve happens before any state is read, and sizing a box to a per-frame value would make the
        // layout jump as a countdown ticked.
        let mut width = self.advance(self.strings.text(key), size);
        if node.widget == Widget::Checkbox {
            // The indicator and its gap are part of the control's own width, so an `Auto` checkbox is
            // wide enough for both rather than clipping its label.
            width += size * 1.2 + self.theme.indicator_gap;
        }
        [width, size]
    }

    fn advance(&self, text: &str, size: f32) -> f32 {
        // Measured in the physical size it will be drawn at and converted back, so multiplying this by
        // the scale reproduces the drawn advance exactly. Measuring logically and scaling afterwards
        // would disagree with the shaper by a fraction of a pixel, and the caret would drift.
        if self.scale <= 0.0 {
            return 0.0;
        }
        self.font.advance(text, size * self.scale) / self.scale
    }
}

/// One vertex, matching `shaders/ui.wgsl`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    colour: [f32; 4],
    textured: u32,
}

impl Vertex {
    /// Appends this vertex's bytes.
    ///
    /// Written by hand rather than through a byte-casting crate, because `unsafe_code` is forbidden at
    /// workspace scope and the whole vertex is nine little-endian words.
    fn write(self, out: &mut Vec<u8>) {
        for value in self.position.into_iter().chain(self.uv).chain(self.colour) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.textured.to_le_bytes());
    }
}

/// One run of vertices sharing a scissor rectangle.
#[derive(Debug, Clone, Copy)]
struct Run {
    clip: Rect,
    from: u32,
    to: u32,
}

/// The vertices and scissor runs one frame's primitives turn into.
///
/// Built without a device, so what a draw list *contains* is testable: how many quads a screen produces,
/// where a glyph landed, that a clip was carried through. Only [`UiRenderer::draw`] needs a GPU.
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    vertices: Vec<u8>,
    count: u32,
    runs: Vec<Run>,
}

impl DrawList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets everything, keeping the allocation for the next frame.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.count = 0;
        self.runs.clear();
    }

    /// How many vertices there are.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.count
    }

    /// Whether there is nothing to draw.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// How many scissor runs the list is split into, which is how many draw calls it costs.
    #[must_use]
    pub fn runs(&self) -> usize {
        self.runs.len()
    }

    /// The vertex bytes, ready to upload.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.vertices
    }

    /// Appends every primitive, rasterising text against an atlas.
    ///
    /// Primitives sharing a scissor rectangle become one run, so a screen with no scrollable container
    /// costs a single draw call however many controls it has. A clip that differs from the previous one
    /// starts a new run rather than being sorted into an existing one: reordering would change what draws
    /// on top of what, and the list's order *is* the depth order.
    pub fn extend(&mut self, primitives: &[Primitive<'_>], atlas: &GlyphAtlas) {
        for primitive in primitives {
            match &primitive.content {
                Content::Fill { rect, colour } => {
                    self.quad(
                        primitive.clip,
                        *rect,
                        [0.0, 0.0, 1.0, 1.0],
                        colour.to_linear(),
                        false,
                    );
                }
                Content::Text {
                    rect,
                    text,
                    colour,
                    size,
                    align,
                } => self.run(
                    primitive.clip,
                    *rect,
                    text,
                    colour.to_linear(),
                    *size,
                    *align,
                    atlas,
                ),
            }
        }
    }

    /// Places one line of text and appends a quad per glyph.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        clip: Rect,
        rect: Rect,
        text: &str,
        colour: [f32; 4],
        size: f32,
        align: TextAlign,
        atlas: &GlyphAtlas,
    ) {
        // The nearest packed size rather than nothing: a host that themed a size it did not pack should
        // see slightly-wrong text instead of a blank screen.
        let Some(size) = atlas.nearest_size(size) else {
            return;
        };
        let font = Font::new();
        let width = font.advance(text, size);
        let mut pen = match align {
            TextAlign::Leading => rect.x,
            TextAlign::Center => rect.x + (rect.width - width) / 2.0,
            TextAlign::Trailing => rect.right() - width,
        };
        // The line box is centred in the node, and glyph offsets are measured from its top because the
        // grid's origin is the top of the ascender.
        let top = rect.y + (rect.height - font.line_height(size)) / 2.0;
        for character in text.chars() {
            if let Some(placed) = atlas.glyph(size, character) {
                // Rounded here and nowhere else: the quad lands on the pixel grid the glyph was
                // rasterised on, so one texel covers one pixel and the letter stays crisp.
                let at = Rect::new(
                    (pen + placed.offset[0]).round(),
                    (top + placed.offset[1]).round(),
                    placed.size[0],
                    placed.size[1],
                );
                self.quad(clip, at, placed.uv, colour, true);
            }
            // The pen stays fractional. Rounding it would accumulate up to a pixel per character and
            // make a long string shorter than the caret claims.
            pen += font.advance(&character.to_string(), size);
        }
    }

    /// Appends two triangles.
    fn quad(&mut self, clip: Rect, rect: Rect, uv: [f32; 4], colour: [f32; 4], textured: bool) {
        if rect.is_empty() || clip.is_empty() || clip.intersection(rect).is_empty() {
            return;
        }
        let textured = u32::from(textured);
        let corners = [
            ([rect.x, rect.y], [uv[0], uv[1]]),
            ([rect.right(), rect.y], [uv[2], uv[1]]),
            ([rect.right(), rect.bottom()], [uv[2], uv[3]]),
            ([rect.x, rect.y], [uv[0], uv[1]]),
            ([rect.right(), rect.bottom()], [uv[2], uv[3]]),
            ([rect.x, rect.bottom()], [uv[0], uv[3]]),
        ];
        let from = self.count;
        for (position, uv) in corners {
            Vertex {
                position,
                uv,
                colour,
                textured,
            }
            .write(&mut self.vertices);
        }
        self.count += 6;
        match self.runs.last_mut() {
            Some(run) if run.clip == clip => run.to = self.count,
            _ => self.runs.push(Run {
                clip,
                from,
                to: self.count,
            }),
        }
    }
}

/// The interface pass: one pipeline, one atlas, one growable vertex buffer.
#[derive(Debug)]
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    viewport: wgpu::Buffer,
    viewport_group: wgpu::BindGroup,
    atlas_layout: wgpu::BindGroupLayout,
    atlas_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    vertices: wgpu::Buffer,
    capacity: usize,
}

impl UiRenderer {
    /// Builds the pass for one target format and one glyph atlas.
    ///
    /// The format is a parameter because the same pass draws into a capture target and into a swapchain,
    /// and those disagree. Both are sRGB-encoded, which is why the colours arriving are linear.
    ///
    /// # Panics
    ///
    /// Panics if the `ui` program does not compose, which a test in [`crate::shader`] already forbids —
    /// the chunk it names is compiled into the binary, so the only route here is deleting it.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        atlas: &GlyphAtlas,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cic-render ui"),
            source: wgpu::ShaderSource::Wgsl(
                crate::shader::compose("ui")
                    .expect("the ui program composes")
                    .into(),
            ),
        });
        let (viewport_layout, viewport, viewport_group) = viewport_bindings(device);
        let atlas_layout = atlas_layout(device);
        let sampler = atlas_sampler(device);
        let atlas_group = upload_atlas(device, queue, &atlas_layout, &sampler, atlas);
        let pipeline = build_pipeline(device, &shader, format, &viewport_layout, &atlas_layout);
        Self {
            pipeline,
            viewport,
            viewport_group,
            atlas_layout,
            atlas_group,
            sampler,
            vertices: vertex_buffer(device, INITIAL_VERTICES),
            capacity: INITIAL_VERTICES,
        }
    }

    /// Replaces the glyph atlas, which is what a change of display scale needs.
    ///
    /// Explicit rather than automatic: an atlas is built for declared sizes, and rebuilding it inside a
    /// draw call would put a texture allocation in the middle of a frame.
    pub fn set_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: &GlyphAtlas) {
        self.atlas_group = upload_atlas(device, queue, &self.atlas_layout, &self.sampler, atlas);
    }

    /// Draws a list into a target, blending over whatever is already there.
    ///
    /// Loads rather than clears, because the interface is drawn over a scene as often as onto nothing and
    /// clearing here would make the second case impossible.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        list: &DrawList,
    ) {
        if list.is_empty() || size[0] == 0 || size[1] == 0 {
            return;
        }
        self.reserve(device, list.len() as usize);
        queue.write_buffer(&self.vertices, 0, list.bytes());
        // The shader divides by this to reach clip space, so it is the surface and not the render
        // resolution: the interface is drawn after any resolve, at the size the window actually is.
        #[allow(clippy::cast_precision_loss)]
        let uniform = [size[0] as f32, size[1] as f32, 0.0, 0.0];
        let mut bytes = Vec::with_capacity(16);
        for value in uniform {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.viewport, 0, &bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cic-render ui pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.viewport_group, &[]);
        pass.set_bind_group(1, &self.atlas_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for run in &list.runs {
            let Some(scissor) = scissor(run.clip, size) else {
                continue;
            };
            pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
            pass.draw(run.from..run.to, 0..1);
        }
    }

    /// Grows the vertex buffer when a list needs more than it holds.
    fn reserve(&mut self, device: &wgpu::Device, vertices: usize) {
        if vertices <= self.capacity {
            return;
        }
        // Doubled until it fits, so a list that grows a little does not reallocate every frame.
        let mut capacity = self.capacity.max(1);
        while capacity < vertices {
            capacity *= 2;
        }
        self.vertices = vertex_buffer(device, capacity);
        self.capacity = capacity;
    }
}

/// The viewport uniform's layout, buffer, and bind group.
fn viewport_bindings(
    device: &wgpu::Device,
) -> (wgpu::BindGroupLayout, wgpu::Buffer, wgpu::BindGroup) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ui viewport layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cic-render ui viewport"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render ui viewport bindings"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (layout, buffer, group)
}

/// The glyph atlas's bind group layout: one texture and one sampler.
fn atlas_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render ui atlas layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The atlas sampler.
///
/// Clamped, so a quad's edge cannot wrap to the far side of the atlas and stripe a glyph with whatever
/// happens to be packed there. Filtered, which costs nothing at the one-texel-per-pixel mapping the
/// shaper produces and keeps text from hardening if it is ever drawn at another scale.
fn atlas_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cic-render ui atlas sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// A vertex buffer holding a given number of vertices.
fn vertex_buffer(device: &wgpu::Device, vertices: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cic-render ui vertices"),
        size: (vertices * VERTEX_BYTES) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The interface pipeline.
fn build_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    viewport_layout: &wgpu::BindGroupLayout,
    atlas_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render ui pipeline layout"),
        bind_group_layouts: &[Some(viewport_layout), Some(atlas_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cic-render ui pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_BYTES as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 8,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 16,
                        shader_location: 2,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Uint32,
                        offset: 32,
                        shader_location: 3,
                    },
                ],
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        // No depth at all. The interface's order *is* the list's order, which is what makes a modal draw
        // over the screen behind it without either of them knowing about the other.
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // Straight alpha, matching how the theme's colours are authored and how the paint layer
                // hands them over. Premultiplying would be one more place for a conversion to be
                // forgotten in.
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Uploads an atlas as a single-channel coverage texture and binds it.
fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    atlas: &GlyphAtlas,
) -> wgpu::BindGroup {
    let extent = wgpu::Extent3d {
        width: atlas.width(),
        height: atlas.height(),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cic-render ui glyph atlas"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // One channel, and *not* sRGB: coverage is a fraction of a pixel a stroke covered, not a colour,
        // so a transfer function applied to it would thin every glyph.
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        atlas.data(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas.width()),
            rows_per_image: Some(atlas.height()),
        },
        extent,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render ui atlas bindings"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// A clip rectangle as a scissor, clamped to the surface.
///
/// `set_scissor_rect` refuses a rectangle that leaves the target, and a layout is explicitly allowed to
/// overflow — so a control pushed off the edge of a small window would otherwise be a validation error
/// rather than a control that is simply not visible.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn scissor(clip: Rect, size: [u32; 2]) -> Option<[u32; 4]> {
    let bounds = Rect::new(0.0, 0.0, size[0] as f32, size[1] as f32);
    let visible = clip.intersection(bounds);
    if visible.is_empty() {
        return None;
    }
    Some([
        visible.x.floor().max(0.0) as u32,
        visible.y.floor().max(0.0) as u32,
        (visible.width.ceil() as u32).min(size[0]),
        (visible.height.ceil() as u32).min(size[1]),
    ])
}

/// Which theme sizes a node's own text would be drawn at, for a caller building an atlas.
///
/// Re-exported convenience over [`atlas_sizes`], kept as a separate function so a host that themes only
/// part of an interface can ask about one node rather than assume the whole set.
#[must_use]
pub fn size_for(theme: &Theme, widget: Widget, style: Option<Style>, scale: f32) -> f32 {
    let logical = if widget == Widget::Label {
        theme.text_role(style).1
    } else {
        theme.text_size
    };
    logical * scale
}

#[cfg(test)]
mod tests {
    // Every figure below is a whole pixel coordinate or a small exact fraction, so an exact comparison is
    // the assertion being made.
    #![allow(clippy::float_cmp)]

    use cic_ui::layout::{FORMAT_VERSION, Layout, Node, Sizing, Widget};
    use cic_ui::paint::{Colour, Content, Painter, Primitive, TextAlign, Theme};
    use cic_ui::solve::{Measure, solve};
    use cic_ui::{Interface, Rect, StringTable, Style, Viewport};

    use super::{DrawList, UiMetrics, VERTEX_BYTES, atlas_sizes, scissor, size_for};
    use crate::text::{Font, GlyphAtlas};

    fn theme() -> Theme {
        Theme::default()
    }

    fn atlas(theme: &Theme) -> GlyphAtlas {
        GlyphAtlas::new(&Font::new(), &atlas_sizes(theme, 1.0))
    }

    #[test]
    fn an_atlas_holds_every_size_the_theme_draws_at() {
        // Derived from the theme rather than configured, so a size the interface uses cannot be one the
        // atlas was not built for.
        let theme = theme();
        let sizes = atlas_sizes(&theme, 2.0);
        assert_eq!(
            sizes,
            [
                theme.text_size * 2.0,
                theme.title_size * 2.0,
                theme.caption_size * 2.0
            ]
        );
        let atlas = atlas(&theme);
        for style in [None, Some(Style::Title), Some(Style::Caption)] {
            let size = size_for(&theme, Widget::Label, style, 1.0);
            assert_eq!(
                atlas.nearest_size(size),
                Some(size),
                "{style:?} draws at a size nobody packed"
            );
        }
    }

    #[test]
    fn a_measured_label_is_as_wide_as_its_own_text() {
        // The loop the solver left open: `cic-ui` cannot know how wide a label is, and this is the
        // implementation it takes the answer from.
        let theme = theme();
        let mut strings = StringTable::new();
        strings.set("menu.title", "Commanders in Chief");
        let metrics = UiMetrics::new(&theme, &strings, 1.0);
        let node = Node {
            widget: Widget::Label,
            style: Some(Style::Title),
            text_key: Some("menu.title".to_owned()),
            ..Node::default()
        };
        let measured = metrics.measure(&node, [1000.0, 1000.0]);
        assert!(measured[0] > 100.0, "a title measured {}", measured[0]);
        assert_eq!(measured[1], theme.title_size);
        // A node with no text has no intrinsic size of its own.
        assert_eq!(
            metrics.measure(&Node::default(), [100.0, 100.0]),
            [0.0, 0.0]
        );
    }

    #[test]
    fn an_advance_scaled_up_matches_what_the_shaper_will_draw() {
        // Measuring logically and scaling afterwards disagrees with the shaper by a fraction of a pixel
        // per character, and the caret drifts along a long string.
        let theme = theme();
        let strings = StringTable::new();
        let font = Font::new();
        for scale in [1.0, 1.5, 2.0] {
            let metrics = UiMetrics::new(&theme, &strings, scale);
            let logical = metrics.advance("Skirmish setup", theme.text_size);
            let physical = font.advance("Skirmish setup", theme.text_size * scale);
            assert!(
                (logical * scale - physical).abs() < 1e-3,
                "at scale {scale}: {} against {physical}",
                logical * scale
            );
        }
        // A degenerate scale reports nothing rather than dividing by it.
        assert_eq!(
            UiMetrics::new(&theme, &strings, 0.0).advance("x", 16.0),
            0.0
        );
    }

    #[test]
    fn a_fill_becomes_one_quad_of_six_vertices() {
        let theme = theme();
        let atlas = atlas(&theme);
        let mut list = DrawList::new();
        list.extend(
            &[Primitive {
                clip: Rect::new(0.0, 0.0, 100.0, 100.0),
                content: Content::Fill {
                    rect: Rect::new(10.0, 10.0, 20.0, 20.0),
                    colour: Colour::rgb(0xff, 0, 0),
                },
            }],
            &atlas,
        );
        assert_eq!(list.len(), 6);
        assert_eq!(list.runs(), 1);
        assert_eq!(list.bytes().len(), 6 * VERTEX_BYTES);
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.runs(), 0);
    }

    #[test]
    fn an_invisible_or_wholly_clipped_quad_costs_nothing() {
        let theme = theme();
        let atlas = atlas(&theme);
        let mut list = DrawList::new();
        for (clip, rect) in [
            // Entirely outside its clip.
            (
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Rect::new(50.0, 50.0, 10.0, 10.0),
            ),
            // No extent of its own.
            (
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Rect::new(0.0, 0.0, 0.0, 10.0),
            ),
            // A clip that collapsed, which is what a scrollable container off screen produces.
            (
                Rect::new(0.0, 0.0, 0.0, 0.0),
                Rect::new(0.0, 0.0, 10.0, 10.0),
            ),
        ] {
            list.extend(
                &[Primitive {
                    clip,
                    content: Content::Fill {
                        rect,
                        colour: Colour::rgb(0xff, 0, 0),
                    },
                }],
                &atlas,
            );
        }
        assert!(list.is_empty(), "{} vertices for nothing", list.len());
    }

    #[test]
    fn primitives_sharing_a_clip_become_one_run_and_a_change_starts_another() {
        // How many runs there are is how many draw calls a frame costs, and a screen with no scrollable
        // container should cost one.
        let theme = theme();
        let atlas = atlas(&theme);
        let outer = Rect::new(0.0, 0.0, 200.0, 200.0);
        let inner = Rect::new(0.0, 0.0, 50.0, 50.0);
        let fill = |clip: Rect| Primitive {
            clip,
            content: Content::Fill {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                colour: Colour::rgb(0xff, 0xff, 0xff),
            },
        };
        let mut list = DrawList::new();
        list.extend(
            &[fill(outer), fill(outer), fill(inner), fill(outer)],
            &atlas,
        );
        assert_eq!(list.runs(), 3, "runs follow the order, they are not sorted");
        assert_eq!(list.len(), 24);
    }

    #[test]
    fn a_text_run_places_one_quad_per_drawable_glyph() {
        // A space has no bitmap, so it advances the pen and emits nothing.
        let theme = theme();
        let atlas = atlas(&theme);
        let mut list = DrawList::new();
        list.extend(
            &[Primitive {
                clip: Rect::new(0.0, 0.0, 400.0, 100.0),
                content: Content::Text {
                    rect: Rect::new(0.0, 0.0, 400.0, 40.0),
                    text: "ab cd",
                    colour: Colour::rgb(0xff, 0xff, 0xff),
                    size: theme.text_size,
                    align: TextAlign::Leading,
                },
            }],
            &atlas,
        );
        assert_eq!(
            list.len(),
            4 * 6,
            "four letters, and the space draws nothing"
        );
    }

    #[test]
    fn text_alignment_moves_the_run_within_its_box() {
        let theme = theme();
        let atlas = atlas(&theme);
        let font = Font::new();
        let width = font.advance("Quit", theme.text_size);
        let first_x = |align| {
            let mut list = DrawList::new();
            list.extend(
                &[Primitive {
                    clip: Rect::new(0.0, 0.0, 400.0, 100.0),
                    content: Content::Text {
                        rect: Rect::new(0.0, 0.0, 200.0, 40.0),
                        text: "Quit",
                        colour: Colour::rgb(0xff, 0xff, 0xff),
                        size: theme.text_size,
                        align,
                    },
                }],
                &atlas,
            );
            f32::from_le_bytes(list.bytes()[0..4].try_into().expect("four bytes"))
        };
        let leading = first_x(TextAlign::Leading);
        let centred = first_x(TextAlign::Center);
        let trailing = first_x(TextAlign::Trailing);
        assert!(
            leading < centred && centred < trailing,
            "{leading} {centred} {trailing}"
        );
        // Compared as shifts of the same run rather than against absolute positions: the first vertex
        // carries the glyph's own left bearing and its antialiasing margin, which cancel this way and
        // would otherwise have to be guessed at.
        assert!(
            (centred - (leading + (200.0 - width) / 2.0)).abs() < 1.0,
            "centring shifted by {}, not by {}",
            centred - leading,
            (200.0 - width) / 2.0
        );
        assert!(
            (trailing - (leading + 200.0 - width)).abs() < 1.0,
            "trailing shifted by {}, not by {}",
            trailing - leading,
            200.0 - width
        );
    }

    #[test]
    fn every_glyph_quad_lands_on_whole_pixels() {
        // The most visible thing that can go wrong with interface text: a glyph rasterised at whole-pixel
        // offsets and drawn at a fractional position is resampled, and the text goes soft.
        let theme = theme();
        let atlas = atlas(&theme);
        let mut list = DrawList::new();
        list.extend(
            &[Primitive {
                // A deliberately fractional box, which is what a solved layout at a fractional scale
                // produces before snapping.
                clip: Rect::new(0.0, 0.0, 400.0, 100.0),
                content: Content::Text {
                    rect: Rect::new(10.5, 7.25, 300.0, 41.0),
                    text: "Settings",
                    colour: Colour::rgb(0xff, 0xff, 0xff),
                    size: theme.text_size,
                    align: TextAlign::Leading,
                },
            }],
            &atlas,
        );
        let bytes = list.bytes();
        for vertex in 0..list.len() as usize {
            let at = vertex * VERTEX_BYTES;
            for axis in 0..2 {
                let value = f32::from_le_bytes(
                    bytes[at + axis * 4..at + axis * 4 + 4]
                        .try_into()
                        .expect("four bytes"),
                );
                assert_eq!(value, value.round(), "vertex {vertex} axis {axis}: {value}");
            }
        }
    }

    #[test]
    fn a_size_the_atlas_does_not_hold_still_draws() {
        // Slightly-wrong text beats a blank screen, and a host that themed a size it did not pack has a
        // bug worth seeing rather than one that hides the whole interface.
        let theme = theme();
        let sparse = GlyphAtlas::new(&Font::new(), &[16.0]);
        let mut list = DrawList::new();
        list.extend(
            &[Primitive {
                clip: Rect::new(0.0, 0.0, 400.0, 100.0),
                content: Content::Text {
                    rect: Rect::new(0.0, 0.0, 300.0, 60.0),
                    text: "Menu",
                    colour: Colour::rgb(0xff, 0xff, 0xff),
                    size: theme.title_size,
                    align: TextAlign::Leading,
                },
            }],
            &sparse,
        );
        assert_eq!(list.len(), 4 * 6);
        // An atlas holding nothing at all draws nothing rather than panicking.
        let mut empty = DrawList::new();
        empty.extend(
            &[Primitive {
                clip: Rect::new(0.0, 0.0, 400.0, 100.0),
                content: Content::Text {
                    rect: Rect::new(0.0, 0.0, 300.0, 60.0),
                    text: "Menu",
                    colour: Colour::rgb(0xff, 0xff, 0xff),
                    size: 16.0,
                    align: TextAlign::Leading,
                },
            }],
            &GlyphAtlas::new(&Font::new(), &[]),
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn a_scissor_is_clamped_to_the_surface_because_a_layout_may_overflow() {
        // `set_scissor_rect` refuses a rectangle that leaves the target, and overflow is explicitly not an
        // error in this layout model -- so a control pushed off a small window must be invisible rather
        // than a validation failure.
        assert_eq!(
            scissor(Rect::new(-20.0, -10.0, 100.0, 100.0), [64, 64]),
            Some([0, 0, 64, 64])
        );
        assert_eq!(
            scissor(Rect::new(10.0, 10.0, 20.0, 20.0), [64, 64]),
            Some([10, 10, 20, 20])
        );
        assert_eq!(scissor(Rect::new(100.0, 100.0, 10.0, 10.0), [64, 64]), None);
        assert_eq!(scissor(Rect::new(0.0, 0.0, 0.0, 10.0), [64, 64]), None);
    }

    #[test]
    fn a_whole_screen_becomes_one_draw_call() {
        // The end-to-end shape of the thing: an authored layout, solved with real metrics, painted, and
        // turned into vertices. A screen with no scrollable container costs one draw call however many
        // controls it has.
        let theme = theme();
        let mut strings = StringTable::new();
        strings.set("menu.title", "Commanders in Chief");
        strings.set("menu.quit", "Quit");
        let layout = Layout {
            format_version: FORMAT_VERSION,
            root: Node {
                style: Some(Style::Card),
                width: Sizing::Fill(1),
                height: Sizing::Fill(1),
                gap: 12.0,
                children: vec![
                    Node {
                        widget: Widget::Label,
                        style: Some(Style::Title),
                        text_key: Some("menu.title".to_owned()),
                        height: Sizing::Fixed(40.0),
                        ..Node::default()
                    },
                    Node {
                        id: Some("quit".to_owned()),
                        widget: Widget::Button,
                        text_key: Some("menu.quit".to_owned()),
                        height: Sizing::Fixed(36.0),
                        ..Node::default()
                    },
                ],
                ..Node::default()
            },
        };
        let viewport = Viewport::new(400, 300, 1.0).expect("viewport");
        let metrics = UiMetrics::new(&theme, &strings, 1.0);
        let solved = solve(&layout, viewport, &metrics);
        let mut interface = Interface::new();
        interface.set_focus(Some("quit"));
        let painted = Painter::new(&theme, &metrics, viewport).paint(&solved, &interface, &strings);
        let mut list = DrawList::new();
        list.extend(&painted, &atlas(&theme));
        assert!(!list.is_empty());
        assert_eq!(list.runs(), 1);
        // Both strings reached the list, so the glyph count is at least their letter count.
        let letters =
            u32::try_from("Commanders in ChiefQuit".len()).expect("a short string counts");
        assert!(
            list.len() >= letters * 6,
            "{} vertices for {letters} letters and the chrome",
            list.len()
        );
    }
}

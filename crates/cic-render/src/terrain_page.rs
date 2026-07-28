//! The GPU side of the terrain virtual-texture cache: page tables, physical pages, and the compose pass.
//!
//! # What this adds to the bookkeeping beside it
//!
//! [`crate::terrain_virtual`] decides *which* pages a view wants and which slot each should occupy. That is
//! arithmetic, and it was deliberately written with no device in sight so it could be tested in isolation —
//! but it had no consumer, so nothing checked that its decisions were expressible on a GPU or that the
//! numbers it produces reach a shader intact. This module is that consumer: it turns a
//! [`VirtualPageUpdate`] into a page-table upload and a compute dispatch.
//!
//! # Why a page is worth composing at all
//!
//! The terrain G-buffer blends up to eight layers per fragment, every fragment, every frame — eight weight
//! samples and eight world-space albedo samples, most of them multiplied by a zero weight. That result is
//! *the same every frame* for a given piece of ground, because it depends only on the terrain data. Baking
//! it moves the cost from per-fragment-per-frame to per-page-once, and it is what lets terrain detail scale
//! past what one texture can hold: a page is composed at a density chosen for how close it is, so the ground
//! under the camera can carry far more texels per metre than a single map-wide texture could afford
//! everywhere.
//!
//! # The state of the wiring
//!
//! The cache composes pages and publishes a page table. The terrain G-buffer does **not** yet sample it —
//! that is the next step, and it is deliberately a separate one: switching the fragment path over changes
//! every terrain frame, and the honest order is to prove the composition first. What is here is verified by
//! reading pages back and checking the two properties that decide whether the cache is usable at all: that a
//! page over known ground holds that ground's surface, and that a page's border matches the interior of the
//! page beside it. See the tests in `tests/terrain_render.rs`.

use crate::RenderError;
use crate::detail::TerrainDetailRequest;
use crate::terrain::TerrainRenderer;
use crate::terrain_virtual::{
    VIRTUAL_PAGE_EXTENT, VIRTUAL_PAGE_LAYERS, VirtualPageCache, VirtualPageView,
};

/// The format a composed page is stored in.
///
/// `Rgba8Unorm` rather than its sRGB sibling because WebGPU has no sRGB *storage* format, and a storage
/// binding is what a compute shader writes through. The compose shader therefore applies the transfer
/// function itself — see `terrain_virtual.wgsl` — so the stored bytes are sRGB-encoded colour in `rgb` and
/// linear roughness in `a`, which is the same content the G-buffer's own albedo target holds.
pub const PAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The format a page table is stored in.
///
/// One unsigned integer per page: zero for "not resident", and otherwise the physical layer plus one. The
/// bias is what makes zero mean absent — a freshly cleared table is a correct empty one, rather than a table
/// claiming every page lives in layer 0.
pub const PAGE_TABLE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// Bytes one page job occupies in the compose pass's storage buffer: two `vec4<u32>`.
const JOB_BYTES: usize = 32;

/// How many page levels the cache holds, matching [`VirtualPageCache`].
const LEVELS: usize = 2;

/// The virtual-texture cache: the physical pages, the tables that index them, and the pass that fills them.
///
/// # Why the layer budget is a parameter
///
/// [`VIRTUAL_PAGE_LAYERS`] pages of [`VIRTUAL_PAGE_EXTENT`] squared is 71 MB, which is a reasonable figure
/// for a shipping cache and an unreasonable one for a test that wants to read a page back. A budget is also
/// the honest shape for this: a cache size is a memory decision, and the residency logic already treats
/// running out of slots as a normal condition rather than an error.
pub struct TerrainPageCache {
    residency: VirtualPageCache,
    pages: wgpu::Texture,
    page_view: wgpu::TextureView,
    tables: [wgpu::Texture; LEVELS],
    table_views: [wgpu::TextureView; LEVELS],
    jobs: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    layers: u32,
    /// How many pages the last update composed, so a caller can see the cache warming rather than guessing.
    composed: u32,
}

impl TerrainPageCache {
    /// Allocates a cache for one terrain, holding at most `layers` pages.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] when `layers` is zero or exceeds [`VIRTUAL_PAGE_LAYERS`].
    ///
    /// # Panics
    ///
    /// Panics if `terrain_virtual` is not a declared shader program, which the shader tests also compose and
    /// validate — so that is a build-time mistake caught by `cargo test` rather than a condition a caller
    /// can reach.
    pub fn new(
        context: &crate::GpuContext,
        terrain: &TerrainRenderer,
        layers: u32,
    ) -> Result<Self, RenderError> {
        let budget = u32::try_from(VIRTUAL_PAGE_LAYERS).unwrap_or(u32::MAX);
        if layers == 0 || layers > budget {
            return Err(RenderError::InvalidTexture);
        }
        let device = context.device();
        let (width, height) = terrain.cell_size();
        let residency = VirtualPageCache::with_layers(
            [width, height],
            usize::try_from(layers).unwrap_or(VIRTUAL_PAGE_LAYERS),
        );

        let pages = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cic-render terrain pages"),
            size: wgpu::Extent3d {
                width: VIRTUAL_PAGE_EXTENT,
                height: VIRTUAL_PAGE_EXTENT,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PAGE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // A write-only array view for the compose pass, which is what a storage binding needs.
        let page_view = pages.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render terrain pages"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let table = |level: usize| {
            let size = residency.table_size(level);
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cic-render terrain page table"),
                size: wgpu::Extent3d {
                    // Floored at one: a terrain smaller than one page still needs a table to hold its
                    // single entry, and a zero-sized texture is invalid.
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: PAGE_TABLE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let tables = [table(0), table(1)];
        let table_views = [
            tables[0].create_view(&wgpu::TextureViewDescriptor::default()),
            tables[1].create_view(&wgpu::TextureViewDescriptor::default()),
        ];

        // Sized for the whole budget, because a single update may replace every resident page — which is
        // exactly what happens on the first frame, and on a jump cut to another part of the map.
        let jobs = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cic-render terrain page jobs"),
            size: u64::from(layers) * JOB_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = build_layout(device);
        let (pipeline, group) = build_compose(device, &layout, terrain, &jobs, &page_view);

        Ok(Self {
            residency,
            pages,
            page_view,
            tables,
            table_views,
            jobs,
            pipeline,
            layout,
            group,
            layers,
            composed: 0,
        })
    }
}

/// Builds the compose pipeline and the group it reads through.
///
/// Separate from [`TerrainPageCache::new`] because the two halves answer different questions — what the
/// cache holds, and what fills it — and because a constructor that allocates six resources and builds a
/// pipeline hides both behind its own length.
fn build_compose(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    terrain: &TerrainRenderer,
    jobs: &wgpu::Buffer,
    page_view: &wgpu::TextureView,
) -> (wgpu::ComputePipeline, wgpu::BindGroup) {
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cic-render terrain compose"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: terrain.uniform_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(terrain.weight_view()),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(terrain.weight_sampler()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(terrain.layer_albedo().view()),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(terrain.albedo_sampler()),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: jobs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(page_view),
            },
        ],
    });

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cic-render terrain_virtual"),
        source: wgpu::ShaderSource::Wgsl(
            crate::shader::compose("terrain_virtual")
                .expect("terrain_virtual is a declared program")
                .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render terrain compose layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cic-render terrain compose"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("compose_page"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    (pipeline, group)
}

impl TerrainPageCache {
    /// Stages the pages a view wants, composing any that were not already resident.
    ///
    /// Returns how many pages this call composed. Zero is the steady state and the point of a cache: a
    /// camera that has not moved far asks for pages that are already there.
    ///
    /// The compose pass runs on its own submission rather than being folded into the frame's encoder. That is
    /// deliberate: it is not part of drawing a frame, it does not have to happen before the frame that
    /// requested it, and a cache miss should not stall a present.
    pub fn update(
        &mut self,
        context: &crate::GpuContext,
        requests: &[TerrainDetailRequest],
        view: VirtualPageView,
    ) -> u32 {
        let update = self.residency.update(requests, view);
        self.composed = u32::try_from(update.jobs.len()).unwrap_or(u32::MAX);
        if update.tables_changed {
            self.upload_tables(context);
        }
        if update.jobs.is_empty() {
            return 0;
        }

        let mut bytes = Vec::with_capacity(update.jobs.len() * JOB_BYTES);
        for job in &update.jobs {
            job.write_bytes(&mut bytes);
        }
        debug_assert_eq!(
            bytes.len(),
            update.jobs.len() * JOB_BYTES,
            "job record size drifted from the shader's PageJob"
        );
        context.queue().write_buffer(&self.jobs, 0, &bytes);

        let mut encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("cic-render terrain compose"),
                });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cic-render terrain compose"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            // One dispatch for every job at once, with the job index as the third dimension — so a hundred
            // page misses is one dispatch rather than a hundred.
            let groups = VIRTUAL_PAGE_EXTENT.div_ceil(8);
            pass.dispatch_workgroups(groups, groups, self.composed);
        }
        context.queue().submit([encoder.finish()]);
        self.composed
    }

    /// Uploads both page tables from the residency map.
    ///
    /// Whole tables rather than the entries that changed. A table is one integer per page — a 1025-cell
    /// terrain's finer table is 129 by 129, half a kilobyte — so tracking which entries moved would cost
    /// more bookkeeping than the upload it saves.
    fn upload_tables(&self, context: &crate::GpuContext) {
        for level in 0..LEVELS {
            let size = self.residency.table_size(level);
            let (width, height) = (size[0].max(1), size[1].max(1));
            let mut bytes = Vec::with_capacity((width * height) as usize * 4);
            let resident = self.residency.table(level);
            for index in 0..(width * height) as usize {
                // Absent entries pad with zero, which the shader reads as "not resident". The table texture
                // is floored at one on each axis and the residency map's may be smaller, so this is the
                // ordinary case for a terrain under one page rather than a guard against a bug.
                bytes.extend_from_slice(&resident.get(index).copied().unwrap_or(0).to_le_bytes());
            }
            context.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.tables[level],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Returns how many pages the last [`Self::update`] composed.
    #[must_use]
    pub const fn composed(&self) -> u32 {
        self.composed
    }

    /// Returns how many pages the cache can hold.
    #[must_use]
    pub const fn layer_count(&self) -> u32 {
        self.layers
    }

    /// Returns the physical page array, for a pass that samples it.
    #[must_use]
    pub const fn page_view(&self) -> &wgpu::TextureView {
        &self.page_view
    }

    /// Returns one level's page table, for a pass that resolves a cell to a page.
    #[must_use]
    pub fn table_view(&self, level: usize) -> Option<&wgpu::TextureView> {
        self.table_views.get(level)
    }

    /// Returns the compose pass's bind group layout, so a caller can see what it reads.
    #[must_use]
    pub const fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Returns the physical page texture, for a readback.
    #[must_use]
    pub const fn pages(&self) -> &wgpu::Texture {
        &self.pages
    }
}

impl std::fmt::Debug for TerrainPageCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerrainPageCache")
            .field("layers", &self.layers)
            .field("composed", &self.composed)
            .finish_non_exhaustive()
    }
}

/// The compose pass's bind group layout.
///
/// Fixed here rather than derived from the shader, on the same reasoning as the terrain layout: a binding
/// that drifts out of agreement fails at pipeline creation rather than composing something wrong.
fn build_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let array_texture = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    };
    let filtering = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render terrain compose layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            array_texture(1),
            filtering(2),
            array_texture(3),
            filtering(4),
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: PAGE_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
                count: None,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::{JOB_BYTES, LEVELS};
    use crate::shader;
    use crate::terrain_virtual::{VIRTUAL_PAGE_BORDER, VirtualPageJob};

    #[test]
    fn a_job_record_is_the_size_the_shader_declares() {
        // Two `vec4<u32>`. A record shorter than the shader's stride reads the next job's fields as this
        // one's, which composes plausible pages at the wrong coordinates.
        let mut bytes = Vec::new();
        VirtualPageJob {
            origin: [1, 2],
            cells_per_page: 8,
            physical_layer: 3,
            pixels_per_cell: 32,
        }
        .write_bytes(&mut bytes);
        assert_eq!(bytes.len(), JOB_BYTES);
    }

    #[test]
    fn the_shader_agrees_about_the_page_border() {
        // Duplicated across the language boundary because the shader needs it as a literal. A disagreement
        // shifts every page by the difference, which reads as terrain sliding under itself.
        let source = shader::chunk("terrain_virtual").expect("the chunk exists");
        assert!(
            source.contains(&format!("const PAGE_BORDER: u32 = {VIRTUAL_PAGE_BORDER}u;")),
            "the shader's page border must match VIRTUAL_PAGE_BORDER"
        );
    }

    #[test]
    fn the_level_count_matches_the_residency_map() {
        // The cache allocates one table per level, and the residency map indexes them by the same number.
        // A mismatch is a panic on a slice index rather than a wrong image, which is why it is worth one
        // line here.
        let cache = crate::terrain_virtual::VirtualPageCache::new([64, 64]);
        for level in 0..LEVELS {
            let size = cache.table_size(level);
            assert!(size[0] > 0 && size[1] > 0, "level {level} has no table");
        }
    }
}

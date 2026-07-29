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
//! The cache composes pages, publishes a page table, and the terrain G-buffer samples it once
//! [`TerrainRenderer::attach_pages`] has been called. The two paths agree: measured over the whole frame, a
//! mean channel difference of **0.004** and a worst case of **5** eight-bit steps, which is the quantisation
//! a page store costs plus the mip level the G-buffer now picks for itself.
//!
//! Three things are verified separately, because no one of them implies the others. That a page over known
//! ground holds that ground's surface, and that a page's border matches the interior of the page beside it,
//! are read back from the pages themselves — `tests/terrain_render.rs` — because a rendered comparison
//! cannot isolate either. That the *fragment* path agrees with the direct blend, and that a cell with no
//! resident page falls back to it, are rendered — `tests/deferred_render.rs`.
//!
//! The forward pass deliberately has no page lookup. It draws terrain alone in one pass, which is the case a
//! cache has nothing to offer.
//!
//! # The mip chain
//!
//! A page used to hold one density, which made the cache correct and not yet *better*: the G-buffer's
//! fallback samples an albedo array that has a chain, so a single-level page aliased at a shallow angle
//! where the direct blend did not — the terrain would have looked worse on exactly the ground a virtual
//! texture is for. Each update therefore reduces the pages it composed, one compute pass per level, and the
//! G-buffer picks a level from screen-space derivatives.
//!
//! Two things about that are not free choices. The chain's *depth* is set by the border rather than by the
//! interior, because every reduction halves the border and a filtered tap needs a whole texel of it — see
//! [`crate::terrain_virtual::VIRTUAL_PAGE_BORDER`], which is why that constant is eight and not four. And
//! the reduction averages in **linear light**, because a page stores sRGB-encoded colour and averaging
//! encoded bytes makes a high-contrast page pale as it recedes.
//!
//! # What is still missing
//!
//! **A view-driven request.** Nothing derives a [`TerrainDetailRequest`] from a camera yet, so a caller has
//! to say which cells it wants at which density. The residency map already ranks by projected size, so this
//! is a small function rather than a design.

use crate::RenderError;
use crate::detail::TerrainDetailRequest;
use crate::terrain::TerrainRenderer;
use crate::terrain_virtual::{
    VIRTUAL_PAGE_EXTENT, VIRTUAL_PAGE_LAYERS, VIRTUAL_PAGE_MIPS, VirtualPageCache, VirtualPageView,
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
/// [`VIRTUAL_PAGE_LAYERS`] pages of [`VIRTUAL_PAGE_EXTENT`] squared is 76 MB, and the mip chain takes that to
/// 101 MB — a reasonable figure for a shipping cache and an unreasonable one for a test that wants to read a
/// page back. A budget is also the honest shape for this: a cache size is a memory decision, and the residency
/// logic already treats running out of slots as a normal condition rather than an error.
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
    /// The reduce pass, and one bind group per reduction: level 0 to 1, then 1 to 2, and so on.
    reduce: wgpu::ComputePipeline,
    reduce_groups: Vec<wgpu::BindGroup>,
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
            mip_level_count: VIRTUAL_PAGE_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PAGE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // The whole chain, for the G-buffer to sample: it picks a level from screen-space derivatives, so it
        // has to be able to see every level from one view.
        let page_view = pages.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cic-render terrain pages"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // And one view per level, because a storage binding has exactly one mip level — which is what the
        // compose pass writes the base through, and what each reduction reads and writes.
        let level_views = (0..VIRTUAL_PAGE_MIPS)
            .map(|level| {
                pages.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("cic-render terrain page level"),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();

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
        let (pipeline, group) = build_compose(device, &layout, terrain, &jobs, &level_views[0]);
        let (reduce, reduce_groups) = build_reduce(device, &jobs, &level_views);

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
            reduce,
            reduce_groups,
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
    base_view: &wgpu::TextureView,
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
                resource: wgpu::BindingResource::TextureView(base_view),
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

/// Builds the reduce pipeline and one bind group per reduction.
///
/// # Why one group per level rather than one group and a level index
///
/// A storage binding is a single mip level, so the destination is a different *view* at every step and a
/// uniform saying "level 2" could not name it. There are three groups for a four-level chain, they are built
/// once, and the alternative — rebuilding a group per level per update — would allocate on every cache miss.
///
/// # Panics
///
/// Panics if `terrain_reduce` is not a declared shader program, which is a build-time mistake the shader
/// tests catch rather than a condition a caller can reach.
fn build_reduce(
    device: &wgpu::Device,
    jobs: &wgpu::Buffer,
    level_views: &[wgpu::TextureView],
) -> (wgpu::ComputePipeline, Vec<wgpu::BindGroup>) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cic-render terrain reduce layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    // Unfilterable, and it is not a compromise: the reduction fetches the exact four texels
                    // its output covers by integer coordinate, so there is no sampler in this pass at all.
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: PAGE_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
                count: None,
            },
        ],
    });

    let groups = level_views
        .windows(2)
        .map(|pair| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cic-render terrain reduce"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: jobs.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&pair[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&pair[1]),
                    },
                ],
            })
        })
        .collect();

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cic-render terrain_reduce"),
        source: wgpu::ShaderSource::Wgsl(
            crate::shader::compose("terrain_reduce")
                .expect("terrain_reduce is a declared program")
                .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cic-render terrain reduce layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cic-render terrain reduce"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("reduce_page"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    (pipeline, groups)
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
        // Then the chain, one pass per level, each reading what the one before it wrote. Separate passes
        // rather than separate dispatches in one: the dependency is the whole point, and a pass boundary is
        // where it is unambiguous. Only the pages this update composed are reduced, so the steady state — a
        // camera that has not moved far — costs nothing here either.
        for (step, group) in self.reduce_groups.iter().enumerate() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cic-render terrain reduce"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce);
            pass.set_bind_group(0, group, &[]);
            // The level being *written*, which is one past the step's index.
            let extent = VIRTUAL_PAGE_EXTENT >> u32::try_from(step + 1).unwrap_or(u32::MAX);
            let groups = extent.div_ceil(8);
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

    /// Returns the physical page array, whole mip chain included, for a pass that samples it.
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
    use crate::terrain_virtual::{
        VIRTUAL_PAGE_BORDER, VIRTUAL_PAGE_EXTENT, VIRTUAL_PAGE_INTERIOR, VirtualPageJob,
    };

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
    fn the_gbuffer_agrees_about_every_page_dimension() {
        // Five figures duplicated into `terrain_gbuffer.wgsl` so a fragment can find its page without a
        // uniform read per lookup. A disagreement in any of them does not fail to compile: it samples the
        // right page at the wrong place, which reads as terrain sliding under itself as the camera moves.
        let source = shader::chunk("terrain_gbuffer").expect("the chunk exists");
        let border = f64::from(VIRTUAL_PAGE_BORDER);
        let extent = f64::from(VIRTUAL_PAGE_EXTENT);
        for (name, value) in [
            ("PAGE_BORDER", border),
            ("PAGE_EXTENT", extent),
            // The chain's depth, which bounds the level the G-buffer may ask for. Declaring one level more
            // than the cache allocates would not fail: `textureSampleLevel` clamps, so the deepest ground
            // would silently sample the level above the one it wanted.
            (
                "PAGE_MIPS",
                f64::from(crate::terrain_virtual::VIRTUAL_PAGE_MIPS),
            ),
            // The two levels the residency map decomposes into. Their product with the density beside them
            // is the page interior, which is what makes both levels the same memory.
            ("PAGE_FINE_CELLS", 8.0),
            ("PAGE_FINE_DENSITY", 32.0),
            ("PAGE_COARSE_CELLS", 16.0),
            ("PAGE_COARSE_DENSITY", 16.0),
        ] {
            let declaration = format!("const {name}: f32 = {value:?};");
            assert!(
                source.contains(&declaration),
                "the G-buffer must declare `{declaration}`"
            );
        }
        // And the two levels really do fill the same interior, which is the invariant the four cell and
        // density figures encode between them. Compared as integers, because that is what they are — the
        // figures above are `f32` in the shader only because a coordinate is.
        assert_eq!(8 * 32, VIRTUAL_PAGE_INTERIOR);
        assert_eq!(16 * 16, VIRTUAL_PAGE_INTERIOR);
        assert_eq!(
            VIRTUAL_PAGE_BORDER * 2 + VIRTUAL_PAGE_INTERIOR,
            VIRTUAL_PAGE_EXTENT
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

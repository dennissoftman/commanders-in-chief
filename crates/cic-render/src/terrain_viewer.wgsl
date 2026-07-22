struct Camera {
    view_projection: mat4x4<f32>,
    camera_position_time: vec4<f32>,
    viewport: vec4<f32>,
    detail_fade_caustics: vec4<f32>,
    water_material: vec4<f32>,
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_position: vec3<f32>,
}

struct GBufferOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position: vec4<f32>,
}

@group(0) @binding(0) var terrain_texture: texture_2d<f32>;
@group(0) @binding(1) var terrain_sampler: sampler;
@group(0) @binding(2) var<uniform> camera: Camera;
@group(0) @binding(3) var virtual_pages: texture_2d_array<f32>;
@group(0) @binding(4) var fine_page_table: texture_2d<u32>;
@group(0) @binding(5) var coarse_page_table: texture_2d<u32>;

struct VirtualConfig {
    cell_source: vec4<u32>,
    cache: vec4<u32>,
}

@group(0) @binding(6) var<uniform> virtual_config: VirtualConfig;

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    output.world_position = input.position;
    return output;
}

fn page_sample(
    table: texture_2d<u32>,
    cell_position: vec2<f32>,
    cells_per_page: u32,
) -> vec4<f32> {
    let table_size = textureDimensions(table);
    let page = min(vec2<u32>(floor(cell_position / f32(cells_per_page))), table_size - vec2<u32>(1u));
    let mapping = textureLoad(table, vec2<i32>(page), 0).x;
    if (mapping == 0u) { return vec4<f32>(0.0); }
    let pixels_per_cell = f32(virtual_config.cache.w) / f32(cells_per_page);
    let local = cell_position - vec2<f32>(page * cells_per_page);
    let page_pixel = vec2<f32>(
        f32(virtual_config.cache.z) + local.x * pixels_per_cell,
        f32(virtual_config.cache.z) + (f32(cells_per_page) - local.y) * pixels_per_cell,
    );
    let coordinate = clamp(
        page_pixel / f32(virtual_config.cache.y),
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
    let cell_dx = dpdx(cell_position);
    let cell_dy = dpdy(cell_position);
    let scale = pixels_per_cell / f32(virtual_config.cache.y);
    let gradient_x = vec2<f32>(cell_dx.x, -cell_dx.y) * scale;
    let gradient_y = vec2<f32>(cell_dy.x, -cell_dy.y) * scale;
    let color = textureSampleGrad(
        virtual_pages,
        terrain_sampler,
        coordinate,
        i32(mapping - 1u),
        gradient_x,
        gradient_y,
    );
    return color;
}

fn terrain_sample(uv: vec2<f32>) -> vec4<f32> {
    let cell_position = clamp(
        vec2<f32>(uv.x, 1.0 - uv.y) * vec2<f32>(virtual_config.cell_source.xy),
        vec2<f32>(0.0),
        vec2<f32>(virtual_config.cell_source.xy) - vec2<f32>(0.0001),
    );
    let fine = page_sample(fine_page_table, cell_position, 8u);
    if (fine.a > 0.5) { return fine; }
    let coarse = page_sample(coarse_page_table, cell_position, 16u);
    if (coarse.a > 0.5) { return coarse; }
    return textureSample(terrain_texture, terrain_sampler, uv);
}

fn surface_output(input: VertexOutput) -> GBufferOutput {
    var normal = normalize(cross(dpdx(input.world_position), dpdy(input.world_position)));
    if (normal.z < 0.0) { normal = -normal; }
    var output: GBufferOutput;
    output.albedo = terrain_sample(input.uv);
    output.normal_roughness = vec4<f32>(normal, 0.88);
    output.world_position = vec4<f32>(input.world_position, 1.0);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> GBufferOutput {
    return surface_output(input);
}

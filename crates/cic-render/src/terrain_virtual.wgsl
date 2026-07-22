struct Material {
    parameters: vec4<u32>,
    u: vec4<f32>,
    v: vec4<f32>,
}

struct Cell {
    base: Material,
    primary: Material,
    extra: Material,
    flags: vec4<u32>,
}

struct PageJob {
    page: vec4<u32>,
    detail: vec4<u32>,
}

struct VirtualConfig {
    cell_source: vec4<u32>,
    cache: vec4<u32>,
}

@group(0) @binding(0) var source_tiles: texture_2d<f32>;
@group(0) @binding(1) var edge_tiles: texture_2d<f32>;
@group(0) @binding(2) var macro_lattice: texture_2d<u32>;
@group(0) @binding(3) var<storage, read> cells: array<Cell>;
@group(0) @binding(4) var<storage, read> compose_jobs: array<PageJob>;
@group(0) @binding(5) var composed_color: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(6) var composed_edge: texture_storage_2d_array<rgba8unorm, write>;
@group(0) @binding(7) var<uniform> config: VirtualConfig;

fn material_sample(material: Material, local_top: vec2<f32>) -> vec4<f32> {
    let top = vec2<f32>(
        mix(material.u.w, material.u.z, local_top.x),
        mix(material.v.w, material.v.z, local_top.x),
    );
    let bottom = vec2<f32>(
        mix(material.u.x, material.u.y, local_top.x),
        mix(material.v.x, material.v.y, local_top.x),
    );
    let uv = clamp(mix(top, bottom, local_top.y), vec2<f32>(0.0), vec2<f32>(1.0));
    let class_width = material.parameters.y;
    let class_extent = class_width * 64u;
    let pixel = vec2<u32>(round(uv * f32(class_extent - 1u)));
    let tile_from_top = pixel / 64u;
    let tile_from_bottom = vec2<u32>(tile_from_top.x, class_width - 1u - tile_from_top.y);
    let slot = material.parameters.x + tile_from_bottom.y * class_width + tile_from_bottom.x;
    let atlas_tile = vec2<u32>(slot % config.cell_source.z, slot / config.cell_source.z);
    let atlas_pixel = atlas_tile * 64u + pixel % 64u;
    return textureLoad(source_tiles, vec2<i32>(atlas_pixel), 0);
}

fn mask_alpha(code: u32, local_top: vec2<f32>) -> f32 {
    if ((code & 1u) == 0u) { return 0.0; }
    let orientation = (code >> 1u) & 3u;
    let inverted = ((code >> 3u) & 1u) != 0u;
    let long_diagonal = ((code >> 4u) & 1u) != 0u;
    var h = local_top.x * 63.0;
    var v = (1.0 - local_top.y) * 63.0;
    var alpha = 255.0;
    if (orientation == 0u) {
        if (!inverted) { h = 63.0 - h; }
        alpha = alpha * h / 63.0;
    } else if (orientation == 1u) {
        if (!inverted) { v = 63.0 - v; }
        alpha = alpha * v / 63.0;
    } else if (orientation == 2u) {
        h = 63.0 - h;
        if (!inverted) { v = 63.0 - v; }
        v += h;
        if (long_diagonal) { v -= 64.0; }
        alpha = alpha * v / 63.0;
    } else {
        if (!inverted) { v = 63.0 - v; }
        v += h;
        if (long_diagonal) { v -= 64.0; }
        alpha = alpha * v / 63.0;
    }
    return (255.0 - clamp(alpha, 0.0, 255.0)) / 255.0;
}

fn edge_sample(slot: u32, local_top: vec2<f32>) -> vec4<f32> {
    let atlas_tile = vec2<u32>(slot % config.cell_source.w, slot / config.cell_source.w);
    let pixel = vec2<u32>(round(clamp(local_top, vec2<f32>(0.0), vec2<f32>(1.0)) * 31.0));
    return textureLoad(edge_tiles, vec2<i32>(atlas_tile * 32u + pixel), 0);
}

fn srgb_to_linear(value: vec3<f32>) -> vec3<f32> {
    let low = value / 12.92;
    let high = pow((value + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, value <= vec3<f32>(0.04045));
}

fn macro_factor(source_position: vec2<f32>) -> f32 {
    if (config.cache.x == 0u) { return 1.0; }
    let lattice_position = max(source_position, vec2<f32>(0.0)) / 8.0;
    let lattice = vec2<u32>(floor(lattice_position));
    let fraction = fract(lattice_position);
    let smoothed = fraction * fraction * (vec2<f32>(3.0) - 2.0 * fraction);
    let top = mix(
        f32(textureLoad(macro_lattice, vec2<i32>(lattice), 0).x),
        f32(textureLoad(macro_lattice, vec2<i32>(lattice + vec2<u32>(1u, 0u)), 0).x),
        smoothed.x,
    );
    let bottom = mix(
        f32(textureLoad(macro_lattice, vec2<i32>(lattice + vec2<u32>(0u, 1u)), 0).x),
        f32(textureLoad(macro_lattice, vec2<i32>(lattice + vec2<u32>(1u, 1u)), 0).x),
        smoothed.x,
    );
    return (242.0 + mix(top, bottom, smoothed.y) * 28.0 / 255.0) / 256.0;
}

@compute @workgroup_size(8, 8, 1)
fn compose_page(@builtin(global_invocation_id) invocation: vec3<u32>) {
    if (invocation.x >= config.cache.y || invocation.y >= config.cache.y) { return; }
    let job = compose_jobs[invocation.z];
    let pixels_per_cell = f32(job.detail.x);
    let page_from_top = (vec2<f32>(invocation.xy) - f32(config.cache.z) + vec2<f32>(0.5)) / pixels_per_cell;
    let source_position = vec2<f32>(
        f32(job.page.x) + page_from_top.x,
        f32(job.page.y + job.page.z) - page_from_top.y,
    );
    let bounded = clamp(
        source_position,
        vec2<f32>(0.0001),
        vec2<f32>(config.cell_source.xy) - vec2<f32>(0.0001),
    );
    let cell_position = vec2<u32>(floor(bounded));
    let local_top = vec2<f32>(fract(bounded.x), 1.0 - fract(bounded.y));
    let cell = cells[cell_position.y * config.cell_source.x + cell_position.x];
    var color = material_sample(cell.base, local_top);
    if (cell.primary.parameters.z != 0u) {
        let layer = material_sample(cell.primary, local_top);
        color = mix(color, layer, mask_alpha(cell.flags.x, local_top));
    }
    if (cell.extra.parameters.z != 0u) {
        let layer = material_sample(cell.extra, local_top);
        color = mix(color, layer, mask_alpha(cell.flags.y, local_top));
    }
    color = vec4<f32>(color.rgb * macro_factor(bounded), color.a);
    color = vec4<f32>(srgb_to_linear(clamp(color.rgb, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
    var edge = edge_sample(cell.flags.z, local_top);
    edge = vec4<f32>(srgb_to_linear(edge.rgb), edge.a);
    textureStore(composed_color, vec2<i32>(invocation.xy), i32(job.page.w), color);
    textureStore(composed_edge, vec2<i32>(invocation.xy), i32(job.page.w), edge);
}

@group(1) @binding(0) var previous_color: texture_2d_array<f32>;
@group(1) @binding(1) var previous_edge: texture_2d_array<f32>;
@group(1) @binding(2) var mip_color: texture_storage_2d_array<rgba8unorm, write>;
@group(1) @binding(3) var mip_edge: texture_storage_2d_array<rgba8unorm, write>;
@group(1) @binding(4) var<storage, read> mip_jobs: array<PageJob>;

@compute @workgroup_size(8, 8, 1)
fn downsample_page(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let size = textureDimensions(mip_color);
    if (invocation.x >= size.x || invocation.y >= size.y) { return; }
    let layer = i32(mip_jobs[invocation.z].page.w);
    let source_size = textureDimensions(previous_color);
    let origin = invocation.xy * 2u;
    var color = vec4<f32>(0.0);
    var edge_rgb = vec3<f32>(0.0);
    var edge_alpha = 0.0;
    for (var y = 0u; y < 2u; y++) {
        for (var x = 0u; x < 2u; x++) {
            let coordinate = min(origin + vec2<u32>(x, y), source_size - vec2<u32>(1u));
            color += textureLoad(previous_color, vec2<i32>(coordinate), layer, 0);
            let edge = textureLoad(previous_edge, vec2<i32>(coordinate), layer, 0);
            edge_rgb += edge.rgb * edge.a;
            edge_alpha += edge.a;
        }
    }
    color *= 0.25;
    let output_alpha = edge_alpha * 0.25;
    let output_rgb = select(
        vec3<f32>(0.0),
        edge_rgb / max(edge_alpha, 0.0001),
        edge_alpha > 0.0001,
    );
    textureStore(mip_color, vec2<i32>(invocation.xy), layer, color);
    textureStore(mip_edge, vec2<i32>(invocation.xy), layer, vec4<f32>(output_rgb, output_alpha));
}

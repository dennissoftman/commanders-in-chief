// The deferred lighting resolve: one fullscreen pass over the G-buffer.
//
// A composition chunk. Requires `scene.wgsl`, `shadow.wgsl` and `atmosphere.wgsl`.
//
// Lighting is deferred because the shadow and occlusion terms are screen-space: each needs the whole
// depth buffer resolved before any pixel can be lit, which a forward pass cannot provide.

// Peak highlight strength, reached only by a fully smooth material.
const SPECULAR_STRENGTH: f32 = 0.06;

@fragment
fn lighting_fragment(input: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(input.position.xy);
    let coverage = textureLoad(g_coverage, pixel, 0).r;
    if (coverage < 0.5) {
        // Pixel y grows downward, so this runs from the zenith at the top of the frame to the horizon
        // at the bottom.
        let horizon = clamp(input.position.y / camera.viewport.y, 0.0, 1.0);
        return vec4<f32>(mix(SKY_ZENITH, SKY_HORIZON, horizon), 1.0);
    }
    let world = world_at(pixel);
    let albedo = textureLoad(g_albedo, pixel, 0).rgb;
    let normal_roughness = textureLoad(g_normal, pixel, 0);
    let normal = normalize(normal_roughness.xyz);
    let view_direction = normalize(camera.camera_position.xyz - world);
    var primary_visibility = shadow_visibility(world, normal);
    // Fade toward fully lit at grazing incidence to the primary light. See SHADOW_INCIDENCE_FADE.
    let primary = camera.lights[0].source_direction.xyz;
    let primary_length = length(primary);
    if (primary_length > 0.00001) {
        let incidence = dot(normal, -primary / primary_length);
        let trust = smoothstep(0.0, SHADOW_INCIDENCE_FADE, incidence);
        primary_visibility = mix(1.0, primary_visibility, trust);
    }
    let occlusion = mix(
        AO_AMBIENT_FLOOR,
        1.0,
        textureLoad(ambient_occlusion, pixel, 0).r
    );
    var color = vec3<f32>(0.0);
    for (var index = 0; index < LIGHT_COUNT; index += 1) {
        let light = camera.lights[index];
        // Slot 0 is the primary and is the only shadowed light, in both its ambient and direct
        // shares. The rest are fills: shadowing them would darken the scene twice for one
        // occluder and defeat the purpose of having them.
        let shadowed = index == 0;
        // Only the primary light is both shadowed and occluded, so it is the only one where the two
        // could compound; the accent fills take occlusion alone and are unaffected by this.
        let ambient_scale = select(
            occlusion,
            primary_ambient_scale(primary_visibility, occlusion),
            shadowed
        );
        color += albedo * light.ambient.rgb * ambient_scale;
        let direction_length = length(light.source_direction.xyz);
        if (direction_length > 0.00001) {
            // The cloud deck attenuates the primary light's *direct* share and nothing else: a cloud
            // occludes the sun's disc, not the sky, so the ambient term above passes through it whole.
            // Only slot 0 is the sun; the fills stand in for sky and bounce and are not shadowed at all.
            let clouds = select(1.0, cloud_shadow(world.xy), shadowed);
            let visibility = select(
                1.0,
                mix(SHADOW_DIRECT_FLOOR, 1.0, primary_visibility) * clouds,
                shadowed
            );
            let light_direction = -light.source_direction.xyz / direction_length;
            let diffuse_factor = max(dot(normal, light_direction), 0.0);
            color += albedo * light.diffuse.rgb * diffuse_factor * visibility;
            // Highlight strength falls off with roughness, not just its width, so a fully rough
            // material has no highlight at all. A fixed strength instead gives every surface a
            // sheen regardless of what its material declared, once per light.
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, normal_roughness.w)
            );
            let specular_strength = SPECULAR_STRENGTH * (1.0 - normal_roughness.w);
            color += light.diffuse.rgb * specular * specular_strength * visibility;
        }
    }
    // Self-illumination, decoded from the G-buffer coverage channel. Added after the light loop so
    // it survives full shade, which is the whole point of a lamp: the emitted term takes its hue
    // from the material's own albedo, and the intensity is the material's emissive strength.
    color += albedo * max(coverage - 1.0, 0.0);
    // Fog last, and inside this pass rather than as a later screen-space one. A depth-based fog pass
    // could not fog the water surface: water writes no depth, so it would be fogged at the depth of the
    // terrain *behind* it and would sit in front of its own fog.
    return vec4<f32>(apply_fog(color, world), 1.0);
}

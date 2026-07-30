// The deferred lighting resolve: one fullscreen pass over the G-buffer.
//
// A composition chunk. Requires `scene.wgsl`, `shadow.wgsl` and `atmosphere.wgsl`.
//
// Lighting is deferred because the shadow and occlusion terms are screen-space: each needs the whole
// depth buffer resolved before any pixel can be lit, which a forward pass cannot provide.

// Peak highlight strength for a dielectric, reached only by a fully smooth material.
//
// It doubles as this renderer's reflectance at normal incidence. The standard figure for common
// insulators is 0.04, from a refractive index near 1.5, and 0.06 is close enough that carrying two
// constants a fiftieth apart would be a distinction the eye cannot make and a second thing to keep in
// step.
const SPECULAR_STRENGTH: f32 = 0.06;

// What full saturation does to a surface. Both are multipliers on what the G-buffer already held, so a pale
// road and dark soil each darken by the same *proportion* rather than toward a common colour.
const WET_ALBEDO_SCALE: f32 = 0.55;
const WET_ROUGHNESS_SCALE: f32 = 0.35;

// Snow's own surface. Slightly blue rather than white: snow is a poor absorber across the visible band but
// scatters short wavelengths marginally better, and a pure white reads as blown-out paper next to terrain.
const SNOW_ALBEDO: vec3<f32> = vec3<f32>(0.90, 0.92, 0.96);
// Not very smooth. Fresh snow is a mass of scattering crystals, so it is closer to chalk than to ice; a low
// roughness here gives a sheen that reads as wet plastic.
const SNOW_ROUGHNESS: f32 = 0.62;
// The cosine against vertical below which snow stops holding. About 63 degrees of slope.
const SNOW_SLOPE_LIMIT: f32 = 0.45;

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
    let normal_roughness = textureLoad(g_normal, pixel, 0);
    let normal = normalize(normal_roughness.xyz);

    // The material, as written by whichever surface shader drew this pixel, then weathered.
    let stored = textureLoad(g_albedo, pixel, 0);
    let material = stored.rgb;
    var albedo = material;
    var roughness = normal_roughness.w;
    // Alpha carries the metallic factor. Terrain writes zero and so does every dielectric material, and
    // at zero every expression below reduces to exactly what this pass computed before the channel
    // carried anything — which is what keeps the committed references byte-identical.
    var metallic = clamp(stored.a, 0.0, 1.0);

    // Wet ground is *darker and smoother*, not bluer. Water fills the pores, so less light scatters back
    // out of the surface and what does reflect leaves more coherently. Darkening is the larger half of the
    // cue by some margin — a wetness that only dropped roughness reads as a polished floor.
    let wetness = clamp(camera.weather.x, 0.0, 1.0);
    albedo *= mix(1.0, WET_ALBEDO_SCALE, wetness);
    roughness *= mix(1.0, WET_ROUGHNESS_SCALE, wetness);

    // Snow settles by *slope*, not by altitude. `normal.z` is the cosine of the surface against vertical,
    // which is the physical criterion: an altitude threshold instead puts snow on a sheer cliff face high up
    // and none on the valley floor beside it, which is precisely backwards.
    let snow = clamp(camera.weather.y, 0.0, 1.0);
    let settled = smoothstep(SNOW_SLOPE_LIMIT, 1.0, normal.z) * snow;
    albedo = mix(albedo, SNOW_ALBEDO, settled);
    roughness = mix(roughness, SNOW_ROUGHNESS, settled);
    // Snow lying on a metal hull is snow, not metal. Without this a snowed-over vehicle keeps a bright
    // coloured highlight and no diffuse term, which reads as a chrome silhouette rather than a covered
    // one.
    metallic = mix(metallic, 0.0, settled);

    // A metal has no subsurface scattering, so it has no diffuse term at all: what a dielectric returns
    // diffusely, a metal returns in its specular lobe, tinted by its own colour. Those are the two
    // substitutions below, and both are the identity at zero.
    let diffuse_albedo = albedo * (1.0 - metallic);
    // Reflectance at normal incidence: the dielectric constant for an insulator, the material's own
    // colour for a metal. This is what makes a copper highlight copper and a painted one white.
    let reflectance = mix(vec3<f32>(SPECULAR_STRENGTH), albedo, metallic);
    // The ambient term is deliberately *not* scaled by metalness. A metal in shade is a mirror of the
    // sky, so `albedo * ambient` is already the right answer for it — the same expression a dielectric
    // wants, for a different reason.
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
    // The screen-space term and the material's baked one, combined by `min` and floored once.
    //
    // `min` rather than a product, because the two describe the *same* occlusion by different means: a
    // crevice a baked map darkens is a crevice the depth buffer also sees, so multiplying them darkens it
    // twice. Taking the stronger of the two keeps a baked map authoritative where it is more confident --
    // it knows about geometry too small to survive into the depth buffer -- without compounding.
    //
    // One floor over the combination rather than one each, so a baked map cannot reach past the floor that
    // exists to stop shaded surfaces going black. And because the G-buffer's green is 1.0 wherever nothing
    // baked occlusion, this reduces to the screen-space term exactly on content that has none.
    let baked_occlusion = textureLoad(g_coverage, pixel, 0).g;
    let occlusion = mix(
        AO_AMBIENT_FLOOR,
        1.0,
        min(textureLoad(ambient_occlusion, pixel, 0).r, baked_occlusion)
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
            color += diffuse_albedo * light.diffuse.rgb * diffuse_factor * visibility;
            // Highlight strength falls off with roughness, not just its width, so a fully rough
            // material has no highlight at all. A fixed strength instead gives every surface a
            // sheen regardless of what its material declared, once per light.
            let half_vector = normalize(light_direction + view_direction);
            let specular = pow(
                max(dot(normal, half_vector), 0.0),
                mix(64.0, 8.0, roughness)
            );
            let specular_strength = reflectance * (1.0 - roughness);
            color += light.diffuse.rgb * specular * specular_strength * visibility;
        }
    }
    // Self-illumination, decoded from the G-buffer coverage channel. Added after the light loop so
    // it survives full shade, which is the whole point of a lamp: the emitted term takes its hue
    // from the material's own albedo, and the intensity is the material's emissive strength.
    // Emission takes its hue from the *material*, not from the weathered surface. A lamp under snow is
    // still a lamp: snow lying on its housing should not change the colour of the light coming out of it.
    color += material * max(coverage - 1.0, 0.0);
    // Fog last, and inside this pass rather than as a later screen-space one. A depth-based fog pass
    // could not fog the water surface: water writes no depth, so it would be fogged at the depth of the
    // terrain *behind* it and would sit in front of its own fog.
    return vec4<f32>(apply_fog(color, world), 1.0);
}

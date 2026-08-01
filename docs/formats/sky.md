# Skies: Radiance `.hdr`, equirectangular

The format decision and its reasoning are in [ADR 4001](../adr/4001-hdri-sky.md). This page is the
reference: what the engine reads, what it does with it, and what a content author has to get right.

## What a sky is, and therefore what it is

A sky is **radiance**, not reflectance. Every other texture this engine reads is eight bits per channel
because an albedo is bounded in `0..=1` by physics; a real sky spans four or five orders of magnitude
between a sun disc and a shadowed cloud, and everything a captured environment contributes to a scene
lives in the part of that range an integer format discards.

| Property | Value |
|---|---|
| Container | Radiance, `.hdr` |
| Colour format | `32-bit_rle_rgbe` |
| Projection | Equirectangular, `-Y <height> +X <width>` |
| Orientation | Row 0 is the **zenith**, the last row the nadir |
| Aspect | 2:1 by convention; nothing enforces it |
| Read at | 2048x1024, reducing anything larger; adjustable through `SkyLimits::target_dimension` |
| Refused above | 16384 along either axis, or 256 MiB encoded |

**An 8K file is fine — it is reduced, not refused,** and that is the one bound in this engine that does
not work the way the others do. HDRIs are distributed at 8K by convention and 8K is more resolution than
a sky can use: one texel then covers half a pixel at the horizon, for 358 MiB of video memory. So an
oversized image is box filtered by a whole power of two *while it is decoded*, scanline by scanline, and
the full-size buffer is never allocated. The 128 MiB 8192x4096 file this was developed against reads in
about 280 ms and peaks around 34 MiB.

Both axes reduce by the same factor, which they must — an equirectangular image whose latitude and
longitude reduced differently would render as a stretched sky — so an image that is not a power of two
lands below the target rather than being stretched onto it.

Both scanline encodings are read: the adaptive run-length form every modern writer produces, and the
original literal form with its `(1, 1, 1, n)` repeat records. An `EXPOSURE` in the header is divided back
out, so the values the engine works with are the original radiance rather than whatever the last tool to
touch the file scaled it by.

**Refused, by name rather than read wrong:** `32-bit_rle_xyze`, whose channels are CIE tristimulus values
and which read as RGB produces a plausible, wrongly coloured picture; and the seven orientations that are
not `-Y ... +X ...`, which read anyway would put the sky on the ground.

## Where the coordinates go

The engine's world is **Z-up**. A direction maps into the image as

```text
u = atan2(direction.y, direction.x) / 2pi + 0.5 + yaw
v = acos(direction.z) / pi
```

so `v = 0` is straight up, `v = 0.5` is the horizon, and `v = 1` is straight down. `u` wraps; the sampler
repeats in that axis and clamps in the other, which is what the projection is.

## Using one

```rust
let bytes = std::fs::read("sky/afternoon.hdr")?;
let asset = cic_assets::sky::decode_radiance(&bytes, cic_assets::sky::SkyLimits::default())?;

let mut sky = cic_render::Sky::new(
    context,
    renderer.sky_layout(),
    &asset,
    cic_render::SkySettings::default(),
)?;
// Turn the image until its own sun sits where the scene's light says the sun is.
sky.aim_at(context, environment.sun_direction());

// Take the ambient and the fog colour off the image, so the ground agrees with what is behind it.
let frame = DeferredFrame::new(pose, width, height)
    .in_environment(environment.under_sky(sky.lighting()));
renderer.render(context, &terrain, &models, &water, Some(&sky), &targets, &view);
```

The two settings a designer turns:

- **`intensity`** scales the stored radiance, and scales the light derived from it by the same factor so
  the two cannot disagree. One means "use the file's own values", which is right for a calibrated HDRI and
  wrong for about half of what is in circulation — an environment captured at an unknown exposure carries
  radiance in arbitrary units, and a number a designer turns is the only honest way to fit it to a tone
  curve.
- **`yaw`** rotates the sky about the vertical axis. Set it with `aim_at` rather than by eye; see below.

The interactive viewer takes one directly, so an HDRI can be looked at before anything else is built:

```bash
cargo run -p cic-render --example terrain_viewer --release -- sky/afternoon.hdr
```

`K` toggles it against the analytic sky, and `,` and `.` scrub the hour — which moves the shadows *and*
turns the sky with them, because the viewer re-aims it every frame.

## Aiming it, which is not optional

A captured sky has a sun in it and a scene has a directional light, and nothing makes the two agree by
default. When they disagree the symptom does not look like a rotation: every shadow falls away from a
bright patch of sky that is somewhere else, which reads as the shadows being wrong.

`Sky::aim_at` finds the brightest direction in the image's upper half and rotates the image until that
direction's azimuth matches the light's. It aligns azimuth only — elevation is fixed in the image, so a sky
captured at noon cannot be rotated into a sunset. The honest answer there is a different file.

## What a sky contributes to the light

Three colours, measured off the image on the CPU when it loads:

| Figure | What it is | What reads it |
|---|---|---|
| `horizon` | Mean radiance in a five-degree band around the horizon | The fog colour |
| `zenith` | Mean radiance in a fifteen-degree cap around straight up | Nothing yet; reported for authoring |
| `ambient` | Cosine-weighted mean radiance over the upper hemisphere | The primary light's ambient term |

All three are area-weighted, which an equirectangular image needs and which is easy to omit: the pole rows
hold as many texels as the equator and cover almost no sky, so an unweighted mean is dominated by whatever
is directly overhead.

**The sun is clamped out of all three.** A texel more than eight times the sky's own mean is scaled down
before the integral, because the renderer already has a directional light standing for the sun and adding
the measured irradiance on top counts it twice. The visible result of not doing this is not a brighter
scene: it is a scene with *no shadow contrast at all*, because the ambient becomes as strong as the beam.

**None of this is applied automatically.** `Environment::under_sky` is the line that applies it, and it is
explicit for the same reason `DeferredFrame::in_environment` is: a renderer that silently rewrites one
input from another is a renderer where nobody can tell which one is in force.

## What is not here

- **No package integration.** A scenario cannot name its sky yet; a host reads the file and hands the
  renderer a `Sky`.
- **No prefiltered environment.** The reflection blur comes from the mip chain, which was built by halving
  the image rather than by convolving it with a reflection lobe — a plausible blur of the right width
  rather than the right shape.
- **No cube maps, and no conversion to one.** See ADR 4001.
- **Metals still reflect the ambient rather than the image.** The two agree in colour, since the ambient is
  now measured off the sky, and disagree in detail.

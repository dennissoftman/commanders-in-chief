# ADR 4003: Reflection providers, and absorption deciding what water hides

- Status: accepted, implemented

## Context

`reflection_colour` returned the sky and nothing else. Water beside a hill reflected *sky where the hill
should be*, and a tree standing in a lake reflected nothing at all.

[ADR 4001](4001-hdri-sky.md) wrote that function as the seam a non-analytic answer would be substituted
at, and used it once: an HDRI replaced the analytic gradient without either call site changing. This
record uses it a second time, for an answer that is not an environment lookup at all, and sets up the
third — hardware ray tracing — so that it is a new file rather than a rewrite.

A second thing turned out to be in the way, and it was not obvious until it was measured. Water was
fully opaque a metre or two from its own shoreline, so nothing seen *through* the surface could be
displaced, tinted or lit. That made refraction unimplementable rather than merely subtle.

## Decision

1. **A reflection provider is a composed program, not a runtime branch.** Every `reflection_*` chunk
   exports the same `reflection_colour`, exactly one is composed into a water program, and
   `ReflectionProvider` selects which the pipeline is built from. `reflection.wgsl` becomes
   `reflection_sky.wgsl`; `reflection_screen.wgsl` joins it.
2. **Switching provider rebuilds the pipeline**, through `DeferredRenderer::set_reflection`. It costs a
   shader compile and is a settings change, not a per-frame call.
3. **The lit scene is copied aside between the lighting pass and the water pass**, and bound as group 4
   by its own chunk, `scene_colour.wgsl`. One copy serves both features that read it.
4. **The device asks the adapter for its bind-group limit** rather than taking wgpu's default of four.
5. **Compositing moves out of the blender and into the water shader.** The pass outputs opaque and mixes
   the bed itself.
6. **The screen-space march is world-space with geometrically growing steps**, refined by bisection,
   falling back to the sky on any miss.
7. **Refraction displaces the transmitted view** by the wave normal, built in world space and projected.
8. **Opacity follows absorption, not the shoreline feather**, exponentially, from the same figure that
   drives the tint. `depth_scale` is an absorption length; `edge_feather` no longer decides anything.

## Rationale

**A provider is a program because that is what keeps the callers identical.** The alternative — one
chunk with a uniform selecting a branch — costs every water pixel the branch whether or not the feature
is on, and grows a new parameter and a new binding for each provider added. Composition already exists
here for exactly this reason, and the cost is one more entry in a table.

The shape is chosen for the provider that does not exist yet. Hardware ray tracing answers the same
question with a completely different mechanism and its own acceleration-structure binding; as a fourth
chunk it needs no change to `water.wgsl`, to the sky, or to either call site. `world_position` has been
in the signature since ADR 4001 precisely because a trace needs an origin.

**Group 4 forced decision 4, and four is not an arbitrary number.** Water already binds the scene, its
own uniform, the shadow cascades and the environment. `wgpu::Limits::default()` stops at four because
that is Vulkan's guaranteed minimum for `maxBoundDescriptorSets`. Asking for *what the adapter reports*
rather than a hardcoded figure is what makes the raise cost nothing: the device requests exactly what
the hardware already has, so no adapter that worked before stops working. Both adapters this project
tests on answer eight, which is wgpu's own ceiling. Every other limit stays at its default, because
raising one because a pass needs it is a different thing from raising all of them because they were
available.

**Decision 5 is what makes refraction possible at all**, and is not an optimisation. Fixed-function
blending can only ever read the destination at *this* pixel, and a displaced bed is by definition read
from another one. With a zero offset the arithmetic is identical — `scene_colour` holds the same lit
scene the destination held — and every reference image passing unchanged when the change landed is the
evidence for that.

**The march grows its steps, and the first version did not.** Fixed seven-unit steps over sixty units is
a defensible march for a rough reflector filling the frame and is useless for water: a lake reflects its
*far shore*, hundreds of units away at a grazing angle, so every ray ran out of march before reaching
anything. Growth puts fine samples where the ray leaves the surface and coarse ones far out, where a
ridge spans many steps anyway. Marching in world space rather than screen space stays right for the same
reason: a screen-space DDA spends samples evenly over pixels, and at a grazing view the pixels near the
horizon each cover enormous distances — so the far end of the ray, the part that matters, is sampled
worst. The technique is standard; McGuire and Mara (2014) for the refinement, Sousa (2013) for the fades.

**The sky provider stays composed in under the screen-space one.** A march that leaves the frame, passes
behind geometry, or points below the surface has no answer, and the fallback is what makes those
acceptable: a miss is a less informed colour, never a wrong one.

**Absorption is one figure because it is one physical thing.** Light crossing water is absorbed along its
path, which both shifts its colour and hides what it came from — so a tint ramp and an opacity ramp are
two names for the same measurement. Having two meant the *shorter* decided opacity, and a one-to-three
unit shoreline feather is not a claim anybody would make about how deep water has to be before its bed
disappears. Exponential rather than clamped because that is what absorption does; the practical
difference is at both ends, with shallows staying genuinely clear and deep water approaching opaque
without snapping to it.

`edge_feather` is left in the material and in the uniform rather than removed. Repacking would move every
field after it, and a silently misaligned uniform block is the failure both size assertions exist for.

## Consequences

- **Every water reference moved again**, and two scenes were added.
- **`depth_scale` means something different.** It was the depth over which one tint became another; it is
  now an absorption length, and the presets are retuned accordingly — a lake from twenty units to six.
  A caller that set it by hand will get more transparent water than before.
- **A full-resolution HDR copy per frame with water in it**, about 18 MiB at 1920×1200, recorded only
  when a frame has water. Not yet attributed to a `TimedPass` of its own.
- **`edge_feather` is now inert.** It is still validated and still packed, and it decides nothing.
- **Screen-space reflection cannot reflect what is not on screen**, in three distinct ways: geometry
  outside the frame, geometry behind other geometry, and geometry behind the camera. All three fail to
  the sky. The second is the one ray tracing exists to fix.
- **Refraction is a sub-unit effect at this engine's scale.** The physical lateral shift is about a
  quarter of the surface slope times the depth; a lake's slope is under four degrees, so the preset moves
  a bed five units down by about half a world unit. A heightfield sampled every eight units cannot carry
  detail finer than sixteen, so on terrain geometry alone that is nearly invisible. It shows where the
  ground carries tiled albedo finer than its own heightfield, which a real map does and the fixtures do
  not.

## What implementing it established

**Two separate dimensional and scale errors, and both passed the entire suite.** The first version of
refraction added a world-space offset straight to a texture coordinate scaled by the reciprocal
viewport, which treats world units as pixels — off by the projection and the distance, so the offset came
to about a hundredth of a pixel. The first version of the march covered sixty units when it needed
hundreds. In both cases every reference image passed unchanged, and the second measured 0.20 of 255
against the provider it was supposed to differ from. A rendering feature that is *silently disconnected*
looks exactly like one that is subtle, and neither a green suite nor a capture distinguishes them.

What distinguishes them is a control that differs in one variable — the same scene through both
providers, the same water with the term zeroed — which is the project's own standing rule about
fixtures, arrived at again from a different direction.

**A fixture can make a feature unmeasurable, and both of these were.** A bowl has nothing standing above
its own water, so every reflected ray leaves the frame and a screen-space provider is a no-op in it. A
smooth pale bed displaces to the same colour, so refraction is a no-op over it: moving it twenty world
units changed 0.18% of pixels by at most 4/255, and it took forcing the sample to a constant colour to
establish the term was connected at all. `reflecting_terrain` exists to have a ridge above the waterline
and ripples beneath it.

**Camera angle decides which half of water is even visible.** At eight degrees above the surface the
reflected share is over two thirds and the body contributes almost nothing; at forty-five it is two
percent and the body is everything. Reflection can only be measured at the first and refraction only at
the second, so the two features needed two poses over one fixture — and the pitch of each is stated with
its arithmetic, because inheriting a camera is how the first attempt at both measured nothing.

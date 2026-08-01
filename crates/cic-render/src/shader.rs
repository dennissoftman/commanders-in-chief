//! The WGSL shader set, assembled from composable chunks.
//!
//! # Why composition
//!
//! WGSL has no include mechanism. That one gap shaped the shader set badly: any pass needing
//! `shadow_visibility`, `world_from_depth`, or the sky colours had to live in the *same file* as them,
//! so the deferred lighting pass, the tone-mapping composite, and the whole water surface accumulated
//! into a single 620-line file that had nothing else in common. The alternative — a second copy of the
//! cascade selection, with its normal offset, depth slack, blend band, and incidence fade — is the kind
//! of duplication that drifts silently, because a shader is code `cargo build` does not compile.
//!
//! Concatenating named chunks in Rust is the whole mechanism. No preprocessor, no dependency, no
//! directives inside the WGSL: a [`Program`] lists its chunks, [`compose`] joins their sources, and the
//! result is what the device is handed. Chunks are ordered by the program, because WGSL requires a
//! declaration before its use.
//!
//! # Why `staged` is a field and not a comment
//!
//! Three programs are wired to no pipeline: the terrain, road and boundary viewer passes, held for the
//! map editor M8 plans. They were previously indistinguishable from dead code, which is how six
//! *genuinely* dead shaders survived in the set with comments describing a uniform layout that no longer
//! existed. Marking them makes the live set countable, and [`Program::staged`] means a reader never has to
//! grep the pipelines to find out whether a file does anything.
//!
//! The mechanism has now done its job twice in the intended direction: `terrain_virtual` was staged for
//! the page cache and `ui` for M4's interface, and each went live when the work that needed it landed,
//! with nothing to clean up because neither had rotted.

/// Every chunk of WGSL in the crate, as `(name, source)`.
///
/// Compiled in rather than loaded from disk: a shader is code, and shipping it as a loose file invites a
/// mismatch between the binary and the file next to it.
const CHUNKS: &[(&str, &str)] = &[
    ("antialias", include_str!("shaders/antialias.wgsl")),
    ("atmosphere", include_str!("shaders/atmosphere.wgsl")),
    (
        "boundary_viewer",
        include_str!("shaders/boundary_viewer.wgsl"),
    ),
    ("composite", include_str!("shaders/composite.wgsl")),
    ("lighting", include_str!("shaders/lighting.wgsl")),
    ("model_gbuffer", include_str!("shaders/model_gbuffer.wgsl")),
    ("motion", include_str!("shaders/motion.wgsl")),
    (
        "reflection_screen",
        include_str!("shaders/reflection_screen.wgsl"),
    ),
    (
        "reflection_sky",
        include_str!("shaders/reflection_sky.wgsl"),
    ),
    ("road_viewer", include_str!("shaders/road_viewer.wgsl")),
    ("scene", include_str!("shaders/scene.wgsl")),
    ("scene_colour", include_str!("shaders/scene_colour.wgsl")),
    ("scenery", include_str!("shaders/scenery.wgsl")),
    ("shadow", include_str!("shaders/shadow.wgsl")),
    ("sky", include_str!("shaders/sky.wgsl")),
    ("taa", include_str!("shaders/taa.wgsl")),
    ("terrain_ao", include_str!("shaders/terrain_ao.wgsl")),
    (
        "terrain_forward",
        include_str!("shaders/terrain_forward.wgsl"),
    ),
    (
        "terrain_gbuffer",
        include_str!("shaders/terrain_gbuffer.wgsl"),
    ),
    (
        "terrain_reduce",
        include_str!("shaders/terrain_reduce.wgsl"),
    ),
    (
        "terrain_viewer",
        include_str!("shaders/terrain_viewer.wgsl"),
    ),
    (
        "terrain_virtual",
        include_str!("shaders/terrain_virtual.wgsl"),
    ),
    ("transfer", include_str!("shaders/transfer.wgsl")),
    ("ui", include_str!("shaders/ui.wgsl")),
    ("water", include_str!("shaders/water.wgsl")),
];

/// One shader module, named and assembled from chunks in order.
#[derive(Debug, Clone, Copy)]
pub struct Program {
    /// The module's name, used as its debug label and to look it up.
    pub name: &'static str,
    /// The chunks it is assembled from, in declaration order.
    pub chunks: &'static [&'static str],
    /// Whether this program is deliberately bound to no pipeline yet.
    ///
    /// True means held for a later milestone, not dead. A staged program is still parsed and validated,
    /// so it cannot rot into something that no longer compiles while nobody is looking.
    pub staged: bool,
}

/// Every shader module the crate can build.
pub const PROGRAMS: &[Program] = &[
    // Live: each of these is bound to a pipeline.
    Program {
        name: "lighting",
        // `sky` before `atmosphere`, which takes `TAU` and the horizon colour from it, and before
        // `lighting`, which asks it what a pixel with no geometry behind it shows.
        chunks: &["scene", "shadow", "sky", "atmosphere", "lighting"],
        staged: false,
    },
    Program {
        name: "composite",
        chunks: &["scene", "composite"],
        staged: false,
    },
    Program {
        name: "antialias",
        chunks: &["scene", "antialias"],
        staged: false,
    },
    Program {
        name: "taa",
        chunks: &["scene", "taa"],
        staged: false,
    },
    // Two water programs, differing in one chunk. That is the whole reflection-provider mechanism:
    // every `reflection_*` chunk exports the same `reflection_colour`, exactly one is composed in, and
    // `ReflectionProvider` picks which. A ray-traced provider is a third entry here and a third arm
    // there rather than a change to either caller. See `reflection_sky.wgsl`.
    //
    // `scene_colour` is in both, because refraction reads it whichever provider is in force -- and
    // because a binding declared inside a chunk that can be substituted disappears when it is.
    Program {
        name: "water",
        // `reflection_sky` after `sky`, because it calls `sky_reflection` from it, and before `water`,
        // which calls `reflection_colour`.
        chunks: &[
            "scene",
            "scene_colour",
            "shadow",
            "sky",
            "atmosphere",
            "reflection_sky",
            "water",
        ],
        staged: false,
    },
    Program {
        name: "water_screen",
        // `reflection_screen` needs `scene_colour` as well as `sky`, and still needs `sky`: a march
        // that misses falls back to it, which is what makes an approximation that cannot see off-screen
        // geometry acceptable.
        chunks: &[
            "scene",
            "scene_colour",
            "shadow",
            "sky",
            "atmosphere",
            "reflection_screen",
            "water",
        ],
        staged: false,
    },
    Program {
        name: "terrain_gbuffer",
        // `transfer` first: it declares the sRGB decode a page read goes through.
        chunks: &["transfer", "motion", "terrain_gbuffer"],
        staged: false,
    },
    Program {
        name: "model_gbuffer",
        // `scenery` and `motion` first: they declare `sway_offset` and `motion_vector`, which the entry
        // points here call, and WGSL requires a declaration before its use.
        chunks: &["scenery", "motion", "model_gbuffer"],
        staged: false,
    },
    Program {
        name: "terrain_ao",
        chunks: &["terrain_ao"],
        staged: false,
    },
    Program {
        name: "terrain_forward",
        chunks: &["terrain_forward"],
        staged: false,
    },
    Program {
        name: "terrain_virtual",
        chunks: &["transfer", "terrain_virtual"],
        staged: false,
    },
    Program {
        name: "terrain_reduce",
        chunks: &["transfer", "terrain_reduce"],
        staged: false,
    },
    Program {
        name: "ui",
        chunks: &["ui"],
        staged: false,
    },
    // Staged: real work held for a milestone that has not started.
    Program {
        name: "terrain_viewer",
        chunks: &["terrain_viewer"],
        staged: true,
    },
    Program {
        name: "road_viewer",
        chunks: &["road_viewer"],
        staged: true,
    },
    Program {
        name: "boundary_viewer",
        chunks: &["boundary_viewer"],
        staged: true,
    },
];

/// Returns one chunk's source by name.
#[must_use]
pub fn chunk(name: &str) -> Option<&'static str> {
    CHUNKS
        .iter()
        .find_map(|(candidate, source)| (*candidate == name).then_some(*source))
}

/// Returns a program by name.
#[must_use]
pub fn program(name: &str) -> Option<Program> {
    PROGRAMS
        .iter()
        .find(|candidate| candidate.name == name)
        .copied()
}

/// Assembles a program's source, or `None` when it names a chunk that does not exist.
///
/// Each chunk is preceded by a marker naming it. A composed module is what `naga` reports line numbers
/// against, and without the markers a validation error in a four-chunk program points into a file the
/// reader has to find by counting.
#[must_use]
pub fn compose(name: &str) -> Option<String> {
    let program = program(name)?;
    let mut source = String::new();
    for chunk_name in program.chunks {
        source.push_str("// ==== chunk: ");
        source.push_str(chunk_name);
        source.push_str(" ====\n");
        source.push_str(chunk(chunk_name)?);
        source.push('\n');
    }
    Some(source)
}

#[cfg(test)]
mod tests {
    use super::{CHUNKS, PROGRAMS, chunk, compose, program};

    #[test]
    fn every_program_composes_parses_and_validates() {
        // The point of this test: the shader set is the most valuable thing in the crate, and a shader is
        // code `cargo build` does not compile. `naga` is the same WGSL front end `wgpu` uses, so passing
        // here means the composed module really compiles — including the staged programs, which nothing
        // else would ever exercise.
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        for entry in PROGRAMS {
            let source = compose(entry.name)
                .unwrap_or_else(|| panic!("{} names a chunk that does not exist", entry.name));
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{} failed to parse: {error:?}", entry.name));
            validator
                .validate(&module)
                .unwrap_or_else(|error| panic!("{} failed to validate: {error:?}", entry.name));
        }
    }

    #[test]
    fn no_chunk_is_orphaned() {
        // The structural fix for how this set went wrong. Eleven of sixteen shaders were bound to no
        // pipeline, six of them genuinely superseded, and nothing failed — an unreferenced file simply
        // sat there validating. A chunk no program names now breaks the build instead.
        for (name, _) in CHUNKS {
            assert!(
                PROGRAMS.iter().any(|entry| entry.chunks.contains(name)),
                "chunk {name} is named by no program: wire it up or delete it"
            );
        }
    }

    #[test]
    fn the_live_and_staged_split_is_what_is_declared() {
        // Stated as a number so adding a program without wiring it is a deliberate act rather than an
        // oversight. Two have left the staged set since it was introduced -- `terrain_virtual` when the
        // page cache bound it and `ui` when M4's interface did -- and the three remaining are the viewer
        // passes M8's map editor wants.
        //
        // The thirteenth live program is the second *water* program. Those two differ in one chunk and
        // are the reflection-provider mechanism: a provider is a program here rather than a branch in a
        // shader, so a third one is a fourteenth entry. That is deliberate and this count is where it
        // has to be admitted.
        let live = PROGRAMS.iter().filter(|entry| !entry.staged).count();
        let staged = PROGRAMS.iter().filter(|entry| entry.staged).count();
        assert_eq!(live, 13, "live programs");
        assert_eq!(staged, 3, "staged programs");
        // Was 23. `reflection` became `reflection_sky`, `reflection_screen` joined it, and
        // `scene_colour` carries the binding both of them and refraction read.
        assert_eq!(CHUNKS.len(), 25);
    }

    #[test]
    fn every_model_entry_point_sways() {
        // The one property no capture of the lit frame can check. A shadow cascade that applied a
        // different displacement than the G-buffer would throw a shadow detached from its caster, and the
        // caster and the shadow are in different parts of the image — so a reference comparison would
        // pass on a frame that is visibly wrong to anyone looking at the ground.
        //
        // Textual rather than semantic, which is what a shader test can be. It asserts that the four
        // entry points all route their position through the one function, which is the structure that
        // makes them agree; it cannot assert that the function is right, and the reference captures do
        // that.
        let source = compose("model_gbuffer").expect("model_gbuffer composes");
        // Five calls across four entry points: the G-buffer stage places its vertex twice, once at this
        // frame's time and once at the previous frame's, which is what makes the motion vector exact for a
        // swaying plant rather than approximate.
        assert_eq!(
            source.matches("place_vertex(input").count(),
            5,
            "every entry point must place its vertex through the shared path"
        );
        assert_eq!(
            source.matches("fn sway_offset").count(),
            1,
            "one displacement, not one per pass"
        );
        // And the shared path is the *only* way a position reaches a clip space here, so a later entry point
        // cannot quietly bypass the sway by transforming the raw attribute.
        assert_eq!(
            source.matches("input.position").count(),
            1,
            "the vertex position must be read in one place"
        );
    }

    #[test]
    fn composition_orders_chunks_as_the_program_lists_them() {
        // WGSL requires a declaration before its use, so order is not cosmetic. `scene` declares the
        // camera that `lighting` reads, and reversing them would fail to parse.
        let source = compose("lighting").expect("lighting composes");
        let scene = source.find("chunk: scene").expect("scene present");
        let lighting = source.find("chunk: lighting").expect("lighting present");
        assert!(scene < lighting, "scene must precede lighting");
    }

    #[test]
    fn an_unknown_name_is_absent_rather_than_a_panic() {
        assert!(compose("no_such_program").is_none());
        assert!(program("no_such_program").is_none());
        assert!(chunk("no_such_chunk").is_none());
    }

    #[test]
    fn no_chunk_is_empty_and_every_one_is_addressable() {
        for (name, source) in CHUNKS {
            assert!(!source.trim().is_empty(), "{name} is empty");
            assert_eq!(chunk(name), Some(*source));
        }
    }

    #[test]
    fn no_chunk_offsets_the_framebuffer_position_by_half_a_pixel() {
        // The framebuffer coordinate of the top-left pixel is (0.5, 0.5), so a pass converting it to a
        // texture coordinate multiplies straight through. Three passes added a further half pixel and each
        // therefore sampled half a pixel down and right of the fragment it was shading: a translation of the
        // whole frame, plus a two-texel average where the downsample was meant to return one exact texel.
        //
        // It survived a long time because every reference was rendered through it, so the images agreed with
        // each other. What exposed it was the temporal resolve, whose accumulation of a static frame could
        // not reach a fixed point while each pass read its history offset from where it had written it.
        //
        // Pinned textually rather than by a capture for that exact reason — a capture comparison cannot
        // catch an error that is applied uniformly to the reference and the result alike.
        for (name, source) in CHUNKS {
            assert!(
                !source.contains("input.position.xy + vec2<f32>(0.5)"),
                "{name} offsets the framebuffer position by half a pixel, which it already carries"
            );
        }
    }

    #[test]
    fn no_chunk_carries_an_inherited_licence_header() {
        // The licence is declared once, in the workspace manifest, so no shader asserts one of its own.
        // That makes *any* licence header in this directory an inherited one, which is exactly the
        // copy-paste this guards against: the predecessor's shaders carried GPL headers, and a pasted
        // region would bring the obligation back with it. See LICENSING.md.
        for (name, source) in CHUNKS {
            for marker in ["SPDX", "GPL", "Copyright (C)", "License"] {
                assert!(!source.contains(marker), "{name} still mentions {marker}");
            }
        }
    }
}

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
//! Five programs are wired to no pipeline. They are real work held for a milestone that has not started
//! — `ui` for M4's interface, the virtual-texture pair for terrain detail past one texture — and they
//! were previously indistinguishable from dead code, which is how six *genuinely* dead shaders survived
//! in the set with comments describing a uniform layout that no longer existed. Marking them makes the
//! live set countable, and [`Program::staged`] means a reader never has to grep the pipelines to find
//! out whether a file does anything.

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
    ("road_viewer", include_str!("shaders/road_viewer.wgsl")),
    ("scene", include_str!("shaders/scene.wgsl")),
    ("shadow", include_str!("shaders/shadow.wgsl")),
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
        "terrain_viewer",
        include_str!("shaders/terrain_viewer.wgsl"),
    ),
    (
        "terrain_virtual",
        include_str!("shaders/terrain_virtual.wgsl"),
    ),
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
        chunks: &["scene", "shadow", "atmosphere", "lighting"],
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
        name: "water",
        chunks: &["scene", "shadow", "atmosphere", "water"],
        staged: false,
    },
    Program {
        name: "terrain_gbuffer",
        chunks: &["terrain_gbuffer"],
        staged: false,
    },
    Program {
        name: "model_gbuffer",
        chunks: &["model_gbuffer"],
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
    // Staged: real work held for a milestone that has not started.
    Program {
        name: "ui",
        chunks: &["ui"],
        staged: true,
    },
    Program {
        name: "terrain_virtual",
        chunks: &["terrain_virtual"],
        staged: true,
    },
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
        // oversight. The staged five are `ui`, the virtual-texture pair, and the two viewer passes.
        let live = PROGRAMS.iter().filter(|entry| !entry.staged).count();
        let staged = PROGRAMS.iter().filter(|entry| entry.staged).count();
        assert_eq!(live, 8, "live programs");
        assert_eq!(staged, 5, "staged programs");
        assert_eq!(CHUNKS.len(), 16);
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

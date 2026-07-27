//! Renderer foundations: the WGSL shader set, GPU-independent bookkeeping, and texture resources.
//!
//! # Scope
//!
//! This crate currently holds the parts of the renderer that need neither a GPU device nor an
//! asset decoder, which makes all of them testable without either:
//!
//! - **The WGSL shader set.** Every shader is parsed and validated at test time by the same front
//!   end the GPU backend uses. A shader is code that `cargo build` does not compile, so without
//!   this a copy error or a syntax regression would produce a clean build and a blank frame.
//! - **Virtual-page residency bookkeeping** ([`terrain_virtual`]), which decides which terrain
//!   pages to stage and evict for a given view. That is arithmetic, and the subtle bugs live in it,
//!   so it is kept separate from device calls.
//! - **Texture resources** ([`resource`]), which deduplicate decoded images by content hash under
//!   explicit byte budgets.
//!
//! The pipelines are next: terrain meshing and the deferred pass, models, shadow cascades, ambient
//! occlusion, water, and the frame loop, all built against [`cic_assets`]. See
//! `docs/milestones/m3-renderer.md`.

pub mod detail;
pub mod resource;
pub mod terrain_virtual;

use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub use resource::{TextureId, TextureResourceManager};

/// Every WGSL shader in the set, as `(name, source)`.
///
/// Compiled in rather than loaded from disk: a shader is code, and shipping it as a loose file
/// invites a mismatch between the binary and the file next to it.
pub const SHADERS: &[(&str, &str)] = &[
    ("boundary_viewer", include_str!("boundary_viewer.wgsl")),
    ("model", include_str!("model.wgsl")),
    ("road_viewer", include_str!("road_viewer.wgsl")),
    ("scene_shadow", include_str!("scene_shadow.wgsl")),
    ("shader", include_str!("shader.wgsl")),
    ("terrain", include_str!("terrain.wgsl")),
    ("terrain_ao", include_str!("terrain_ao.wgsl")),
    ("terrain_deferred", include_str!("terrain_deferred.wgsl")),
    ("terrain_shadow", include_str!("terrain_shadow.wgsl")),
    ("terrain_viewer", include_str!("terrain_viewer.wgsl")),
    ("terrain_virtual", include_str!("terrain_virtual.wgsl")),
    ("ui", include_str!("ui.wgsl")),
    ("viewer", include_str!("viewer.wgsl")),
];

/// Returns one shader's source by name.
#[must_use]
pub fn shader(name: &str) -> Option<&'static str> {
    SHADERS
        .iter()
        .find_map(|(candidate, source)| (*candidate == name).then_some(*source))
}

/// A failure in a renderer operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// A texture's dimensions, byte length, or declared size were inconsistent or out of range.
    InvalidTexture,
    /// A texture, or the texture set as a whole, exceeded its explicit byte budget.
    TextureTooLarge,
}

impl Display for RenderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTexture => "texture dimensions or byte length are invalid",
            Self::TextureTooLarge => "texture exceeds its explicit byte budget",
        })
    }
}

impl Error for RenderError {}

#[cfg(test)]
mod shader_tests {
    use super::{SHADERS, shader};

    #[test]
    fn every_shader_parses_and_validates() {
        // The point of this test: the shader set is the most valuable thing carried across, and a
        // silent copy error would otherwise surface as a blank frame much later. `naga` is the same
        // WGSL front end `wgpu` uses, so passing here means the shader compiles for real.
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        for (name, source) in SHADERS {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{name}.wgsl failed to parse: {error:?}"));
            validator
                .validate(&module)
                .unwrap_or_else(|error| panic!("{name}.wgsl failed to validate: {error:?}"));
        }
    }

    #[test]
    fn the_shader_set_is_complete_and_addressable() {
        assert_eq!(SHADERS.len(), 13, "the seeded, licence-clean shader set");
        for (name, source) in SHADERS {
            assert!(!source.trim().is_empty(), "{name}.wgsl is empty");
            assert_eq!(shader(name), Some(*source));
        }
        assert_eq!(shader("no_such_shader"), None);
    }

    #[test]
    fn no_shader_carries_an_inherited_licence_header() {
        // The licence is an open decision, so no file may assert one. This guards against a
        // header being reintroduced by a copy-paste before that decision is made.
        for (name, source) in SHADERS {
            for marker in ["SPDX", "GPL", "Copyright (C)", "License"] {
                assert!(
                    !source.contains(marker),
                    "{name}.wgsl still mentions {marker}"
                );
            }
        }
    }
}

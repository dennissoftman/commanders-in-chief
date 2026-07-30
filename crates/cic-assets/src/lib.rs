//! Native asset formats: glTF models, the terrain container, and the map package.
//!
//! # The format decisions, in one place
//!
//! | Data | Format | Why |
//! |---|---|---|
//! | Models, props, units | **glTF 2.0** (`.glb`) | A published standard every DCC tool exports. Writing a mesh format would mean writing a Blender exporter first. |
//! | Textures | **DDS** with BC1/BC5/BC7 blocks | Stays compressed in video memory and carries its own mip chain, so the upload is a copy. Every texture tool writes it. |
//! | Terrain heightfield and layers | **custom chunked binary** (`.cict`) | A regular numeric grid. No standard describes one well, and `u16` elevations upload directly as a baseline `R16Uint` GPU texture. |
//! | Scenario: placements, players, waypoints | **JSON** (`map.json`) | Diffable, reviewable, hand-fixable. The bulk numerics are elsewhere, so the size argument for a binary encoding does not apply. |
//! | A whole map | **zip** (`.cicmap`) | Already has a directory, per-member compression, and universal tooling. |
//!
//! Every decoder here is bounded and total: it takes explicit limits, refuses rather than allocates
//! when one is crossed, and reports a structured error naming what it found and what it expected.
//! Nothing panics on hostile input.

pub mod bc;
pub mod image;
pub mod model;
pub mod package;
pub mod scenario;
pub mod templates;
pub mod terrain;
#[cfg(test)]
mod testing;
pub mod texture;

pub use image::ColourSpace;
pub use model::{
    AlphaMode, DEFAULT_ALPHA_CUTOFF, Model, ModelError, ModelImage, ModelLimits, ModelMaterial,
    ModelPrimitive, ModelTextures, ModelVertex, import_model, resolve_model_textures,
};
pub use package::{MapPackage, PackageError, PackageLimits};
pub use scenario::{
    ObjectPlacement, PlayerSlot, Position, Scenario, ScenarioError, TerrainReference, Waypoint,
};
pub use templates::{Template, TemplateError, TemplateKind, TemplateSet};
pub use terrain::{
    Terrain, TerrainError, TerrainLayer, TerrainLimits, decode_terrain, resolve_terrain_textures,
};
pub use texture::{
    BlockFormat, TEXTURE_DIRECTORY, TextureAsset, TextureError, TextureLimits, TextureResolveError,
    decode_dds, resolve_named_textures,
};

#[cfg(test)]
mod model_tests {
    // Every float compared here is an exactly-representable constant the fixtures set directly
    // (0.0, 1.0, 10.0, 0.9, ...), so exact comparison is the correct assertion -- an epsilon would
    // weaken these tests rather than make them robust.
    #![allow(clippy::float_cmp)]
    use crate::model::{AlphaMode, ModelError, ModelLimits, import_model};
    use crate::testing::{
        TEXEL_COLOURS, TriangleOptions, foliage_triangle_glb, normal_mapped_triangle_glb,
        textured_triangle_glb, triangle_glb,
    };

    #[test]
    fn imports_a_triangle_with_its_attributes_and_material() {
        let glb = triangle_glb(TriangleOptions::default());
        let model = import_model(&glb, ModelLimits::default()).expect("import");

        assert_eq!(model.name, "fixture");
        assert_eq!(model.primitives.len(), 1);
        assert_eq!(model.vertex_count(), 3);
        assert_eq!(model.triangle_count(), 1);
        assert!(!model.has_skin);
        assert!(!model.has_animation);

        let primitive = &model.primitives[0];
        assert_eq!(primitive.indices, [0, 1, 2]);
        assert_eq!(primitive.vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(primitive.vertices[1].normal, [0.0, 0.0, 1.0]);
        assert_eq!(primitive.vertices[1].uv, [1.0, 0.0]);
        assert_eq!(primitive.material, Some(0));

        let material = &model.materials[0];
        assert_eq!(material.name, "stone");
        assert_eq!(material.base_color, [0.8, 0.7, 0.6, 1.0]);
        assert_eq!(material.metallic, 0.0);
        assert_eq!(material.roughness, 0.9);
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
        assert_eq!(material.base_color_texture, None);
        assert_eq!(material.normal_texture, None);
        assert_eq!(material.metallic_roughness_texture, None);
        // Absent means one, not zero: the extension multiplies the emissive factor, so a document
        // without it has to read exactly as it did before the extension existed.
        assert_eq!(material.emissive_strength, 1.0);
        assert_eq!(material.emissive, [0.0, 0.0, 0.0]);
        assert!(!material.double_sided);
        assert!(
            model.images.is_empty(),
            "a fixture with no texture must carry no image"
        );
    }

    #[test]
    fn reads_the_pbr_map_set_and_its_scales() {
        // Every slot at once, because the failure this guards against is a slot resolved to the wrong
        // image: they are all indices into one list, and a shift by one puts a roughness map where a
        // normal map belongs and nothing reports it.
        let glb = normal_mapped_triangle_glb();
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        let material = &model.materials[0];
        assert_eq!(material.base_color_texture, Some(0));
        assert_eq!(material.normal_texture, Some(1));
        assert_eq!(material.metallic_roughness_texture, Some(2));
        assert_eq!(
            material.occlusion_texture,
            Some(2),
            "shared with the MR map"
        );
        assert_eq!(material.normal_scale, 0.75);
        assert_eq!(material.occlusion_strength, 0.5);
        assert_eq!(model.images.len(), 3);
    }

    #[test]
    fn derives_a_tangent_frame_when_a_normal_mapped_primitive_omits_one() {
        // glTF asks for TANGENT on a normal-mapped primitive and exporters routinely omit it, so this
        // is the ordinary path rather than an edge case. The fixture's UVs increase with +X, so the
        // derived tangent must point along +X -- if the 2x2 solve were transposed it would come out
        // along +Y and every normal map would be rotated a quarter turn.
        let glb = normal_mapped_triangle_glb();
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        for vertex in &model.primitives[0].vertices {
            assert!(
                (vertex.tangent[0] - 1.0).abs() < 1.0e-5,
                "expected +X, got {:?}",
                vertex.tangent
            );
            assert_eq!(vertex.tangent[3], 1.0, "the fixture's UVs are right-handed");
            // Orthogonal to the normal, which is what Gram-Schmidt is for.
            let along = vertex.tangent[0] * vertex.normal[0]
                + vertex.tangent[1] * vertex.normal[1]
                + vertex.tangent[2] * vertex.normal[2];
            assert!(
                along.abs() < 1.0e-5,
                "tangent must lie in the tangent plane"
            );
        }
    }

    #[test]
    fn a_primitive_with_no_normal_map_is_not_charged_for_a_tangent_frame() {
        // The derivation is the most expensive thing in the import, and nothing reads a tangent
        // without a normal map. The default is still a unit vector rather than a zero, so a renderer
        // that builds a basis from it unconditionally gets a valid one.
        let glb = triangle_glb(TriangleOptions::default());
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        for vertex in &model.primitives[0].vertices {
            assert_eq!(vertex.tangent, [0.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn reads_a_masked_material_with_its_cutoff() {
        // The distinction a boolean could not carry. Masked geometry can be drawn in a G-buffer and in
        // every shadow cascade by discarding fragments; blended geometry cannot, because a G-buffer
        // pixel holds one material.
        let glb = foliage_triangle_glb();
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        let material = &model.materials[0];
        assert_eq!(material.alpha_mode, AlphaMode::Masked { cutoff: 0.4 });
        assert_eq!(material.alpha_mode.cutoff(), Some(0.4));
        assert!(material.double_sided, "a leaf card is seen from both faces");
    }

    #[test]
    fn an_opaque_material_has_no_cutoff_and_a_blended_one_reports_the_default() {
        assert_eq!(AlphaMode::Opaque.cutoff(), None);
        assert_eq!(AlphaMode::Blended.cutoff(), Some(0.5));
    }

    #[test]
    fn decodes_an_embedded_image_and_links_it_to_its_material() {
        // The whole texture path from container to pixels: a PNG in the binary chunk, reached through
        // a buffer view, decoded, and normalized to RGBA8 with its index preserved so the renderer can
        // resolve `base_color_texture` against `images`.
        let glb = textured_triangle_glb();
        let model = import_model(&glb, ModelLimits::default()).expect("import");

        assert_eq!(model.images.len(), 1);
        let image = &model.images[0];
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(
            image.rgba,
            TEXEL_COLOURS.as_flattened(),
            "the decode must preserve channel order and row order, not merely the byte count"
        );
        assert_eq!(model.materials[0].base_color_texture, Some(0));
    }

    #[test]
    fn refuses_an_image_past_the_declared_bound() {
        // The bound is checked before the conversion allocates, so a hostile declaration cannot make
        // the importer reserve the memory it is being refused.
        let glb = textured_triangle_glb();
        let error = import_model(
            &glb,
            ModelLimits {
                maximum_image_bytes: 8,
                ..ModelLimits::default()
            },
        )
        .expect_err("a 2x2 RGBA image needs sixteen bytes");
        assert!(
            matches!(
                error,
                ModelError::LimitExceeded {
                    what: "image bytes",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn bakes_a_parent_node_transform_into_the_vertices() {
        // The importer flattens the hierarchy, so a nested node's transform must reach its geometry.
        let glb = triangle_glb(TriangleOptions {
            nested_transform: true,
            ..TriangleOptions::default()
        });
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        let vertices = &model.primitives[0].vertices;
        // Scale 2 then translate (10, 20, 30): the origin vertex lands on the translation, and the
        // unit-X vertex two units past it.
        assert_eq!(vertices[0].position, [10.0, 20.0, 30.0]);
        assert_eq!(vertices[1].position, [12.0, 20.0, 30.0]);
        assert_eq!(vertices[2].position, [10.0, 22.0, 30.0]);
        // A uniform scale leaves the renormalized normal unchanged.
        assert_eq!(vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(
            model.bounds(),
            Some(([10.0, 20.0, 30.0], [12.0, 22.0, 30.0]))
        );
    }

    #[test]
    fn generates_flat_normals_when_the_source_omits_them() {
        // Legal glTF: the spec says to compute them. Shipping zero normals to the GPU instead would
        // render the model unlit and look like a shader bug.
        let glb = triangle_glb(TriangleOptions {
            without_normals: true,
            ..TriangleOptions::default()
        });
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        for vertex in &model.primitives[0].vertices {
            assert_eq!(
                vertex.normal,
                [0.0, 0.0, 1.0],
                "a counter-clockwise XY triangle faces +Z"
            );
        }
    }

    #[test]
    fn defaults_texture_coordinates_when_the_source_omits_them() {
        let glb = triangle_glb(TriangleOptions {
            without_uvs: true,
            ..TriangleOptions::default()
        });
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        for vertex in &model.primitives[0].vertices {
            assert_eq!(vertex.uv, [0.0, 0.0]);
        }
    }

    #[test]
    fn synthesizes_indices_for_an_unindexed_primitive() {
        let glb = triangle_glb(TriangleOptions {
            without_indices: true,
            ..TriangleOptions::default()
        });
        let model = import_model(&glb, ModelLimits::default()).expect("import");
        assert_eq!(model.primitives[0].indices, [0, 1, 2]);
        assert_eq!(model.triangle_count(), 1);
    }

    #[test]
    fn refuses_a_non_triangle_topology() {
        let glb = triangle_glb(TriangleOptions {
            line_topology: true,
            ..TriangleOptions::default()
        });
        let error = import_model(&glb, ModelLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, ModelError::UnsupportedTopology { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_an_index_past_the_vertex_array() {
        let glb = triangle_glb(TriangleOptions {
            out_of_range_index: true,
            ..TriangleOptions::default()
        });
        let error = import_model(&glb, ModelLimits::default()).expect_err("must refuse");
        assert!(
            matches!(
                error,
                ModelError::IndexOutOfRange {
                    index: 99,
                    vertices: 3
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_a_vertex_count_past_the_limit() {
        let glb = triangle_glb(TriangleOptions::default());
        let limits = ModelLimits {
            maximum_vertices: 2,
            ..ModelLimits::default()
        };
        let error = import_model(&glb, limits).expect_err("must refuse");
        assert!(
            matches!(
                error,
                ModelError::LimitExceeded {
                    what: "vertex count",
                    ..
                }
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn refuses_bytes_that_are_not_a_gltf_container() {
        let error =
            import_model(b"certainly not a glb", ModelLimits::default()).expect_err("must refuse");
        assert!(matches!(error, ModelError::Gltf(_)), "got {error:?}");
    }

    #[test]
    fn refuses_a_truncated_container() {
        let glb = triangle_glb(TriangleOptions::default());
        for fraction in [2, 4, 8] {
            let cut = glb.len() / fraction;
            let error = import_model(&glb[..cut], ModelLimits::default())
                .expect_err("truncation must refuse");
            assert!(matches!(error, ModelError::Gltf(_)), "got {error:?}");
        }
    }
}

#[cfg(test)]
mod model_texture_tests {
    use cic_vfs::{Vfs, VirtualPath};

    use crate::model::resolve_model_textures;
    use crate::texture::{
        BlockFormat, TEXTURE_DIRECTORY, TextureAsset, TextureLimits, TextureResolveError,
    };
    use crate::{Model, ModelImage};

    /// A model carrying named images and nothing else: the sidecar lookup reads only the names.
    fn model(names: &[&str]) -> Model {
        Model {
            name: "fixture".to_owned(),
            primitives: Vec::new(),
            materials: Vec::new(),
            images: names
                .iter()
                .map(|name| ModelImage {
                    width: 1,
                    height: 1,
                    // A placeholder, which is exactly what an author leaves in the container once the
                    // sidecar exists.
                    rgba: vec![0, 0, 0, 255],
                    name: (*name).to_owned(),
                })
                .collect(),
            has_skin: false,
            has_animation: false,
        }
    }

    fn vfs(members: &[(&str, Vec<u8>)]) -> Vfs {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "textures",
            members
                .iter()
                .map(|(path, bytes)| (VirtualPath::new(path).expect("path"), bytes.clone()))
                .collect::<Vec<_>>(),
        )
        .expect("mount");
        vfs
    }

    fn dds(format: BlockFormat) -> Vec<u8> {
        TextureAsset::solid(8, 8, format, [u8::MAX; 4], TextureLimits::default())
            .expect("solid texture")
            .encode()
    }

    #[test]
    fn finds_a_sidecar_named_after_the_image_and_leaves_the_others_alone() {
        // The whole convention in one assertion: the glTF image's own name is the key, so the link is
        // authored in the DCC tool rather than derived from a filename the container may not carry.
        let model = model(&["hull_basecolor", "hull_normal"]);
        let vfs = vfs(&[(
            &format!("{TEXTURE_DIRECTORY}/hull_basecolor.dds"),
            dds(BlockFormat::Bc7UnormSrgb),
        )]);
        let textures =
            resolve_model_textures(&model, &vfs, TextureLimits::default()).expect("resolve");
        assert_eq!(textures.resolved_count(), 1);
        assert_eq!(
            textures.get(0).map(TextureAsset::format),
            Some(BlockFormat::Bc7UnormSrgb)
        );
        assert!(
            textures.get(1).is_none(),
            "an image with no sidecar keeps the pixels the container carried"
        );
    }

    #[test]
    fn a_model_with_no_sidecars_at_all_resolves_to_an_empty_table() {
        // The ordinary case for a model whose textures have not been converted, and it must not be an
        // error: the container's own images are a working answer.
        let textures = resolve_model_textures(
            &model(&["hull_basecolor"]),
            &vfs(&[("other/thing.dds", vec![1, 2, 3])]),
            TextureLimits::default(),
        )
        .expect("an absent sidecar is not a failure");
        assert!(textures.is_empty());
        assert_eq!(textures.resolved_count(), 0);
    }

    #[test]
    fn an_unnamed_image_is_never_looked_up() {
        // There is no key to look up, and guessing one from the image's position would silently give two
        // models the same texture.
        let textures = resolve_model_textures(
            &model(&[""]),
            &vfs(&[(".dds", dds(BlockFormat::Bc7Unorm))]),
            TextureLimits::default(),
        )
        .expect("resolve");
        assert!(textures.is_empty());
    }

    #[test]
    fn a_sidecar_that_exists_but_will_not_read_is_a_failure_rather_than_a_shrug() {
        // The distinction this function draws. An *absent* sidecar means "not converted yet"; a broken
        // one means a converted texture is being silently rendered from its placeholder, and a content
        // author needs telling rather than left to notice.
        let error = resolve_model_textures(
            &model(&["hull_basecolor"]),
            &vfs(&[(
                &format!("{TEXTURE_DIRECTORY}/hull_basecolor.dds"),
                b"not a dds at all".to_vec(),
            )]),
            TextureLimits::default(),
        )
        .expect_err("a malformed sidecar must be reported");
        assert!(
            matches!(&error, TextureResolveError::Texture { path, .. } if path.contains("hull_basecolor")),
            "got {error:?}"
        );
    }
}

#[cfg(test)]
mod terrain_texture_tests {
    use cic_vfs::{Vfs, VirtualPath};

    use crate::terrain::{Terrain, TerrainLayer, resolve_terrain_textures};
    use crate::texture::{
        BlockFormat, TEXTURE_DIRECTORY, TextureAsset, TextureLimits, TextureResolveError,
    };

    fn terrain(layers: &[&str]) -> Terrain {
        Terrain::new(
            4,
            4,
            10.0,
            0.5,
            vec![100u16; 16],
            layers
                .iter()
                .map(|name| TerrainLayer {
                    name: (*name).to_owned(),
                    weights: vec![255; 16],
                })
                .collect(),
        )
        .expect("valid terrain")
    }

    fn vfs(members: &[(&str, Vec<u8>)]) -> Vfs {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "textures",
            members
                .iter()
                .map(|(path, bytes)| (VirtualPath::new(path).expect("path"), bytes.clone()))
                .collect::<Vec<_>>(),
        )
        .expect("mount");
        vfs
    }

    fn dds(format: BlockFormat) -> Vec<u8> {
        TextureAsset::solid(16, 16, format, [u8::MAX; 4], TextureLimits::default())
            .expect("solid texture")
            .encode()
    }

    #[test]
    fn a_layer_is_textured_by_the_file_named_after_it() {
        // The layer name has always been the key -- the container carries names and weights, never
        // pixels, and the renderer has always resolved the name against a material set. This makes it
        // resolve against the package too, in layer order so it indexes alongside `Terrain::layers`.
        let terrain = terrain(&["grass", "rock", "sand"]);
        let vfs = vfs(&[
            (
                &format!("{TEXTURE_DIRECTORY}/grass.dds"),
                dds(BlockFormat::Bc7UnormSrgb),
            ),
            (
                &format!("{TEXTURE_DIRECTORY}/sand.dds"),
                dds(BlockFormat::Bc1RgbaUnormSrgb),
            ),
        ]);
        let textures =
            resolve_terrain_textures(&terrain, &vfs, TextureLimits::default()).expect("resolve");

        assert_eq!(textures.len(), 3, "one entry per layer, in layer order");
        assert_eq!(
            textures[0].as_ref().map(TextureAsset::format),
            Some(BlockFormat::Bc7UnormSrgb)
        );
        assert!(
            textures[1].is_none(),
            "a layer with no file renders as its palette colour, as it always has"
        );
        assert_eq!(
            textures[2].as_ref().map(TextureAsset::format),
            Some(BlockFormat::Bc1RgbaUnormSrgb),
            "the format is the file's, not a per-terrain choice"
        );
    }

    #[test]
    fn a_terrain_with_no_layers_or_no_files_resolves_to_nothing_rather_than_failing() {
        // Both are ordinary: an unconverted map, and a heightfield with no layer set at all.
        assert!(
            resolve_terrain_textures(&terrain(&[]), &vfs(&[]), TextureLimits::default())
                .expect("no layers is not a failure")
                .is_empty()
        );
        let textures = resolve_terrain_textures(
            &terrain(&["grass"]),
            &vfs(&[("elsewhere/grass.dds", dds(BlockFormat::Bc7UnormSrgb))]),
            TextureLimits::default(),
        )
        .expect("an absent file is not a failure");
        assert_eq!(textures, vec![None]);
    }

    #[test]
    fn a_layer_texture_that_will_not_read_is_reported_against_its_layer_name() {
        // The distinction: absent means "not converted"; broken means a converted texture is silently
        // rendering as a flat colour, and the error has to name which layer so it is actionable.
        let error = resolve_terrain_textures(
            &terrain(&["grass"]),
            &vfs(&[(
                &format!("{TEXTURE_DIRECTORY}/grass.dds"),
                b"not a dds".to_vec(),
            )]),
            TextureLimits::default(),
        )
        .expect_err("a malformed layer texture must be reported");
        assert!(
            matches!(&error, TextureResolveError::Texture { path, .. } if path.contains("grass")),
            "got {error:?}"
        );
    }
}

#[cfg(test)]
mod package_tests {
    // Every float compared here is an exactly-representable constant the fixtures set directly
    // (0.0, 1.0, 10.0, 0.9, ...), so exact comparison is the correct assertion -- an epsilon would
    // weaken these tests rather than make them robust.
    #![allow(clippy::float_cmp)]
    // Fixture sizes are small, known constants, so the width casts below cannot truncate.
    #![allow(clippy::cast_possible_truncation)]
    use crate::package::{MapPackage, PackageError, PackageLimits, SCENARIO_PATH};
    use crate::scenario::{
        FORMAT_VERSION, ObjectPlacement, PlayerSlot, Position, Scenario, TerrainReference, Waypoint,
    };
    use crate::terrain::{Terrain, TerrainLayer};

    /// Builds a zip the same way a packaging tool would, so the test exercises the real container.
    fn zip(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut directory = Vec::new();
        let mut offsets = Vec::new();
        for (name, payload) in members {
            offsets.push(body.len() as u32);
            let size = payload.len() as u32;
            body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            body.extend_from_slice(&20u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes()); // stored
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&size.to_le_bytes());
            body.extend_from_slice(&(name.len() as u16).to_le_bytes());
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(payload);
        }
        for ((name, payload), offset) in members.iter().zip(&offsets) {
            let size = payload.len() as u32;
            directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            directory.extend_from_slice(&20u16.to_le_bytes());
            directory.extend_from_slice(&20u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u32.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u16.to_le_bytes());
            directory.extend_from_slice(&0u32.to_le_bytes());
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = body.len() as u32;
        let directory_size = directory.len() as u32;
        let count = members.len() as u16;
        let mut archive = body;
        archive.extend_from_slice(&directory);
        archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&count.to_le_bytes());
        archive.extend_from_slice(&directory_size.to_le_bytes());
        archive.extend_from_slice(&directory_offset.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive
    }

    fn terrain() -> Terrain {
        // 11x11 samples at 10 units apart spans 100x100 world units.
        Terrain::new(
            11,
            11,
            10.0,
            0.25,
            vec![200; 121],
            vec![TerrainLayer {
                name: "grass".to_owned(),
                weights: vec![255; 121],
            }],
        )
        .expect("valid terrain")
    }

    fn scenario() -> Scenario {
        Scenario {
            format_version: FORMAT_VERSION,
            name: "Alpine".to_owned(),
            description: String::new(),
            terrain: TerrainReference {
                path: "terrain/alpine.cict".to_owned(),
            },
            players: vec![
                PlayerSlot {
                    id: "north".to_owned(),
                    name: "North".to_owned(),
                    faction: "vanguard".to_owned(),
                    start: Position {
                        x: 10.0,
                        y: 90.0,
                        z: 0.0,
                    },
                    team: 1,
                },
                PlayerSlot {
                    id: "south".to_owned(),
                    name: "South".to_owned(),
                    faction: "coalition".to_owned(),
                    start: Position {
                        x: 90.0,
                        y: 10.0,
                        z: 0.0,
                    },
                    team: 2,
                },
            ],
            objects: vec![ObjectPlacement {
                template: "prop/pine".to_owned(),
                position: Position {
                    x: 50.0,
                    y: 50.0,
                    z: 0.0,
                },
                rotation: 0.0,
                scale: 1.0,
                owner: Some("north".to_owned()),
            }],
            waypoints: vec![Waypoint {
                name: "centre".to_owned(),
                position: Position {
                    x: 50.0,
                    y: 50.0,
                    z: 0.0,
                },
            }],
        }
    }

    fn package_bytes(scenario: &Scenario, terrain: &Terrain) -> Vec<u8> {
        zip(&[
            (SCENARIO_PATH, scenario.to_json().expect("serialize")),
            ("terrain/alpine.cict", terrain.encode()),
            ("thumbnail.png", b"not really a png".to_vec()),
        ])
    }

    #[test]
    fn opens_a_package_and_resolves_both_halves() {
        let bytes = package_bytes(&scenario(), &terrain());
        let package = MapPackage::open(&bytes, PackageLimits::default()).expect("open");

        assert_eq!(package.scenario().name, "Alpine");
        assert_eq!(package.scenario().players.len(), 2);
        assert_eq!(package.terrain().width(), 11);
        assert_eq!(package.terrain().world_extent(), [100.0, 100.0]);
        assert_eq!(package.terrain().layers()[0].name, "grass");
        // 200 quantization steps at 0.25 units each.
        assert_eq!(package.terrain().elevation_at(5, 5), Some(50.0));
        assert_eq!(
            package.thumbnail(1_024).expect("thumbnail"),
            Some(b"not really a png".to_vec())
        );
    }

    #[test]
    fn a_layer_texture_travels_in_the_package_and_resolves_through_its_mount() {
        // The whole chain a map actually uses: a zip holding a scenario, a terrain declaring a layer
        // named `grass`, and `textures/grass.dds` beside them. Opening the package mounts it, and the
        // layer name resolves against that mount -- so nothing in between has to be told where textures
        // live.
        use crate::terrain::resolve_terrain_textures;
        use crate::texture::{BlockFormat, TextureAsset, TextureLimits};

        let grass = TextureAsset::solid(
            16,
            16,
            BlockFormat::Bc7UnormSrgb,
            [201, 199, 87, 255],
            TextureLimits::default(),
        )
        .expect("solid texture");
        let bytes = zip(&[
            (SCENARIO_PATH, scenario().to_json().expect("serialize")),
            ("terrain/alpine.cict", terrain().encode()),
            ("textures/grass.dds", grass.encode()),
        ]);
        let package = MapPackage::open(&bytes, PackageLimits::default()).expect("open");
        assert_eq!(package.terrain().layers()[0].name, "grass");

        let textures = resolve_terrain_textures(
            package.terrain(),
            package.contents(),
            TextureLimits::default(),
        )
        .expect("resolve through the package mount");
        assert_eq!(textures.len(), 1, "one entry per layer");
        assert_eq!(
            textures[0].as_ref(),
            Some(&grass),
            "the texture that came out is the one that went in"
        );
    }

    #[test]
    fn reports_a_missing_scenario() {
        let bytes = zip(&[("terrain/alpine.cict", terrain().encode())]);
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, PackageError::MissingMember(SCENARIO_PATH)),
            "got {error:?}"
        );
    }

    #[test]
    fn reports_a_scenario_naming_terrain_the_package_lacks() {
        let mut scenario = scenario();
        scenario.terrain.path = "terrain/absent.cict".to_owned();
        let bytes = zip(&[(SCENARIO_PATH, scenario.to_json().expect("serialize"))]);
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(
            matches!(error, PackageError::MissingTerrain(ref path) if path == "terrain/absent.cict"),
            "got {error:?}"
        );
    }

    #[test]
    fn reports_a_player_start_outside_the_terrain() {
        // The cross-check neither format can make alone. 100x100 world, start at 500.
        let mut scenario = scenario();
        scenario.players[0].start.x = 500.0;
        let bytes = package_bytes(&scenario, &terrain());
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(
            matches!(
                &error,
                PackageError::OutsideTerrain { what, extent, .. }
                    if what.contains("north") && *extent == [100.0, 100.0]
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn reports_an_object_and_a_waypoint_outside_the_terrain() {
        let mut misplaced_object = scenario();
        misplaced_object.objects[0].position.y = -5.0;
        let bytes = package_bytes(&misplaced_object, &terrain());
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(
            matches!(&error, PackageError::OutsideTerrain { what, .. } if what.contains("prop/pine")),
            "got {error:?}"
        );

        let mut misplaced_waypoint = scenario();
        misplaced_waypoint.waypoints[0].position.x = 101.0;
        let bytes = package_bytes(&misplaced_waypoint, &terrain());
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(
            matches!(&error, PackageError::OutsideTerrain { what, .. } if what.contains("centre")),
            "got {error:?}"
        );
    }

    #[test]
    fn accepts_a_position_exactly_on_the_terrain_boundary() {
        let mut scenario = scenario();
        scenario.players[0].start = Position {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        scenario.players[1].start = Position {
            x: 100.0,
            y: 100.0,
            z: 0.0,
        };
        let bytes = package_bytes(&scenario, &terrain());
        MapPackage::open(&bytes, PackageLimits::default()).expect("boundary is inclusive");
    }

    #[test]
    fn reports_a_malformed_terrain_container() {
        let bytes = zip(&[
            (SCENARIO_PATH, scenario().to_json().expect("serialize")),
            ("terrain/alpine.cict", b"not a terrain".to_vec()),
        ]);
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(matches!(error, PackageError::Terrain(_)), "got {error:?}");
    }

    #[test]
    fn reports_a_malformed_scenario_document() {
        let bytes = zip(&[
            (SCENARIO_PATH, b"{ not json".to_vec()),
            ("terrain/alpine.cict", terrain().encode()),
        ]);
        let error = MapPackage::open(&bytes, PackageLimits::default()).expect_err("must refuse");
        assert!(matches!(error, PackageError::Scenario(_)), "got {error:?}");
    }

    #[test]
    fn reports_bytes_that_are_not_a_zip() {
        let error =
            MapPackage::open(b"not a zip", PackageLimits::default()).expect_err("must refuse");
        assert!(matches!(error, PackageError::Mount(_)), "got {error:?}");
    }

    #[test]
    fn a_thumbnail_is_optional() {
        let bytes = zip(&[
            (SCENARIO_PATH, scenario().to_json().expect("serialize")),
            ("terrain/alpine.cict", terrain().encode()),
        ]);
        let package = MapPackage::open(&bytes, PackageLimits::default()).expect("open");
        assert_eq!(package.thumbnail(1_024).expect("thumbnail"), None);
    }
}

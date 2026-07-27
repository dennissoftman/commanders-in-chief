//! Native asset formats: glTF models, the terrain container, and the map package.
//!
//! # The format decisions, in one place
//!
//! | Data | Format | Why |
//! |---|---|---|
//! | Models, props, units | **glTF 2.0** (`.glb`) | A published standard every DCC tool exports. Writing a mesh format would mean writing a Blender exporter first. |
//! | Terrain heightfield and layers | **custom chunked binary** (`.cict`) | A regular numeric grid. No standard describes one well, and `u16` elevations upload directly as an `R16Unorm` GPU texture. |
//! | Scenario: placements, players, waypoints | **JSON** (`map.json`) | Diffable, reviewable, hand-fixable. The bulk numerics are elsewhere, so the size argument for a binary encoding does not apply. |
//! | A whole map | **zip** (`.cicmap`) | Already has a directory, per-member compression, and universal tooling. |
//!
//! Every decoder here is bounded and total: it takes explicit limits, refuses rather than allocates
//! when one is crossed, and reports a structured error naming what it found and what it expected.
//! Nothing panics on hostile input.

pub mod model;
pub mod package;
pub mod scenario;
pub mod terrain;
#[cfg(test)]
mod testing;

pub use model::{
    Model, ModelError, ModelLimits, ModelMaterial, ModelPrimitive, ModelVertex, import_model,
};
pub use package::{MapPackage, PackageError, PackageLimits};
pub use scenario::{
    ObjectPlacement, PlayerSlot, Position, Scenario, ScenarioError, TerrainReference, Waypoint,
};
pub use terrain::{Terrain, TerrainError, TerrainLayer, TerrainLimits, decode_terrain};

#[cfg(test)]
mod model_tests {
    // Every float compared here is an exactly-representable constant the fixtures set directly
    // (0.0, 1.0, 10.0, 0.9, ...), so exact comparison is the correct assertion -- an epsilon would
    // weaken these tests rather than make them robust.
    #![allow(clippy::float_cmp)]
    use crate::model::{ModelError, ModelLimits, import_model};
    use crate::testing::{TriangleOptions, triangle_glb};

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
        assert!(!material.blended);
        assert_eq!(material.base_color_texture, None);
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

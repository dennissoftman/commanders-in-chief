//! The map package: one zip holding one map's authored and bulk data together.
//!
//! # Why a zip rather than one bespoke container
//!
//! A map is not one kind of data. It is a small diffable description, a large numeric grid, and some
//! number of binary assets. A single custom container would have to reinvent a directory, per-member
//! compression, and a tool ecosystem — whereas zip already has all three, and a designer can open a
//! map in any file manager to see what is inside it.
//!
//! ```text
//! alpine.cicmap  (a zip)
//!   map.json                 the scenario -- stored uncompressed so diff tools can reach it
//!   terrain/alpine.cict      the heightfield container
//!   models/*.glb             map-specific models, if any
//!   thumbnail.png            lobby preview, optional
//! ```
//!
//! Loading goes through [`cic_vfs`], so a package mounted after base content overrides it by the
//! same last-mounted-wins rule as everything else — and a member named `../../etc/passwd` is refused
//! at mount time rather than here.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_vfs::{ArchiveLimits, MountError, ResourceReadError, Vfs, VirtualPath};

use crate::scenario::{Scenario, ScenarioError};
use crate::terrain::{Terrain, TerrainError, TerrainLimits, decode_terrain};

/// Package-relative path of the scenario document.
pub const SCENARIO_PATH: &str = "map.json";

/// Package-relative path of the optional lobby thumbnail.
pub const THUMBNAIL_PATH: &str = "thumbnail.png";

/// Explicit bounds applied while opening an untrusted package.
#[derive(Debug, Clone, Copy)]
pub struct PackageLimits {
    /// Bounds for the zip container itself.
    pub archive: ArchiveLimits,
    /// Bounds for the terrain container inside it.
    pub terrain: TerrainLimits,
    /// Maximum bytes read for the scenario document.
    pub maximum_scenario_bytes: usize,
    /// Maximum bytes read for the terrain container.
    pub maximum_terrain_bytes: usize,
    /// Maximum bytes read for one script member.
    pub maximum_script_bytes: usize,
}

impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            archive: ArchiveLimits::default(),
            terrain: TerrainLimits::default(),
            maximum_scenario_bytes: 16 * 1_024 * 1_024,
            maximum_terrain_bytes: 512 * 1_024 * 1_024,
            // A script is hand-written text. The language's own parse limit is a megabyte of source,
            // so anything larger is not a script that was going to compile.
            maximum_script_bytes: 1_024 * 1_024,
        }
    }
}

/// An opened map package with its scenario and terrain resolved.
#[derive(Debug)]
pub struct MapPackage {
    scenario: Scenario,
    terrain: Terrain,
    vfs: Vfs,
}

impl MapPackage {
    /// Opens a package from zip bytes, decoding and cross-checking its scenario and terrain.
    ///
    /// # Errors
    ///
    /// Returns a structured [`PackageError`] when the container will not mount, the scenario or
    /// terrain member is absent or malformed, or the scenario's declared player starts fall outside
    /// the terrain it names.
    pub fn open(bytes: &[u8], limits: PackageLimits) -> Result<Self, PackageError> {
        let mut vfs = Vfs::new();
        vfs.mount_zip_bytes("package", bytes, limits.archive)
            .map_err(PackageError::Mount)?;

        let scenario_path = virtual_path(SCENARIO_PATH)?;
        let scenario_bytes = vfs
            .resolve(&scenario_path)
            .ok_or(PackageError::MissingMember(SCENARIO_PATH))?
            .read(limits.maximum_scenario_bytes)
            .map_err(|error| PackageError::Read {
                member: SCENARIO_PATH.to_owned(),
                error,
            })?;
        let scenario = Scenario::from_json(&scenario_bytes).map_err(PackageError::Scenario)?;

        let terrain_path = virtual_path(&scenario.terrain.path)?;
        let terrain_bytes = vfs
            .resolve(&terrain_path)
            .ok_or_else(|| PackageError::MissingTerrain(scenario.terrain.path.clone()))?
            .read(limits.maximum_terrain_bytes)
            .map_err(|error| PackageError::Read {
                member: scenario.terrain.path.clone(),
                error,
            })?;
        let terrain =
            decode_terrain(&terrain_bytes, limits.terrain).map_err(PackageError::Terrain)?;

        let package = Self {
            scenario,
            terrain,
            vfs,
        };
        package.check_placements_are_on_the_terrain()?;
        Ok(package)
    }

    /// Returns the authored scenario.
    #[must_use]
    pub const fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// Returns the decoded terrain.
    #[must_use]
    pub const fn terrain(&self) -> &Terrain {
        &self.terrain
    }

    /// Returns the package's mounted contents, for reading models and other members.
    #[must_use]
    pub const fn contents(&self) -> &Vfs {
        &self.vfs
    }

    /// Reads the lobby thumbnail, if the package carries one.
    ///
    /// # Errors
    ///
    /// Returns [`PackageError::Read`] when the member exists but cannot be read within `maximum`.
    pub fn thumbnail(&self, maximum: usize) -> Result<Option<Vec<u8>>, PackageError> {
        let path = virtual_path(THUMBNAIL_PATH)?;
        match self.vfs.resolve(&path) {
            None => Ok(None),
            Some(entry) => entry
                .read(maximum)
                .map(Some)
                .map_err(|error| PackageError::Read {
                    member: THUMBNAIL_PATH.to_owned(),
                    error,
                }),
        }
    }

    /// Reads the scenario's scripts, in authored order, as source text.
    ///
    /// The paths come from the scenario's `scripts` array and nothing else: there is no directory
    /// scan, so a script the scenario does not name is not read however it got into the archive. The
    /// order is preserved because it is the dispatch order — see
    /// [ADR 7002](../../../docs/adr/7002-script-events.md).
    ///
    /// Returns the paths alongside their text so a compile diagnostic can name the file. Compiling is
    /// deliberately not done here: this crate knows nothing about the language, and the interface a
    /// script compiles against belongs to whatever is going to run it.
    ///
    /// # Errors
    ///
    /// Returns [`PackageError::MissingScript`] for a listed path the package does not contain,
    /// [`PackageError::Read`] for a member that will not read within the limit, and
    /// [`PackageError::ScriptEncoding`] for one that is not UTF-8.
    pub fn scripts(&self, limits: PackageLimits) -> Result<Vec<(String, String)>, PackageError> {
        let mut sources = Vec::with_capacity(self.scenario.scripts.len());
        for declared in &self.scenario.scripts {
            let path = virtual_path(declared)?;
            let bytes = self
                .vfs
                .resolve(&path)
                .ok_or_else(|| PackageError::MissingScript(declared.clone()))?
                .read(limits.maximum_script_bytes)
                .map_err(|error| PackageError::Read {
                    member: declared.clone(),
                    error,
                })?;
            // Refused rather than replaced with U+FFFD: a lossy conversion turns a mis-encoded file
            // into a compile error somewhere else, pointing at a character the author never wrote.
            let source = String::from_utf8(bytes).map_err(|_| PackageError::ScriptEncoding {
                member: declared.clone(),
            })?;
            sources.push((declared.clone(), source));
        }
        Ok(sources)
    }

    /// Verifies every authored position sits within the terrain's world extent.
    ///
    /// This is the cross-check neither format can do alone: the scenario knows where things are, the
    /// terrain knows how big the world is, and only the package sees both. Catching it here turns a
    /// unit spawning in the void into a load-time error.
    fn check_placements_are_on_the_terrain(&self) -> Result<(), PackageError> {
        let [extent_x, extent_y] = self.terrain.world_extent();
        let check = |x: f32, y: f32, what: &str| -> Result<(), PackageError> {
            if x < 0.0 || y < 0.0 || x > extent_x || y > extent_y {
                return Err(PackageError::OutsideTerrain {
                    what: what.to_owned(),
                    position: [x, y],
                    extent: [extent_x, extent_y],
                });
            }
            Ok(())
        };
        for player in &self.scenario.players {
            check(
                player.start.x,
                player.start.y,
                &format!("player {} start", player.id),
            )?;
        }
        for (index, object) in self.scenario.objects.iter().enumerate() {
            check(
                object.position.x,
                object.position.y,
                &format!("object {index} ({})", object.template),
            )?;
        }
        for waypoint in &self.scenario.waypoints {
            check(
                waypoint.position.x,
                waypoint.position.y,
                &format!("waypoint {}", waypoint.name),
            )?;
        }
        Ok(())
    }
}

fn virtual_path(text: &str) -> Result<VirtualPath, PackageError> {
    VirtualPath::new(text).map_err(|error| PackageError::MemberPath {
        path: text.to_owned(),
        error,
    })
}

/// A structured failure while opening a map package.
#[derive(Debug)]
pub enum PackageError {
    /// The zip container would not mount.
    Mount(MountError),
    /// A member's declared path could not become a safe virtual path.
    MemberPath {
        /// The offending path text.
        path: String,
        /// The underlying normalization failure.
        error: cic_vfs::PathError,
    },
    /// A required member was absent.
    MissingMember(&'static str),
    /// The scenario named a terrain member the package does not contain.
    MissingTerrain(String),
    /// The scenario named a script member the package does not contain.
    MissingScript(String),
    /// A script member was not valid UTF-8.
    ScriptEncoding {
        /// Package-relative member path.
        member: String,
    },
    /// A member existed but could not be read.
    Read {
        /// Package-relative member path.
        member: String,
        /// The underlying read failure.
        error: ResourceReadError,
    },
    /// The scenario document was invalid.
    Scenario(ScenarioError),
    /// The terrain container was invalid.
    Terrain(TerrainError),
    /// An authored position fell outside the terrain's world extent.
    OutsideTerrain {
        /// What was misplaced.
        what: String,
        /// Its authored XY position.
        position: [f32; 2],
        /// The terrain's world extent.
        extent: [f32; 2],
    },
}

impl Display for PackageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mount(error) => Display::fmt(error, formatter),
            Self::MemberPath { path, error } => {
                write!(formatter, "member path {path}: {error}")
            }
            Self::MissingMember(member) => {
                write!(formatter, "package is missing its {member} member")
            }
            Self::MissingTerrain(path) => write!(
                formatter,
                "the scenario names terrain {path}, which the package does not contain"
            ),
            Self::MissingScript(path) => write!(
                formatter,
                "the scenario names script {path}, which the package does not contain"
            ),
            Self::ScriptEncoding { member } => {
                write!(formatter, "script {member} is not valid UTF-8")
            }
            Self::Read { member, error } => write!(formatter, "{member}: {error}"),
            Self::Scenario(error) => Display::fmt(error, formatter),
            Self::Terrain(error) => Display::fmt(error, formatter),
            Self::OutsideTerrain {
                what,
                position,
                extent,
            } => write!(
                formatter,
                "{what} at ({}, {}) is outside the terrain extent ({}, {})",
                position[0], position[1], extent[0], extent[1]
            ),
        }
    }
}

impl Error for PackageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Mount(error) => Some(error),
            Self::MemberPath { error, .. } => Some(error),
            Self::Read { error, .. } => Some(error),
            Self::Scenario(error) => Some(error),
            Self::Terrain(error) => Some(error),
            _ => None,
        }
    }
}

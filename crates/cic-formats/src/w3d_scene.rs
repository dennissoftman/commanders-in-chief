//! W3D hierarchy, HLOD composition, and classic raw-animation decoding.
//!
//! Provenance: authored for Commanders in Chief from `w3d_file.h`, `htree.cpp`,
//! `hlod.cpp`, `hrawanim.cpp`, and `motchan.cpp` at `GeneralsGameCode` revision
//! `9f7abb866f5afd446db14149979e744c7216baaf`. Those sources are GPL-3.0-or-later
//! with Electronic Arts Section 7 terms; no source code or retail content is copied.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use cic_core::{BinaryError, BinaryReader};

use crate::{W3dChunk, W3dFile, W3dMeshError, W3dMeshLimits, W3dStaticMesh, decode_static_mesh};

const HIERARCHY: u32 = 0x100;
const HIERARCHY_HEADER: u32 = 0x101;
const PIVOTS: u32 = 0x102;
const ANIMATION: u32 = 0x200;
const ANIMATION_HEADER: u32 = 0x201;
const ANIMATION_CHANNEL: u32 = 0x202;
const HLOD: u32 = 0x700;
const HLOD_HEADER: u32 = 0x701;
const HLOD_ARRAY: u32 = 0x702;
const HLOD_ARRAY_HEADER: u32 = 0x703;
const HLOD_SUB_OBJECT: u32 = 0x704;
const BOX: u32 = 0x740;
const MESH: u32 = 0;

const HIERARCHY_HEADER_BYTES: usize = 36;
const PIVOT_BYTES: usize = 60;
const ANIMATION_HEADER_BYTES: usize = 44;
const ANIMATION_CHANNEL_HEADER_BYTES: usize = 12;
const HLOD_HEADER_BYTES: usize = 40;
const HLOD_ARRAY_HEADER_BYTES: usize = 8;
const HLOD_SUB_OBJECT_BYTES: usize = 36;

/// Allocation and recursion-independent limits for a composed W3D model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3dSceneLimits {
    pub maximum_pivots: usize,
    pub maximum_animations: usize,
    pub maximum_animation_frames: usize,
    pub maximum_animation_channels: usize,
    pub maximum_lods: usize,
    pub maximum_sub_objects_per_lod: usize,
}

impl Default for W3dSceneLimits {
    fn default() -> Self {
        Self {
            maximum_pivots: 65_536,
            maximum_animations: 4_096,
            maximum_animation_frames: 1_000_000,
            maximum_animation_channels: 1_000_000,
            maximum_lods: 256,
            maximum_sub_objects_per_lod: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct W3dQuaternion([f32; 4]);

impl W3dQuaternion {
    #[must_use]
    pub const fn components(self) -> [f32; 4] {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dPivot {
    name: Vec<u8>,
    parent: Option<u32>,
    translation: [f32; 3],
    rotation: W3dQuaternion,
}

impl W3dPivot {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub const fn parent(&self) -> Option<u32> {
        self.parent
    }
    #[must_use]
    pub const fn translation(&self) -> [f32; 3] {
        self.translation
    }
    #[must_use]
    pub const fn rotation(&self) -> W3dQuaternion {
        self.rotation
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dHierarchy {
    version: u32,
    name: Vec<u8>,
    center: [f32; 3],
    pivots: Vec<W3dPivot>,
}

impl W3dHierarchy {
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub const fn center(&self) -> [f32; 3] {
        self.center
    }
    #[must_use]
    pub fn pivots(&self) -> &[W3dPivot] {
        &self.pivots
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum W3dAnimationChannelKind {
    X,
    Y,
    Z,
    XRotation,
    YRotation,
    ZRotation,
    Quaternion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dAnimationChannel {
    first_frame: u16,
    last_frame: u16,
    pivot: u16,
    kind: W3dAnimationChannelKind,
    vector_length: u16,
    values: Vec<f32>,
}

impl W3dAnimationChannel {
    #[must_use]
    pub const fn first_frame(&self) -> u16 {
        self.first_frame
    }
    #[must_use]
    pub const fn last_frame(&self) -> u16 {
        self.last_frame
    }
    #[must_use]
    pub const fn pivot(&self) -> u16 {
        self.pivot
    }
    #[must_use]
    pub const fn kind(&self) -> W3dAnimationChannelKind {
        self.kind
    }
    #[must_use]
    pub const fn vector_length(&self) -> u16 {
        self.vector_length
    }
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dAnimation {
    version: u32,
    name: Vec<u8>,
    hierarchy_name: Vec<u8>,
    frame_count: u32,
    frame_rate: u32,
    channels: Vec<W3dAnimationChannel>,
}

impl W3dAnimation {
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub fn hierarchy_name_bytes(&self) -> &[u8] {
        &self.hierarchy_name
    }
    #[must_use]
    pub const fn frame_count(&self) -> u32 {
        self.frame_count
    }
    #[must_use]
    pub const fn frame_rate(&self) -> u32 {
        self.frame_rate
    }
    #[must_use]
    pub fn channels(&self) -> &[W3dAnimationChannel] {
        &self.channels
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dSubObject {
    bone_index: u32,
    name: Vec<u8>,
}

impl W3dSubObject {
    #[must_use]
    pub const fn bone_index(&self) -> u32 {
        self.bone_index
    }
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dLod {
    maximum_screen_size: f32,
    sub_objects: Vec<W3dSubObject>,
}

impl W3dLod {
    #[must_use]
    pub const fn maximum_screen_size(&self) -> f32 {
        self.maximum_screen_size
    }
    #[must_use]
    pub fn sub_objects(&self) -> &[W3dSubObject] {
        &self.sub_objects
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct W3dHlod {
    name: Vec<u8>,
    hierarchy_name: Vec<u8>,
    lods: Vec<W3dLod>,
}

impl W3dHlod {
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }
    #[must_use]
    pub fn hierarchy_name_bytes(&self) -> &[u8] {
        &self.hierarchy_name
    }
    #[must_use]
    pub fn lods(&self) -> &[W3dLod] {
        &self.lods
    }
}

/// One decoded mesh and the hierarchy pivot selected by the highest-detail HLOD.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dModelMesh {
    mesh: W3dStaticMesh,
    pivot: u32,
}

impl W3dModelMesh {
    #[must_use]
    pub const fn mesh(&self) -> &W3dStaticMesh {
        &self.mesh
    }
    #[must_use]
    pub const fn pivot(&self) -> u32 {
        self.pivot
    }
}

/// Complete preview-grade W3D model composition.
#[derive(Debug, Clone, PartialEq)]
pub struct W3dModel {
    hierarchy: W3dHierarchy,
    hlod: W3dHlod,
    meshes: Vec<W3dModelMesh>,
    animations: Vec<W3dAnimation>,
}

impl W3dModel {
    #[must_use]
    pub const fn hierarchy(&self) -> &W3dHierarchy {
        &self.hierarchy
    }
    #[must_use]
    pub const fn hlod(&self) -> &W3dHlod {
        &self.hlod
    }
    #[must_use]
    pub fn meshes(&self) -> &[W3dModelMesh] {
        &self.meshes
    }
    #[must_use]
    pub fn animations(&self) -> &[W3dAnimation] {
        &self.animations
    }
}

/// Decodes the hierarchy, highest-detail HLOD composition, referenced meshes, and raw animations.
///
/// # Errors
///
/// Returns a structured error when required chunks are absent or duplicated, declared sizes or
/// relationships are invalid, configured limits are exceeded, or a referenced mesh is malformed.
pub fn decode_w3d_model(
    file: &W3dFile,
    mesh_limits: W3dMeshLimits,
    scene_limits: W3dSceneLimits,
) -> Result<W3dModel, W3dSceneError> {
    decode_w3d_model_set(&[file], mesh_limits, scene_limits)
}

/// Returns the hierarchy resource name referenced by a model's HLOD, if present.
///
/// # Errors
///
/// Returns a structured error when an HLOD or its fixed-size header is malformed or duplicated.
pub fn w3d_model_hierarchy_name(file: &W3dFile) -> Result<Option<Vec<u8>>, W3dSceneError> {
    let Some(hlod) = unique_top(file.chunks(), HLOD)? else {
        return Ok(None);
    };
    let children = container(hlod)?;
    let header = data(required(children, HLOD_HEADER)?)?;
    exact(HLOD_HEADER, header.len(), HLOD_HEADER_BYTES)?;
    let mut reader = BinaryReader::new(header, "W3D HLOD header");
    reader.skip(8)?;
    reader.skip(16)?;
    Ok(Some(fixed_name(read_array::<16>(&mut reader)?)))
}

/// Composes a model from a primary W3D and separate hierarchy or animation W3D files.
///
/// Files are searched in the supplied stable order. Exactly one hierarchy and HLOD must be
/// present; meshes and matching classic animations may be distributed across the set.
///
/// # Errors
///
/// Returns a structured error when required chunks are absent or duplicated, declared sizes or
/// relationships are invalid, configured limits are exceeded, or a referenced mesh is malformed.
pub fn decode_w3d_model_set(
    files: &[&W3dFile],
    mesh_limits: W3dMeshLimits,
    scene_limits: W3dSceneLimits,
) -> Result<W3dModel, W3dSceneError> {
    let hierarchy_chunk =
        unique_top_set(files, HIERARCHY)?.ok_or(W3dSceneError::MissingChunk(HIERARCHY))?;
    let hierarchy = decode_hierarchy(hierarchy_chunk, scene_limits)?;
    let hlod_chunk = unique_top_set(files, HLOD)?.ok_or(W3dSceneError::MissingChunk(HLOD))?;
    let hlod = decode_hlod(hlod_chunk, hierarchy.pivots.len(), scene_limits)?;
    if !ascii_eq(&hlod.hierarchy_name, &hierarchy.name) {
        return Err(W3dSceneError::HierarchyNameMismatch);
    }
    let selected = hlod.lods.last().ok_or(W3dSceneError::EmptyHlod)?;
    let mut meshes = Vec::with_capacity(selected.sub_objects.len());
    for sub_object in &selected.sub_objects {
        let chunk = files.iter().flat_map(|file| file.chunks()).find(|chunk| {
            chunk.id() == MESH
                && mesh_full_name(chunk).is_some_and(|name| ascii_eq(&name, &sub_object.name))
        });
        let Some(chunk) = chunk else {
            if files.iter().flat_map(|file| file.chunks()).any(|chunk| {
                chunk.id() == BOX
                    && render_object_name(chunk)
                        .is_some_and(|name| ascii_eq(&name, &sub_object.name))
            }) {
                continue;
            }
            return Err(W3dSceneError::MissingMesh(sub_object.name.clone()));
        };
        let mesh = decode_static_mesh(chunk, mesh_limits)?;
        if let Some(bones) = mesh.vertex_bones() {
            for (vertex, bone) in bones.iter().copied().enumerate() {
                if usize::from(bone) >= hierarchy.pivots.len() {
                    return Err(W3dSceneError::BoneIndexOutOfRange {
                        vertex,
                        index: u32::from(bone),
                        count: hierarchy.pivots.len(),
                    });
                }
            }
        }
        meshes.push(W3dModelMesh {
            mesh,
            pivot: sub_object.bone_index,
        });
    }
    let mut animations = Vec::new();
    for chunk in files
        .iter()
        .flat_map(|file| file.chunks())
        .filter(|chunk| chunk.id() == ANIMATION)
    {
        if animations.len() >= scene_limits.maximum_animations {
            return Err(limit(
                "W3D animation count",
                animations.len().saturating_add(1),
                scene_limits.maximum_animations,
            ));
        }
        let animation = decode_animation(chunk, hierarchy.pivots.len(), scene_limits)?;
        if ascii_eq(&animation.hierarchy_name, &hierarchy.name) {
            animations.push(animation);
        }
    }
    Ok(W3dModel {
        hierarchy,
        hlod,
        meshes,
        animations,
    })
}

fn unique_top_set<'a>(
    files: &'a [&W3dFile],
    id: u32,
) -> Result<Option<&'a W3dChunk>, W3dSceneError> {
    let mut matching = files
        .iter()
        .flat_map(|file| file.chunks())
        .filter(|chunk| chunk.id() == id);
    let first = matching.next();
    if matching.next().is_some() {
        Err(W3dSceneError::DuplicateChunk(id))
    } else {
        Ok(first)
    }
}

fn decode_hierarchy(
    chunk: &W3dChunk,
    limits: W3dSceneLimits,
) -> Result<W3dHierarchy, W3dSceneError> {
    let children = container(chunk)?;
    let header = data(required(children, HIERARCHY_HEADER)?)?;
    exact(HIERARCHY_HEADER, header.len(), HIERARCHY_HEADER_BYTES)?;
    let mut reader = BinaryReader::new(header, "W3D hierarchy header");
    let version = reader.read_u32_le()?;
    let name = fixed_name(read_array::<16>(&mut reader)?);
    let count = limited(
        reader.read_u32_le()?,
        "W3D pivot count",
        limits.maximum_pivots,
    )?;
    if count == 0 {
        return Err(W3dSceneError::EmptyHierarchy);
    }
    let center = read_vec3(&mut reader, "hierarchy center", 0)?;
    let bytes = data(required(children, PIVOTS)?)?;
    exact(
        PIVOTS,
        bytes.len(),
        size(count, PIVOT_BYTES, "pivot array")?,
    )?;
    let mut reader = BinaryReader::new(bytes, "W3D pivots");
    let mut pivots = Vec::with_capacity(count);
    for index in 0..count {
        let pivot_name = fixed_name(read_array::<16>(&mut reader)?);
        let raw_parent = reader.read_u32_le()?;
        let parent = if raw_parent == u32::MAX {
            None
        } else {
            Some(raw_parent)
        };
        if (index == 0) != parent.is_none()
            || parent.is_some_and(|parent| {
                usize::try_from(parent).map_or(true, |parent| parent >= index)
            })
        {
            return Err(W3dSceneError::InvalidParent {
                pivot: index,
                parent: raw_parent,
            });
        }
        let translation = read_vec3(&mut reader, "pivot translation", index)?;
        let _euler = read_vec3(&mut reader, "pivot Euler angles", index)?;
        let rotation = W3dQuaternion(read_floats::<4>(&mut reader, "pivot quaternion", index)?);
        pivots.push(W3dPivot {
            name: pivot_name,
            parent,
            translation,
            rotation,
        });
    }
    Ok(W3dHierarchy {
        version,
        name,
        center,
        pivots,
    })
}

fn decode_animation(
    chunk: &W3dChunk,
    pivot_count: usize,
    limits: W3dSceneLimits,
) -> Result<W3dAnimation, W3dSceneError> {
    let children = container(chunk)?;
    let header = data(required(children, ANIMATION_HEADER)?)?;
    exact(ANIMATION_HEADER, header.len(), ANIMATION_HEADER_BYTES)?;
    let mut reader = BinaryReader::new(header, "W3D animation header");
    let version = reader.read_u32_le()?;
    let name = fixed_name(read_array::<16>(&mut reader)?);
    let hierarchy_name = fixed_name(read_array::<16>(&mut reader)?);
    let frame_count = reader.read_u32_le()?;
    if frame_count == 0 {
        return Err(W3dSceneError::ZeroFrameCount);
    }
    limited(
        frame_count,
        "W3D animation frame count",
        limits.maximum_animation_frames,
    )?;
    let frame_rate = reader.read_u32_le()?;
    if frame_rate == 0 {
        return Err(W3dSceneError::ZeroFrameRate);
    }
    let mut channels = Vec::new();
    for child in children
        .iter()
        .filter(|child| child.id() == ANIMATION_CHANNEL)
    {
        if channels.len() >= limits.maximum_animation_channels {
            return Err(limit(
                "W3D animation channel count",
                channels.len().saturating_add(1),
                limits.maximum_animation_channels,
            ));
        }
        channels.push(decode_animation_channel(child, pivot_count, frame_count)?);
    }
    Ok(W3dAnimation {
        version,
        name,
        hierarchy_name,
        frame_count,
        frame_rate,
        channels,
    })
}

fn decode_animation_channel(
    chunk: &W3dChunk,
    pivot_count: usize,
    frame_count: u32,
) -> Result<W3dAnimationChannel, W3dSceneError> {
    let bytes = data(chunk)?;
    if bytes.len() < ANIMATION_CHANNEL_HEADER_BYTES {
        return Err(W3dSceneError::InvalidLength {
            id: ANIMATION_CHANNEL,
            actual: bytes.len(),
            expected: ANIMATION_CHANNEL_HEADER_BYTES,
        });
    }
    let mut reader = BinaryReader::new(bytes, "W3D animation channel");
    let first_frame = reader.read_u16_le()?;
    let last_frame = reader.read_u16_le()?;
    let vector_length = reader.read_u16_le()?;
    let raw_kind = reader.read_u16_le()?;
    let pivot = reader.read_u16_le()?;
    reader.skip(2)?;
    if first_frame > last_frame || u32::from(last_frame) >= frame_count {
        return Err(W3dSceneError::InvalidFrameRange {
            first: first_frame,
            last: last_frame,
            count: frame_count,
        });
    }
    if usize::from(pivot) >= pivot_count {
        return Err(W3dSceneError::BoneIndexOutOfRange {
            vertex: 0,
            index: u32::from(pivot),
            count: pivot_count,
        });
    }
    let kind = match raw_kind {
        0 => W3dAnimationChannelKind::X,
        1 => W3dAnimationChannelKind::Y,
        2 => W3dAnimationChannelKind::Z,
        3 => W3dAnimationChannelKind::XRotation,
        4 => W3dAnimationChannelKind::YRotation,
        5 => W3dAnimationChannelKind::ZRotation,
        6 => W3dAnimationChannelKind::Quaternion,
        _ => return Err(W3dSceneError::UnsupportedAnimationChannel(raw_kind)),
    };
    let required_vector = if kind == W3dAnimationChannelKind::Quaternion {
        4
    } else {
        1
    };
    if usize::from(vector_length) != required_vector {
        return Err(W3dSceneError::InvalidVectorLength {
            kind: raw_kind,
            actual: vector_length,
            expected: required_vector,
        });
    }
    let samples = usize::from(last_frame - first_frame) + 1;
    let value_count = size(
        samples,
        usize::from(vector_length),
        "animation channel values",
    )?;
    let expected = ANIMATION_CHANNEL_HEADER_BYTES
        .checked_add(size(value_count, 4, "animation channel bytes")?)
        .ok_or(W3dSceneError::SizeOverflow("animation channel"))?;
    if bytes.len() < expected {
        return Err(W3dSceneError::InvalidLength {
            id: ANIMATION_CHANNEL,
            actual: bytes.len(),
            expected,
        });
    }
    // Retail files can retain unused whole-float samples after the declared frame range.
    // `HRawAnimClass::Load_W3D` at the revision named above reads only the declared range
    // before closing the chunk, so compatibility requires accepting the same bounded padding.
    if !(bytes.len() - expected).is_multiple_of(4) {
        return Err(W3dSceneError::InvalidAnimationPadding {
            actual: bytes.len() - expected,
        });
    }
    let mut values = Vec::with_capacity(value_count);
    for index in 0..value_count {
        let value = f32::from_bits(reader.read_u32_le()?);
        if !value.is_finite() {
            return Err(W3dSceneError::NonFinite {
                what: "animation value",
                index,
            });
        }
        values.push(value);
    }
    Ok(W3dAnimationChannel {
        first_frame,
        last_frame,
        pivot,
        kind,
        vector_length,
        values,
    })
}

fn decode_hlod(
    chunk: &W3dChunk,
    pivot_count: usize,
    limits: W3dSceneLimits,
) -> Result<W3dHlod, W3dSceneError> {
    let children = container(chunk)?;
    let header = data(required(children, HLOD_HEADER)?)?;
    exact(HLOD_HEADER, header.len(), HLOD_HEADER_BYTES)?;
    let mut reader = BinaryReader::new(header, "W3D HLOD header");
    let _version = reader.read_u32_le()?;
    let lod_count = limited(reader.read_u32_le()?, "W3D HLOD count", limits.maximum_lods)?;
    let name = fixed_name(read_array::<16>(&mut reader)?);
    let hierarchy_name = fixed_name(read_array::<16>(&mut reader)?);
    let wrappers = children
        .iter()
        .filter(|child| child.id() == HLOD_ARRAY)
        .collect::<Vec<_>>();
    if wrappers.len() != lod_count {
        return Err(W3dSceneError::CountMismatch {
            what: "HLOD arrays",
            declared: lod_count,
            actual: wrappers.len(),
        });
    }
    let mut lods = Vec::with_capacity(lod_count);
    for wrapper in wrappers {
        lods.push(decode_lod(wrapper, pivot_count, limits)?);
    }
    Ok(W3dHlod {
        name,
        hierarchy_name,
        lods,
    })
}

fn decode_lod(
    chunk: &W3dChunk,
    pivot_count: usize,
    limits: W3dSceneLimits,
) -> Result<W3dLod, W3dSceneError> {
    let children = container(chunk)?;
    let header = data(required(children, HLOD_ARRAY_HEADER)?)?;
    exact(HLOD_ARRAY_HEADER, header.len(), HLOD_ARRAY_HEADER_BYTES)?;
    let mut reader = BinaryReader::new(header, "W3D HLOD array header");
    let count = limited(
        reader.read_u32_le()?,
        "W3D HLOD sub-object count",
        limits.maximum_sub_objects_per_lod,
    )?;
    let maximum_screen_size = f32::from_bits(reader.read_u32_le()?);
    if !maximum_screen_size.is_finite() {
        return Err(W3dSceneError::NonFinite {
            what: "HLOD screen size",
            index: 0,
        });
    }
    let objects = children
        .iter()
        .filter(|child| child.id() == HLOD_SUB_OBJECT)
        .collect::<Vec<_>>();
    if objects.len() != count {
        return Err(W3dSceneError::CountMismatch {
            what: "HLOD sub-objects",
            declared: count,
            actual: objects.len(),
        });
    }
    let mut sub_objects = Vec::with_capacity(count);
    for (index, object) in objects.into_iter().enumerate() {
        let bytes = data(object)?;
        exact(HLOD_SUB_OBJECT, bytes.len(), HLOD_SUB_OBJECT_BYTES)?;
        let mut reader = BinaryReader::new(bytes, "W3D HLOD sub-object");
        let bone_index = reader.read_u32_le()?;
        if usize::try_from(bone_index).map_or(true, |bone| bone >= pivot_count) {
            return Err(W3dSceneError::BoneIndexOutOfRange {
                vertex: index,
                index: bone_index,
                count: pivot_count,
            });
        }
        let name = fixed_name(read_array::<32>(&mut reader)?);
        sub_objects.push(W3dSubObject { bone_index, name });
    }
    Ok(W3dLod {
        maximum_screen_size,
        sub_objects,
    })
}

fn mesh_full_name(chunk: &W3dChunk) -> Option<Vec<u8>> {
    let header = chunk.children()?.first()?.data()?;
    if header.len() < 40 {
        return None;
    }
    let mesh = fixed_name::<16>(header.get(8..24)?.try_into().ok()?);
    let container = fixed_name::<16>(header.get(24..40)?.try_into().ok()?);
    let mut full = container;
    if !full.is_empty() && !mesh.is_empty() {
        full.push(b'.');
    }
    full.extend(mesh);
    Some(full)
}

fn render_object_name(chunk: &W3dChunk) -> Option<Vec<u8>> {
    let bytes = chunk.data()?.get(8..40)?;
    Some(
        bytes[..bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len())]
            .to_vec(),
    )
}

fn unique_top(chunks: &[W3dChunk], id: u32) -> Result<Option<&W3dChunk>, W3dSceneError> {
    let mut matching = chunks.iter().filter(|chunk| chunk.id() == id);
    let first = matching.next();
    if matching.next().is_some() {
        Err(W3dSceneError::DuplicateChunk(id))
    } else {
        Ok(first)
    }
}

fn required(children: &[W3dChunk], id: u32) -> Result<&W3dChunk, W3dSceneError> {
    let mut matching = children.iter().filter(|chunk| chunk.id() == id);
    let first = matching.next().ok_or(W3dSceneError::MissingChunk(id))?;
    if matching.next().is_some() {
        Err(W3dSceneError::DuplicateChunk(id))
    } else {
        Ok(first)
    }
}

fn container(chunk: &W3dChunk) -> Result<&[W3dChunk], W3dSceneError> {
    chunk
        .children()
        .ok_or(W3dSceneError::ExpectedContainer(chunk.id()))
}
fn data(chunk: &W3dChunk) -> Result<&[u8], W3dSceneError> {
    chunk.data().ok_or(W3dSceneError::ExpectedData(chunk.id()))
}
fn exact(id: u32, actual: usize, expected: usize) -> Result<(), W3dSceneError> {
    if actual == expected {
        Ok(())
    } else {
        Err(W3dSceneError::InvalidLength {
            id,
            actual,
            expected,
        })
    }
}
fn size(count: usize, width: usize, what: &'static str) -> Result<usize, W3dSceneError> {
    count
        .checked_mul(width)
        .ok_or(W3dSceneError::SizeOverflow(what))
}
fn limited(value: u32, what: &'static str, maximum: usize) -> Result<usize, W3dSceneError> {
    let value = usize::try_from(value).unwrap_or(usize::MAX);
    if value > maximum {
        Err(limit(what, value, maximum))
    } else {
        Ok(value)
    }
}
fn limit(what: &'static str, actual: usize, maximum: usize) -> W3dSceneError {
    W3dSceneError::Binary(BinaryError::LimitExceeded {
        what,
        actual,
        maximum,
    })
}
fn fixed_name<const N: usize>(bytes: [u8; N]) -> Vec<u8> {
    bytes[..bytes.iter().position(|byte| *byte == 0).unwrap_or(N)].to_vec()
}
fn read_array<const N: usize>(reader: &mut BinaryReader<'_>) -> Result<[u8; N], BinaryError> {
    reader
        .read_exact(N)?
        .try_into()
        .map_err(|_| BinaryError::UnexpectedEof {
            source: "W3D fixed array".to_owned(),
            offset: 0,
            requested: N,
            remaining: 0,
        })
}
fn read_vec3(
    reader: &mut BinaryReader<'_>,
    what: &'static str,
    index: usize,
) -> Result<[f32; 3], W3dSceneError> {
    read_floats(reader, what, index)
}
fn read_floats<const N: usize>(
    reader: &mut BinaryReader<'_>,
    what: &'static str,
    index: usize,
) -> Result<[f32; N], W3dSceneError> {
    let mut values = [0.0; N];
    for value in &mut values {
        *value = f32::from_bits(reader.read_u32_le()?);
        if !value.is_finite() {
            return Err(W3dSceneError::NonFinite { what, index });
        }
    }
    Ok(values)
}
fn ascii_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum W3dSceneError {
    Binary(BinaryError),
    Mesh(W3dMeshError),
    MissingChunk(u32),
    DuplicateChunk(u32),
    ExpectedContainer(u32),
    ExpectedData(u32),
    InvalidLength {
        id: u32,
        actual: usize,
        expected: usize,
    },
    CountMismatch {
        what: &'static str,
        declared: usize,
        actual: usize,
    },
    InvalidParent {
        pivot: usize,
        parent: u32,
    },
    InvalidFrameRange {
        first: u16,
        last: u16,
        count: u32,
    },
    InvalidVectorLength {
        kind: u16,
        actual: u16,
        expected: usize,
    },
    UnsupportedAnimationChannel(u16),
    InvalidAnimationPadding {
        actual: usize,
    },
    BoneIndexOutOfRange {
        vertex: usize,
        index: u32,
        count: usize,
    },
    NonFinite {
        what: &'static str,
        index: usize,
    },
    EmptyHierarchy,
    ZeroFrameCount,
    ZeroFrameRate,
    SizeOverflow(&'static str),
    HierarchyNameMismatch,
    EmptyHlod,
    MissingMesh(Vec<u8>),
}

impl Display for W3dSceneError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => Display::fmt(error, f),
            Self::Mesh(error) => Display::fmt(error, f),
            Self::MissingChunk(id) => write!(f, "W3D model is missing chunk 0x{id:08X}"),
            Self::DuplicateChunk(id) => write!(f, "W3D model repeats chunk 0x{id:08X}"),
            Self::ExpectedContainer(id) => write!(f, "W3D chunk 0x{id:08X} must be a container"),
            Self::ExpectedData(id) => write!(f, "W3D chunk 0x{id:08X} must be data"),
            Self::InvalidLength {
                id,
                actual,
                expected,
            } => write!(
                f,
                "W3D chunk 0x{id:08X} has {actual} bytes; expected {expected}"
            ),
            Self::CountMismatch {
                what,
                declared,
                actual,
            } => write!(
                f,
                "W3D declares {declared} {what}, but {actual} were decoded"
            ),
            Self::InvalidParent { pivot, parent } => {
                write!(f, "W3D pivot {pivot} has invalid parent {parent}")
            }
            Self::InvalidFrameRange { first, last, count } => write!(
                f,
                "W3D animation frame range {first}..={last} exceeds {count} frames"
            ),
            Self::InvalidVectorLength {
                kind,
                actual,
                expected,
            } => write!(
                f,
                "W3D animation channel {kind} vector length is {actual}; expected {expected}"
            ),
            Self::UnsupportedAnimationChannel(kind) => {
                write!(f, "unsupported classic W3D animation channel {kind}")
            }
            Self::InvalidAnimationPadding { actual } => write!(
                f,
                "W3D animation channel has {actual} trailing bytes; expected whole floats"
            ),
            Self::BoneIndexOutOfRange {
                vertex,
                index,
                count,
            } => write!(
                f,
                "W3D element {vertex} references bone {index}, but only {count} exist"
            ),
            Self::NonFinite { what, index } => write!(f, "W3D {what} {index} is non-finite"),
            Self::EmptyHierarchy => f.write_str("W3D hierarchy contains no pivots"),
            Self::ZeroFrameCount => f.write_str("W3D animation frame count is zero"),
            Self::ZeroFrameRate => f.write_str("W3D animation frame rate is zero"),
            Self::SizeOverflow(what) => write!(f, "W3D {what} size overflowed"),
            Self::HierarchyNameMismatch => f.write_str("W3D HLOD and hierarchy names differ"),
            Self::EmptyHlod => f.write_str("W3D HLOD contains no detail levels"),
            Self::MissingMesh(name) => write!(
                f,
                "W3D HLOD references missing mesh {}",
                String::from_utf8_lossy(name)
            ),
        }
    }
}

impl Error for W3dSceneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            Self::Mesh(error) => Some(error),
            _ => None,
        }
    }
}
impl From<BinaryError> for W3dSceneError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}
impl From<W3dMeshError> for W3dSceneError {
    fn from(error: W3dMeshError) -> Self {
        Self::Mesh(error)
    }
}

#[cfg(test)]
mod tests {
    use crate::{W3dLimits, parse_w3d};

    use super::{
        W3dMeshLimits, W3dSceneError, W3dSceneLimits, decode_animation_channel, decode_w3d_model,
    };

    fn chunk(id: u32, container: bool, payload: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(&id.to_le_bytes());
        let length = u32::try_from(payload.len()).expect("test chunk length fits u32")
            | if container { 0x8000_0000 } else { 0 };
        result.extend_from_slice(&length.to_le_bytes());
        result.extend_from_slice(payload);
        result
    }

    #[test]
    fn rejects_empty_hierarchy_before_model_allocation() {
        let mut header = vec![0; 36];
        header[..4].copy_from_slice(&0x0004_0001_u32.to_le_bytes());
        let mut children = chunk(0x101, false, &header);
        children.extend_from_slice(&chunk(0x102, false, &[]));
        let bytes = chunk(0x100, true, &children);
        let file = parse_w3d(&bytes, "empty-hierarchy.w3d", W3dLimits::default())
            .expect("valid chunk stream");

        assert_eq!(
            decode_w3d_model(&file, W3dMeshLimits::default(), W3dSceneLimits::default()),
            Err(W3dSceneError::EmptyHierarchy)
        );
    }

    #[test]
    fn animation_padding_must_be_whole_floats() {
        let mut payload = Vec::new();
        for value in [0_u16, 0, 1, 0, 1, 0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&0.0_f32.to_le_bytes());
        payload.extend_from_slice(&[0, 0]);
        let bytes = chunk(0x202, false, &payload);
        let file =
            parse_w3d(&bytes, "bad-padding.w3d", W3dLimits::default()).expect("valid chunk stream");

        assert_eq!(
            decode_animation_channel(&file.chunks()[0], 2, 1),
            Err(W3dSceneError::InvalidAnimationPadding { actual: 2 })
        );
    }
}

//! Converting a `.glb`'s own textures: derive each image's slot, merge ORM, write sidecars, slim the glb.
//!
//! # Why the slot does not have to be guessed
//!
//! A glTF material states exactly which slot every image is read through — `baseColorTexture`,
//! `normalTexture`, `metallicRoughnessTexture`, `occlusionTexture`, `emissiveTexture`. So the slot, and
//! therefore the block format and the colour space, are *derived* rather than inferred from a filename. No
//! `_normal` suffix convention, no heuristics, and nothing to get wrong on an asset that names its files
//! unhelpfully.
//!
//! An image referenced through slots that disagree — a base colour in one material and a normal map in
//! another — has no single answer, and is reported rather than resolved by picking one.
//!
//! # The ORM merge, and why it forces a rewrite
//!
//! glTF puts occlusion in one texture's red and metallic-roughness in another's green and blue, and permits
//! those to be two different images. This engine reads all three from *one* image, because that is what
//! content overwhelmingly authors and because a fourth array slot and a fourth slice index are not free —
//! see `baked_occlusion_strength` in `cic-render`.
//!
//! So a model whose occlusion is separate has its two images merged into one: red from the occlusion map,
//! green and blue from the metallic-roughness map. And *then* both material slots have to point at the
//! merged image, which means the glTF itself changes. That is why this converts and rewrites together
//! rather than only emitting textures.
//!
//! The absent channels matter as much as the present ones. With no occlusion map, red is **255** — glTF
//! leaves red unused in a metallic-roughness image, so whatever it happens to carry would otherwise be read
//! as occlusion and darken the surface for no reason. With no metallic-roughness map, green and blue are
//! 255, which is the identity for the factors they multiply.
//!
//! # Why the images are replaced rather than removed
//!
//! The container keeps declaring its images, as 1x1 placeholders. It cannot carry the DDS itself: the
//! runtime does not follow a glTF `uri`, because that would read the host filesystem from an untrusted
//! asset, and the sidecar convention exists for exactly that reason. And the images cannot simply be
//! deleted, because a material's slot references are how the runtime knows which sidecar belongs to which
//! slot — an image entry with a name is the link. A placeholder keeps the link and costs nothing.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use cic_assets::image::{ColourSpace, decode_in, encode_in, mip_chain, resample};
use cic_assets::texture::{BlockFormat, TextureAsset, TextureLimits};
use cic_assets::{Model, ModelLimits, import_model};
use serde_json::{Map, Value};

use crate::encode;

/// Where the sidecars go by default, matching the directory the runtime looks in.
pub const TEXTURE_DIRECTORY: &str = cic_assets::TEXTURE_DIRECTORY;

/// What a glTF image is read as, and therefore how it converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// Base colour or albedo.
    BaseColour,
    /// Tangent-space normal map.
    Normal,
    /// Packed occlusion, roughness and metallic.
    Orm,
    /// Emissive colour.
    Emissive,
}

impl Slot {
    const fn format(self) -> BlockFormat {
        match self {
            // Emissive is a colour a human picked, so it takes the base colour's treatment.
            Self::BaseColour | Self::Emissive => BlockFormat::Bc7UnormSrgb,
            Self::Normal => BlockFormat::Bc5Unorm,
            Self::Orm => BlockFormat::Bc7Unorm,
        }
    }

    /// The suffix a synthesised name gets, for an image the container left unnamed.
    const fn suffix(self) -> &'static str {
        match self {
            Self::BaseColour => "basecolor",
            Self::Normal => "normal",
            Self::Orm => "orm",
            Self::Emissive => "emissive",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::BaseColour => "base colour",
            Self::Normal => "normal",
            Self::Orm => "occlusion/roughness/metallic",
            Self::Emissive => "emissive",
        }
    }
}

/// One texture the repack will write, and where its pixels come from.
#[derive(Debug)]
struct Conversion {
    /// Sidecar name, which is also the glTF image name the runtime looks it up by.
    name: String,
    slot: Slot,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    /// The image index this reuses, or `None` when its pixels are new and an image entry must be added.
    reuses: Option<usize>,
}

/// What a repack produced.
pub struct Repacked {
    /// The rewritten container.
    pub glb: Vec<u8>,
    /// Sidecar file stem and its DDS bytes, in the order they were planned.
    pub textures: Vec<(String, Vec<u8>)>,
    /// A human-readable account of what happened, for the tool to print.
    pub report: String,
}

/// Converts a `.glb`'s textures to sidecars and rewrites it to reference them.
///
/// `stem` names the model, and is what synthesised texture names are built from.
///
/// # Errors
///
/// Returns a message when the container will not parse or import, an image is referenced through
/// disagreeing slots, an ORM merge would join two images sampled at different texture coordinates, or the
/// rewrite cannot preserve something it does not understand.
pub fn repack(bytes: &[u8], stem: &str) -> Result<Repacked, String> {
    let model = import_model(bytes, ModelLimits::default()).map_err(|error| error.to_string())?;
    let (mut json, bin) = split_glb(bytes)?;

    let plans = plan(&json, &model, stem)?;
    let mut report = String::new();
    let mut textures = Vec::new();
    for conversion in &plans.conversions {
        let format = conversion.slot.format();
        let levels: Vec<Vec<u8>> = mip_chain(
            &conversion.rgba,
            conversion.width,
            conversion.height,
            format.colour_space(),
        )
        .into_iter()
        .map(|(width, height, pixels)| encode::encode_level(&pixels, width, height, format))
        .collect();
        let texture = TextureAsset::new(
            conversion.width,
            conversion.height,
            format,
            levels,
            TextureLimits::default(),
        )
        .map_err(|error| format!("{}: {error}", conversion.name))?;
        let encoded = texture.encode();
        let _ = writeln!(
            report,
            "  {:28} {:>4}x{:<4} {:>9.1} KiB  {}{}",
            format!("{}.dds", conversion.name),
            conversion.width,
            conversion.height,
            kib(encoded.len()),
            format.name(),
            if conversion.reuses.is_some() {
                String::new()
            } else {
                format!(" (merged for {})", conversion.slot.label())
            }
        );
        textures.push((conversion.name.clone(), encoded));
    }

    rewrite(&mut json, &plans)?;
    let glb = assemble_glb(&json, &bin, &plans)?;
    // An image whose only reader was a slot that has been merged away is now unreferenced. It stays in the
    // container as a placeholder rather than being removed, because removing one renumbers every later
    // image and every texture's `source` with it -- a rewrite with more ways to go wrong than the hundred
    // bytes are worth. Reported so an author can prune the source instead.
    let reused: std::collections::BTreeSet<usize> = plans
        .conversions
        .iter()
        .filter_map(|conversion| conversion.reuses)
        .collect();
    let orphans = plans.image_count.saturating_sub(reused.len());
    if orphans > 0 {
        let _ = writeln!(
            report,
            "  {orphans} image{} no longer referenced by any material, left as a placeholder",
            if orphans == 1 { " is" } else { "s are" }
        );
    }
    let _ = writeln!(
        report,
        "  glb {:.1} KiB -> {:.1} KiB, {} image{} replaced by a 1x1 placeholder",
        kib(bytes.len()),
        kib(glb.len()),
        plans.image_count,
        if plans.image_count == 1 { "" } else { "s" }
    );
    Ok(Repacked {
        glb,
        textures,
        report,
    })
}

/// Everything the rewrite needs to know, worked out before anything is encoded.
#[derive(Debug, Default)]
struct Plans {
    conversions: Vec<Conversion>,
    /// Per material index: the ORM conversion it should point at.
    material_orm: BTreeMap<usize, usize>,
    /// Names to set on existing images, by image index.
    image_names: BTreeMap<usize, String>,
    /// How many image entries the container declares.
    image_count: usize,
}

/// Works out which image fills which slot, and what has to be merged.
#[allow(clippy::too_many_lines)]
fn plan(json: &Value, model: &Model, stem: &str) -> Result<Plans, String> {
    let images = array(json, "images");
    let textures = array(json, "textures");
    let materials = array(json, "materials");
    let mut plans = Plans {
        image_count: images.len(),
        ..Plans::default()
    };

    // A texture index resolves to an image index through `textures[t].source`.
    let source = |texture: usize| -> Option<usize> {
        textures
            .get(texture)?
            .get("source")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
    };
    let slot_of = |material: &Value, path: &[&str]| -> Option<(usize, u64)> {
        let mut node = material;
        for key in path {
            node = node.get(*key)?;
        }
        let index = usize::try_from(node.get("index")?.as_u64()?).ok()?;
        let coordinates = node.get("texCoord").and_then(Value::as_u64).unwrap_or(0);
        Some((index, coordinates))
    };

    // Every non-ORM slot, by image, so a disagreement is caught before anything converts.
    let mut simple: BTreeMap<usize, Slot> = BTreeMap::new();
    let note = |image: usize,
                slot: Slot,
                plans: &mut BTreeMap<usize, Slot>|
     -> Result<(), String> {
        match plans.get(&image) {
            Some(existing) if *existing != slot => Err(format!(
                "image {image} is read as a {} by one material and as a {} by another, so it has no \
                 single format; split it into two images before converting",
                existing.label(),
                slot.label()
            )),
            _ => {
                plans.insert(image, slot);
                Ok(())
            }
        }
    };

    // The ORM pairs, keyed so materials sharing a pair share one output texture.
    let mut orm_pairs: BTreeMap<(Option<usize>, Option<usize>), usize> = BTreeMap::new();

    for (index, material) in materials.iter().enumerate() {
        for (path, slot) in [
            (
                &["pbrMetallicRoughness", "baseColorTexture"][..],
                Slot::BaseColour,
            ),
            (&["normalTexture"][..], Slot::Normal),
            (&["emissiveTexture"][..], Slot::Emissive),
        ] {
            if let Some((texture, _)) = slot_of(material, path)
                && let Some(image) = source(texture)
            {
                note(image, slot, &mut simple)?;
            }
        }

        let occlusion = slot_of(material, &["occlusionTexture"]);
        let metallic_roughness = slot_of(
            material,
            &["pbrMetallicRoughness", "metallicRoughnessTexture"],
        );
        if occlusion.is_none() && metallic_roughness.is_none() {
            continue;
        }
        // Merging two images sampled at different coordinate sets would put one map's texels under the
        // other's UVs. There is no correct merge, so this is refused rather than approximated.
        if let (Some((_, occlusion_set)), Some((_, mr_set))) = (occlusion, metallic_roughness)
            && occlusion_set != mr_set
        {
            return Err(format!(
                "material {index} samples its occlusion map at texture coordinate set {occlusion_set} and \
                 its metallic-roughness map at set {mr_set}; they cannot be merged into one image"
            ));
        }
        let pair = (
            occlusion.and_then(|(texture, _)| source(texture)),
            metallic_roughness.and_then(|(texture, _)| source(texture)),
        );
        let next = plans.conversions.len();
        let conversion = *orm_pairs.entry(pair).or_insert(next);
        if conversion == next {
            plans
                .conversions
                .push(orm_conversion(pair, model, stem, next)?);
        }
        plans.material_orm.insert(index, conversion);
    }

    // Now the simple slots, after the ORM ones so a merged image's name cannot collide with a real one.
    for (image, slot) in simple {
        let source_image = model
            .images
            .get(image)
            .ok_or_else(|| format!("material references image {image}, which was not decoded"))?;
        let name = if source_image.name.is_empty() {
            let synthesised = format!("{stem}_{}", slot.suffix());
            plans.image_names.insert(image, synthesised.clone());
            synthesised
        } else {
            source_image.name.clone()
        };
        plans.conversions.push(Conversion {
            name,
            slot,
            width: source_image.width,
            height: source_image.height,
            rgba: source_image.rgba.clone(),
            reuses: Some(image),
        });
    }

    Ok(plans)
}

/// Builds the ORM conversion for one occlusion/metallic-roughness pair.
///
/// Reuses the source image untouched when both channels already come from it, which is the common authored
/// case and keeps the container's own image where it was.
fn orm_conversion(
    pair: (Option<usize>, Option<usize>),
    model: &Model,
    stem: &str,
    ordinal: usize,
) -> Result<Conversion, String> {
    let fetch = |index: Option<usize>| match index {
        None => Ok(None),
        Some(index) => model
            .images
            .get(index)
            .map(Some)
            .ok_or_else(|| format!("material references image {index}, which was not decoded")),
    };
    let occlusion = fetch(pair.0)?;
    let metallic_roughness = fetch(pair.1)?;

    // Already one image carrying all three: nothing to merge, and its own name is the right one.
    if let (Some(first), Some(second)) = (pair.0, pair.1)
        && first == second
    {
        let image = occlusion.ok_or("unreachable: fetched above")?;
        let name = if image.name.is_empty() {
            format!("{stem}_{}", Slot::Orm.suffix())
        } else {
            image.name.clone()
        };
        return Ok(Conversion {
            name,
            slot: Slot::Orm,
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
            reuses: Some(first),
        });
    }

    // One output at the larger of the two sizes, so neither map is downsampled.
    let width = occlusion
        .map_or(0, |image| image.width)
        .max(metallic_roughness.map_or(0, |image| image.height.min(image.width)))
        .max(metallic_roughness.map_or(0, |image| image.width));
    let height = occlusion
        .map_or(0, |image| image.height)
        .max(metallic_roughness.map_or(0, |image| image.height));
    if width == 0 || height == 0 {
        return Err("an ORM merge needs at least one source image".to_owned());
    }

    // 255 in a channel with no source: glTF leaves red unused in a metallic-roughness image, so carrying
    // its contents into the occlusion channel would darken the surface by whatever happened to be there,
    // and green and blue multiply factors they must leave alone.
    let mut rgba = vec![u8::MAX; (width as usize) * (height as usize) * 4];
    if let Some(image) = occlusion {
        let resampled = to_size(image.width, image.height, &image.rgba, width, height);
        for (texel, source) in rgba.chunks_exact_mut(4).zip(resampled.chunks_exact(4)) {
            texel[0] = source[0];
        }
    }
    if let Some(image) = metallic_roughness {
        let resampled = to_size(image.width, image.height, &image.rgba, width, height);
        for (texel, source) in rgba.chunks_exact_mut(4).zip(resampled.chunks_exact(4)) {
            texel[1] = source[1];
            texel[2] = source[2];
        }
    }

    Ok(Conversion {
        name: format!("{stem}_{}{}", Slot::Orm.suffix(), suffix_ordinal(ordinal)),
        slot: Slot::Orm,
        width,
        height,
        rgba,
        reuses: None,
    })
}

/// A distinguishing suffix for the second and later merged textures of one model.
fn suffix_ordinal(ordinal: usize) -> String {
    if ordinal == 0 {
        String::new()
    } else {
        format!("_{ordinal}")
    }
}

/// Resamples RGBA to a size, in linear light because these are measurements rather than colour.
fn to_size(width: u32, height: u32, rgba: &[u8], to_width: u32, to_height: u32) -> Vec<u8> {
    if width == to_width && height == to_height {
        return rgba.to_vec();
    }
    encode_in(
        &resample(
            &decode_in(rgba, ColourSpace::Linear),
            width,
            height,
            to_width,
            to_height,
        ),
        ColourSpace::Linear,
    )
}

/// A byte count in kibibytes, for the report.
///
/// The cast is from a length this process is holding in memory, so it is nowhere near the point where an
/// `f64` mantissa stops representing an integer exactly.
#[allow(clippy::cast_precision_loss)]
fn kib(bytes: usize) -> f64 {
    bytes as f64 / 1024.0
}

/// Reads an array member of the document, or an empty slice when absent.
fn array<'a>(json: &'a Value, key: &str) -> &'a [Value] {
    json.get(key).and_then(Value::as_array).map_or(&[], |v| v)
}

// ------------------------------------------------------------------ the container

// Spelled from the bytes rather than written as hex, the way `cic_assets::terrain` spells its own magic.
// The hex is not reviewable -- the first version of this file had `glTF` as `0x4674_6C67`, one nibble out,
// and it was the `gltf` crate refusing the *fixture* that found it rather than anything reading the hex.
/// GLB magic, `"glTF"`.
const GLB_MAGIC: u32 = u32::from_le_bytes(*b"glTF");
/// The JSON chunk's type.
const CHUNK_JSON: u32 = u32::from_le_bytes(*b"JSON");
/// The binary chunk's type.
const CHUNK_BIN: u32 = u32::from_le_bytes(*b"BIN\0");

/// Splits a GLB into its parsed JSON chunk and its binary chunk.
///
/// A GLB is a twelve-byte header and then length-tagged chunks, each padded to four bytes. The JSON is
/// parsed into an untyped tree rather than into typed structures on purpose: everything this does not touch
/// has to survive the round trip byte for byte, and an untyped tree cannot silently drop an extension it
/// does not know about.
fn split_glb(bytes: &[u8]) -> Result<(Value, Vec<u8>), String> {
    let word = |offset: usize| -> Result<u32, String> {
        bytes
            .get(offset..offset + 4)
            .and_then(|slice| slice.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| format!("the container ends inside its header at byte {offset}"))
    };
    if word(0)? != GLB_MAGIC {
        return Err("not a binary glTF: the magic is not `glTF`".to_owned());
    }
    let version = word(4)?;
    if version != 2 {
        return Err(format!(
            "glTF binary version {version} is not 2, and a version bump can change what fields mean"
        ));
    }

    let mut json = None;
    let mut bin = Vec::new();
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let length = word(offset)? as usize;
        let kind = word(offset + 4)?;
        let start = offset + 8;
        let payload = bytes.get(start..start + length).ok_or_else(|| {
            format!("a chunk at byte {offset} declares {length} bytes it does not have")
        })?;
        match kind {
            CHUNK_JSON => {
                json = Some(
                    serde_json::from_slice::<Value>(payload)
                        .map_err(|error| format!("the JSON chunk will not parse: {error}"))?,
                );
            }
            CHUNK_BIN => bin = payload.to_vec(),
            // An unknown chunk is skipped by the specification, and dropping one on rewrite would be a
            // silent loss -- so this refuses rather than pretending it round-tripped.
            other => {
                return Err(format!(
                    "the container holds a chunk of type {other:#010x} this rewriter would drop"
                ));
            }
        }
        // Chunks are four-byte aligned; the padding is part of the container.
        offset = start + length + ((4 - length % 4) % 4);
    }
    json.ok_or_else(|| "the container has no JSON chunk".to_owned())
        .map(|json| (json, bin))
}

/// A 1x1 opaque-white PNG, the placeholder every image entry is pointed at.
fn placeholder_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .expect("a 1x1 header is always valid");
        writer
            .write_image_data(&[u8::MAX; 4])
            .expect("four bytes for one texel");
        writer.finish().expect("finish");
    }
    out
}

/// Applies the plan to the document: names, merged images, repointed slots, placeholder images.
fn rewrite(json: &mut Value, plans: &Plans) -> Result<(), String> {
    let object = json
        .as_object_mut()
        .ok_or("the JSON chunk is not an object")?;

    // Names first, so every sidecar has something to be found by. An unnamed image cannot be resolved at
    // all -- see `resolve_named_textures` -- so this is what makes an unnamed source usable rather than a
    // cosmetic touch.
    if let Some(images) = object.get_mut("images").and_then(Value::as_array_mut) {
        for (index, name) in &plans.image_names {
            if let Some(image) = images.get_mut(*index).and_then(Value::as_object_mut) {
                image.insert("name".to_owned(), Value::String(name.clone()));
            }
        }
        for conversion in &plans.conversions {
            if let Some(index) = conversion.reuses
                && let Some(image) = images.get_mut(index).and_then(Value::as_object_mut)
            {
                image.insert("name".to_owned(), Value::String(conversion.name.clone()));
            }
        }
    }

    // Merged ORM textures need an image entry and a texture entry of their own. A *new* texture rather than
    // repointing the existing ones, because a texture may be shared between materials whose merges differ,
    // and repointing would then serve one material's pixels to another.
    let mut added_images: BTreeMap<usize, usize> = BTreeMap::new();
    let mut added_textures: BTreeMap<usize, usize> = BTreeMap::new();
    for (ordinal, conversion) in plans.conversions.iter().enumerate() {
        if conversion.reuses.is_some() {
            continue;
        }
        let images = object
            .entry("images")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("`images` is not an array")?;
        let mut entry = Map::new();
        entry.insert("name".to_owned(), Value::String(conversion.name.clone()));
        entry.insert("mimeType".to_owned(), Value::String("image/png".to_owned()));
        images.push(Value::Object(entry));
        added_images.insert(ordinal, images.len() - 1);

        let image_index = images.len() - 1;
        let textures = object
            .entry("textures")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("`textures` is not an array")?;
        let mut texture = Map::new();
        texture.insert(
            "source".to_owned(),
            Value::Number(serde_json::Number::from(image_index)),
        );
        textures.push(Value::Object(texture));
        added_textures.insert(ordinal, textures.len() - 1);
    }

    // Point both halves of each material's ORM at its merged texture.
    if let Some(materials) = object.get_mut("materials").and_then(Value::as_array_mut) {
        for (material_index, conversion) in &plans.material_orm {
            let Some(texture) = added_textures.get(conversion) else {
                continue; // Reused its source image, so the existing references already resolve.
            };
            let Some(material) = materials
                .get_mut(*material_index)
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            // Only `index` changes. Everything else on a texture reference belongs to the *slot* rather
            // than to the image -- `strength` scales the occlusion term, `texCoord` picks the UV set, and
            // an extension like `KHR_texture_transform` carries a tiling -- so rebuilding the reference
            // from scratch silently resets all of it to the specification's defaults. Which it did: the
            // first version of this reset `strength` from 0.8 to 1, and the test above is why that is not
            // still true.
            let repoint = |reference: &mut Value| {
                let entry = match reference {
                    Value::Object(entry) => entry,
                    other => {
                        *other = Value::Object(Map::new());
                        other.as_object_mut().expect("just assigned an object")
                    }
                };
                entry.insert(
                    "index".to_owned(),
                    Value::Number(serde_json::Number::from(*texture)),
                );
            };
            repoint(
                material
                    .entry("occlusionTexture")
                    .or_insert_with(|| Value::Object(Map::new())),
            );
            let pbr = material
                .entry("pbrMetallicRoughness")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(pbr) = pbr.as_object_mut() {
                repoint(
                    pbr.entry("metallicRoughnessTexture")
                        .or_insert_with(|| Value::Object(Map::new())),
                );
            }
        }
    }

    Ok(())
}

/// Rebuilds the container: a compacted binary chunk and the rewritten JSON.
///
/// The binary chunk is compacted rather than appended to, which is the whole size win: an image's buffer
/// view is dropped and every kept view's offset is rewritten. Accessors reference views by *index* and the
/// indices do not move, so nothing else has to change — and each kept view is placed at a four-byte
/// boundary, which is the strictest alignment any accessor component needs.
#[allow(clippy::too_many_lines)]
fn assemble_glb(json: &Value, bin: &[u8], plans: &Plans) -> Result<Vec<u8>, String> {
    let mut json = json.clone();
    let object = json
        .as_object_mut()
        .ok_or("the JSON chunk is not an object")?;

    let image_views: Vec<usize> = array(&Value::Object(object.clone()), "images")
        .iter()
        .filter_map(|image| image.get("bufferView")?.as_u64())
        .filter_map(|index| usize::try_from(index).ok())
        .collect();

    let views = object
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut compacted = Vec::with_capacity(bin.len());
    let mut moved = Vec::with_capacity(views.len());
    for (index, view) in views.iter().enumerate() {
        let offset = view
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|_| "a buffer view offset does not fit this machine's address space")?;
        let length: usize = view
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("a buffer view has no byteLength")?
            .try_into()
            .map_err(|_| "a buffer view length does not fit this machine's address space")?;
        if image_views.contains(&index) {
            // Dropped: its image is a placeholder now.
            moved.push(None);
            continue;
        }
        let payload = bin
            .get(offset..offset + length)
            .ok_or_else(|| format!("buffer view {index} reaches past the binary chunk"))?;
        while !compacted.len().is_multiple_of(4) {
            compacted.push(0);
        }
        moved.push(Some(compacted.len()));
        compacted.extend_from_slice(payload);
    }

    // The placeholder, shared by every image entry.
    let placeholder = placeholder_png();
    while !compacted.len().is_multiple_of(4) {
        compacted.push(0);
    }
    let placeholder_offset = compacted.len();
    compacted.extend_from_slice(&placeholder);

    let mut rebuilt: Vec<Value> = Vec::with_capacity(views.len() + 1);
    for (view, offset) in views.iter().zip(&moved) {
        let mut entry = view
            .as_object()
            .ok_or("a buffer view is not an object")?
            .clone();
        // A dropped view keeps its slot so every accessor index stays valid, and is pointed at the
        // placeholder: a zero-length view is invalid glTF, and renumbering would mean rewriting every
        // accessor.
        let (offset, length) = match offset {
            Some(offset) => (
                *offset,
                entry
                    .get("byteLength")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .try_into()
                    .unwrap_or(0usize),
            ),
            None => (placeholder_offset, placeholder.len()),
        };
        entry.insert(
            "byteOffset".to_owned(),
            Value::Number(serde_json::Number::from(offset)),
        );
        entry.insert(
            "byteLength".to_owned(),
            Value::Number(serde_json::Number::from(length)),
        );
        rebuilt.push(Value::Object(entry));
    }
    // One more view for the placeholder, which every image now points at.
    let mut placeholder_view = Map::new();
    placeholder_view.insert(
        "buffer".to_owned(),
        Value::Number(serde_json::Number::from(0)),
    );
    placeholder_view.insert(
        "byteOffset".to_owned(),
        Value::Number(serde_json::Number::from(placeholder_offset)),
    );
    placeholder_view.insert(
        "byteLength".to_owned(),
        Value::Number(serde_json::Number::from(placeholder.len())),
    );
    rebuilt.push(Value::Object(placeholder_view));
    let placeholder_view_index = rebuilt.len() - 1;
    object.insert("bufferViews".to_owned(), Value::Array(rebuilt));

    // Every image becomes the placeholder.
    if let Some(images) = object.get_mut("images").and_then(Value::as_array_mut) {
        for image in images.iter_mut() {
            let entry = image.as_object_mut().ok_or("an image is not an object")?;
            entry.remove("uri");
            entry.insert(
                "bufferView".to_owned(),
                Value::Number(serde_json::Number::from(placeholder_view_index)),
            );
            entry.insert("mimeType".to_owned(), Value::String("image/png".to_owned()));
        }
    }

    // One buffer, whose length is the compacted chunk's.
    let mut buffer = Map::new();
    buffer.insert(
        "byteLength".to_owned(),
        Value::Number(serde_json::Number::from(compacted.len())),
    );
    object.insert(
        "buffers".to_owned(),
        Value::Array(vec![Value::Object(buffer)]),
    );
    let _ = plans;

    // And out as a container: header, JSON chunk, binary chunk, each padded to four bytes.
    let mut text = serde_json::to_vec(&json).map_err(|error| error.to_string())?;
    while !text.len().is_multiple_of(4) {
        text.push(b' '); // JSON pads with spaces, the binary chunk with zeroes.
    }
    while !compacted.len().is_multiple_of(4) {
        compacted.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + compacted.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(total)
            .map_err(|_| "the rewritten container does not fit a 32-bit length")?
            .to_le_bytes(),
    );
    for (payload, kind) in [(&text, CHUNK_JSON), (&compacted, CHUNK_BIN)] {
        out.extend_from_slice(
            &u32::try_from(payload.len())
                .map_err(|_| "a chunk does not fit a 32-bit length")?
                .to_le_bytes(),
        );
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(payload);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    // The vertex positions compared below are the fixture's own literals travelling through a container
    // unchanged, so exact comparison is the assertion -- an epsilon would hide precisely the buffer-offset
    // mistake these tests exist to catch.
    #![allow(clippy::float_cmp)]
    use super::{Slot, repack, split_glb};
    use cic_assets::texture::{BlockFormat, TextureLimits, decode_dds};
    use cic_assets::{ModelLimits, import_model};
    use serde_json::{Value, json};

    /// A solid RGBA PNG, which is what a glTF container actually carries.
    fn png(colour: [u8; 4], size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, size, size);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer
                .write_image_data(&colour.repeat((size * size) as usize))
                .expect("pixels");
            writer.finish().expect("finish");
        }
        out
    }

    /// Assembles a binary glTF: header, JSON chunk, binary chunk, each padded to four bytes.
    fn glb(document: &Value, binary: &[u8]) -> Vec<u8> {
        let mut text = serde_json::to_vec(document).expect("serialize");
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let mut bin = binary.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + text.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&u32::from_le_bytes(*b"glTF").to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&u32::try_from(total).expect("fits").to_le_bytes());
        for (payload, kind) in [
            (&text, u32::from_le_bytes(*b"JSON")),
            (&bin, u32::from_le_bytes(*b"BIN\0")),
        ] {
            out.extend_from_slice(&u32::try_from(payload.len()).expect("fits").to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(payload);
        }
        out
    }

    /// One triangle with a material whose occlusion is a *separate* image from its metallic-roughness --
    /// the arrangement this whole module exists to fix, and the one the renderer cannot read.
    ///
    /// `named` decides whether the images carry names, because an unnamed image is the case where the
    /// rewrite has to invent one: without a name there is no key a sidecar could be found under.
    fn separate_occlusion_glb(named: bool) -> Vec<u8> {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut binary: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        let base = png([200, 120, 90, 255], 4);
        let occlusion = png([64, 0, 0, 255], 4);
        let metallic_roughness = png([0, 180, 30, 255], 4);

        let mut views = vec![json!({"buffer": 0, "byteOffset": 0, "byteLength": binary.len()})];
        for image in [&base, &occlusion, &metallic_roughness] {
            while !binary.len().is_multiple_of(4) {
                binary.push(0);
            }
            views.push(json!({
                "buffer": 0, "byteOffset": binary.len(), "byteLength": image.len()
            }));
            binary.extend_from_slice(image);
        }

        let name = |text: &str| -> Value { if named { json!(text) } else { Value::Null } };
        let image_entry = |view: usize, label: &str| -> Value {
            let mut entry = json!({"bufferView": view, "mimeType": "image/png"});
            if named {
                entry["name"] = name(label);
            }
            entry
        };

        let document = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0}]}],
            "accessors": [{
                "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]
            }],
            "bufferViews": views,
            "buffers": [{"byteLength": binary.len()}],
            "images": [
                image_entry(1, "hull_basecolor"),
                image_entry(2, "hull_occlusion"),
                image_entry(3, "hull_mr"),
            ],
            "textures": [{"source": 0}, {"source": 1}, {"source": 2}],
            "materials": [{
                "name": "hull",
                "pbrMetallicRoughness": {
                    "baseColorTexture": {"index": 0},
                    "metallicRoughnessTexture": {"index": 2}
                },
                "occlusionTexture": {"index": 1, "strength": 0.8}
            }]
        });
        glb(&document, &binary)
    }

    #[test]
    fn a_separate_occlusion_map_is_merged_and_both_slots_repointed() {
        // The whole point. Before: occlusion in one image, metallic-roughness in another, which the
        // renderer reads as "no occlusion". After: one ORM image both slots name, so it reads all three.
        let repacked = repack(&separate_occlusion_glb(true), "hull").expect("repack");
        let model = import_model(&repacked.glb, ModelLimits::default()).expect("reimport");
        let material = &model.materials[0];
        assert_eq!(
            material.occlusion_texture, material.metallic_roughness_texture,
            "both slots must name one image, which is what makes the occlusion readable"
        );
        assert!(material.occlusion_texture.is_some());
        // The strength has to survive the repointing: it is what scales the term, and a rewrite that
        // rebuilt the reference from scratch would quietly reset it to glTF's default of one.
        assert!(
            (material.occlusion_strength - 0.8).abs() < 1.0e-6,
            "occlusion strength came back as {}",
            material.occlusion_strength
        );

        // And the merged image carries red from the occlusion map and green/blue from the other.
        let merged = repacked
            .textures
            .iter()
            .find(|(name, _)| name.contains("orm"))
            .expect("an ORM sidecar");
        let texture = decode_dds(&merged.1, TextureLimits::default()).expect("read the sidecar");
        assert_eq!(
            texture.format(),
            BlockFormat::Bc7Unorm,
            "ORM is linear data"
        );
        let texel = texture.decode();
        assert!(
            texel[0].abs_diff(64) <= 2,
            "red is the occlusion map's, got {}",
            texel[0]
        );
        assert!(
            texel[1].abs_diff(180) <= 2 && texel[2].abs_diff(30) <= 2,
            "green and blue are the metallic-roughness map's, got {:?}",
            &texel[..3]
        );
    }

    #[test]
    fn every_slot_is_derived_from_the_material_rather_than_from_a_name() {
        // No filename heuristics: the material says which slot each image is read through, so the format
        // follows from the reference. The base colour is sRGB and the ORM is linear, and getting that pair
        // wrong is the quiet mistake the whole slot design exists to prevent.
        let repacked = repack(&separate_occlusion_glb(true), "hull").expect("repack");
        let formats: Vec<(String, BlockFormat)> = repacked
            .textures
            .iter()
            .map(|(name, dds)| {
                (
                    name.clone(),
                    decode_dds(dds, TextureLimits::default())
                        .expect("read")
                        .format(),
                )
            })
            .collect();
        let base = formats
            .iter()
            .find(|(name, _)| name.contains("basecolor"))
            .expect("a base colour sidecar");
        assert_eq!(base.1, BlockFormat::Bc7UnormSrgb);
        assert_eq!(Slot::BaseColour.format(), BlockFormat::Bc7UnormSrgb);
        assert_eq!(Slot::Normal.format(), BlockFormat::Bc5Unorm);
        assert_eq!(Slot::Orm.format(), BlockFormat::Bc7Unorm);
    }

    #[test]
    fn an_unnamed_image_is_given_a_name_so_a_sidecar_can_be_found_for_it() {
        // An unnamed image cannot be resolved at all -- the name *is* the key. So the rewrite invents one
        // from the model's own file stem, which is also what keeps two models in one package from both
        // claiming `textures/basecolor.dds`.
        let repacked = repack(&separate_occlusion_glb(false), "tank").expect("repack");
        let model = import_model(&repacked.glb, ModelLimits::default()).expect("reimport");
        let base = model.materials[0]
            .base_color_texture
            .expect("a base colour");
        assert_eq!(model.images[base].name, "tank_basecolor");
        assert!(
            repacked
                .textures
                .iter()
                .any(|(name, _)| name == "tank_basecolor"),
            "the sidecar is named the same, got {:?}",
            repacked.textures.iter().map(|(n, _)| n).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_rewritten_model_keeps_its_geometry_and_shrinks() {
        // The rewrite drops every image's buffer view and compacts the binary chunk, which means rewriting
        // the offset of every view kept. An accessor references a view by index and the indices do not
        // move -- but if an offset were wrong the geometry would come back as noise, so it is asserted
        // rather than assumed.
        let original = separate_occlusion_glb(true);
        let before = import_model(&original, ModelLimits::default()).expect("import");
        let repacked = repack(&original, "hull").expect("repack");
        let after = import_model(&repacked.glb, ModelLimits::default()).expect("reimport");

        assert_eq!(after.primitives.len(), before.primitives.len());
        assert_eq!(
            after.primitives[0].vertices.len(),
            before.primitives[0].vertices.len()
        );
        for (new, old) in after.primitives[0]
            .vertices
            .iter()
            .zip(&before.primitives[0].vertices)
        {
            assert_eq!(new.position, old.position, "a vertex moved");
        }
        assert_eq!(after.materials[0].name, "hull", "the material survived");
        assert!(
            repacked.glb.len() < original.len(),
            "the rewrite must shrink the container: {} against {}",
            repacked.glb.len(),
            original.len()
        );
        // Every image is now the placeholder, which is what makes it smaller.
        for image in &after.images {
            assert_eq!(
                (image.width, image.height),
                (1, 1),
                "an image was left at full size"
            );
        }
    }

    #[test]
    fn a_container_it_cannot_faithfully_rewrite_is_refused() {
        // Refusing beats a silent loss. An unknown chunk would be dropped by this rewriter, and the
        // specification says a reader should skip one -- so skipping is right for a *reader* and wrong for
        // something that writes the file back out.
        let mut with_extra = separate_occlusion_glb(true);
        with_extra.extend_from_slice(&4u32.to_le_bytes());
        with_extra.extend_from_slice(&u32::from_le_bytes(*b"ZOOM").to_le_bytes());
        with_extra.extend_from_slice(&[0u8; 4]);
        let error = split_glb(&with_extra).expect_err("an unknown chunk must be refused");
        assert!(error.contains("would drop"), "got {error}");

        assert!(
            split_glb(b"not a glb at all").is_err(),
            "and so must bytes that are not a container"
        );
    }
}

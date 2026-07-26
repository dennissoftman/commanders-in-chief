// Commanders in Chief
// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only
//
// Provenance: the mapped-image load order, the localized definition paths, and the language path
// component are derived from Electronic Arts' GPL-3.0 source release, GeneralsGameCode revision
// 9f7abb866f5afd446db14149979e744c7216baaf, specifically
// `Core/GameEngine/Source/GameClient/System/Image.cpp` (`ImageCollection::load`),
// `Core/GameEngine/Source/Common/INI/INI.cpp` (`INI::loadDirectory`, `INI::loadFileDirectory`),
// `Core/GameEngine/Source/GameClient/GlobalLanguage.cpp` (`GlobalLanguage::init`),
// `Core/GameEngine/Source/GameClient/GUI/HeaderTemplate.cpp` (`HeaderTemplateManager::init`),
// `GeneralsMD/Code/GameEngine/Source/GameClient/GameClient.cpp`
// (`TheMappedImageCollection->load( 512 )`), and `GeneralsMD/Code/Main/WinMain.cpp`
// (`data\%s\Generals.csf`). Composition, reporting, and every diagnostic here are project design.

//! Resolution of the UI definition resources a decoded WND layout names.
//!
//! A WND document names mapped images, font families, header templates, and localized labels as
//! strings. Nothing in the parser resolves them: this module composes the mounted VFS, the narrow
//! UI INI decoders, and the CSF decoder into an immutable resolution result where every demand is
//! either bound to a definition with its defining file recorded, or explicitly missing.
//!
//! Load order follows the original client exactly, because a mod that relies on an override must
//! resolve the same way here:
//!
//! - mapped images load from `Data/INI/MappedImages/TextureSize_<N>` and then
//!   `Data/INI/MappedImages/HandCreated`, so a hand-created definition overrides a packed one of
//!   the same name. The client selects one texture-size directory rather than merging all of them.
//! - within one directory, files that sit directly in it load before files in its subdirectories,
//!   each group in sorted order, which the source comments describe as keeping machines in step.
//! - header templates, fonts, and labels load from `Data/<Language>/`, so resolution needs the
//!   localization archive mounted alongside the window archive.

use std::collections::BTreeMap;

use cic_formats::{
    CsfFile, CsfLimits, HeaderTemplateIni, LanguageFontRole, LanguageIni, MappedImage,
    UiIniDiagnostic, UiIniError, UiIniLimits, WndDocument, WndWindow, parse_csf,
    parse_header_template_ini, parse_language_ini, parse_mapped_image_ini,
};
use cic_vfs::{ResourceReadError, Vfs, VirtualPath};

/// The texture-page size the original client selects.
///
/// `GameClient::init` calls `TheMappedImageCollection->load( 512 )` with a literal, so
/// `TextureSize_512` is the shipped selection and any other `TextureSize_<N>` directory present in
/// the data is deliberately not loaded.
pub const DEFAULT_MAPPED_IMAGE_TEXTURE_SIZE: u32 = 512;

/// The directory every mapped-image definition file lives beneath.
const MAPPED_IMAGE_ROOT: &str = "data/ini/mappedimages";

/// Explicit bounds for UI resource resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiResourceLimits {
    /// Per-file bounds handed to each narrow INI decoder.
    pub ini: UiIniLimits,
    /// Bounds handed to the CSF decoder.
    pub csf: CsfLimits,
    /// Maximum definition files loaded from one directory tree.
    pub max_definition_files: usize,
    /// Maximum definitions retained in the merged catalog.
    pub max_catalog_definitions: usize,
}

impl Default for UiResourceLimits {
    fn default() -> Self {
        Self {
            ini: UiIniLimits::default(),
            csf: CsfLimits::default(),
            max_definition_files: 4_096,
            max_catalog_definitions: 65_536,
        }
    }
}

/// A structured UI resource resolution failure.
#[derive(Debug)]
pub enum UiResourceError {
    /// A definition file could not be read from the VFS.
    Read {
        /// The virtual path being read.
        path: VirtualPath,
        /// The underlying failure.
        error: ResourceReadError,
    },
    /// A definition file was enumerated but no longer resolves.
    Vanished {
        /// The virtual path that stopped resolving.
        path: VirtualPath,
    },
    /// A definition file could not be decoded.
    Ini {
        /// The virtual path being decoded.
        path: VirtualPath,
        /// The underlying failure.
        error: UiIniError,
    },
    /// The localized string table could not be decoded.
    Csf {
        /// The virtual path being decoded.
        path: VirtualPath,
        /// The underlying failure, rendered because the CSF error type is not `Clone`.
        message: String,
    },
    /// One directory tree holds more definition files than the configured limit.
    TooManyDefinitionFiles {
        /// The directory prefix being loaded.
        prefix: String,
        /// The configured limit.
        limit: usize,
    },
    /// The merged catalog holds more definitions than the configured limit.
    TooManyCatalogDefinitions {
        /// The configured limit.
        limit: usize,
    },
    /// A language name cannot form a virtual path.
    InvalidLanguage {
        /// The rejected language name.
        language: String,
    },
}

impl std::fmt::Display for UiResourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, error } => write!(formatter, "cannot read {path}: {error}"),
            Self::Vanished { path } => write!(formatter, "{path} no longer resolves"),
            Self::Ini { path, error } => write!(formatter, "cannot decode {path}: {error}"),
            Self::Csf { path, message } => write!(formatter, "cannot decode {path}: {message}"),
            Self::TooManyDefinitionFiles { prefix, limit } => write!(
                formatter,
                "{prefix} holds more than the {limit}-definition-file limit"
            ),
            Self::TooManyCatalogDefinitions { limit } => write!(
                formatter,
                "mapped-image catalog exceeds the {limit}-definition limit"
            ),
            Self::InvalidLanguage { language } => {
                write!(
                    formatter,
                    "language {language:?} is not a usable path component"
                )
            }
        }
    }
}

impl std::error::Error for UiResourceError {}

/// One definition file that contributed to a catalog, with its own diagnostics.
#[derive(Debug, Clone)]
pub struct DefinitionFile {
    path: VirtualPath,
    definitions: usize,
    diagnostics: Vec<UiIniDiagnostic>,
}

impl DefinitionFile {
    /// Returns the virtual path the definitions came from.
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    /// Returns how many definitions the file declared.
    #[must_use]
    pub const fn definitions(&self) -> usize {
        self.definitions
    }

    /// Returns the file's non-fatal decode observations.
    #[must_use]
    pub fn diagnostics(&self) -> &[UiIniDiagnostic] {
        &self.diagnostics
    }
}

/// A mapped-image definition together with the file that won it.
#[derive(Debug, Clone)]
pub struct CatalogedImage {
    image: MappedImage,
    source: VirtualPath,
}

impl CatalogedImage {
    /// Returns the winning definition.
    #[must_use]
    pub const fn image(&self) -> &MappedImage {
        &self.image
    }

    /// Returns the file that declared the winning definition.
    #[must_use]
    pub const fn source(&self) -> &VirtualPath {
        &self.source
    }
}

/// The merged mapped-image catalog for one texture-size selection.
#[derive(Debug, Clone, Default)]
pub struct MappedImageCatalog {
    images: BTreeMap<Vec<u8>, CatalogedImage>,
    files: Vec<DefinitionFile>,
    unselected_files: Vec<VirtualPath>,
    overrides: Vec<ImageOverride>,
}

/// A later definition file replacing an earlier definition of the same name.
#[derive(Debug, Clone)]
pub struct ImageOverride {
    name: Vec<u8>,
    previous: VirtualPath,
    winner: VirtualPath,
}

impl ImageOverride {
    /// Returns the definition name that was redefined.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// Returns the file whose definition was replaced.
    #[must_use]
    pub const fn previous(&self) -> &VirtualPath {
        &self.previous
    }

    /// Returns the file whose definition won.
    #[must_use]
    pub const fn winner(&self) -> &VirtualPath {
        &self.winner
    }
}

impl MappedImageCatalog {
    /// Returns the definition bound to a name, matched case-insensitively as the source's
    /// lowercased name key does.
    #[must_use]
    pub fn find(&self, name: &[u8]) -> Option<&CatalogedImage> {
        self.images.get(&name.to_ascii_lowercase())
    }

    /// Returns every definition, in case-insensitive name order.
    pub fn images(&self) -> impl Iterator<Item = &CatalogedImage> {
        self.images.values()
    }

    /// Returns how many definitions the catalog holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.images.len()
    }

    /// Returns whether the catalog is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Returns every file that contributed, in load order.
    #[must_use]
    pub fn files(&self) -> &[DefinitionFile] {
        &self.files
    }

    /// Returns definition files present under `Data/INI/MappedImages` that the active texture-size
    /// selection does not load, so an unloaded variant directory stays visible.
    #[must_use]
    pub fn unselected_files(&self) -> &[VirtualPath] {
        &self.unselected_files
    }

    /// Returns every name a later file redefined, in load order.
    #[must_use]
    pub fn overrides(&self) -> &[ImageOverride] {
        &self.overrides
    }
}

/// Loads the mapped-image catalog for one texture-size selection.
///
/// # Errors
///
/// Returns an error when a definition file cannot be read or decoded, or when the file or
/// definition count exceeds its configured limit.
pub fn load_mapped_image_catalog(
    vfs: &Vfs,
    texture_size: u32,
    limits: UiResourceLimits,
) -> Result<MappedImageCatalog, UiResourceError> {
    let selected = format!("{MAPPED_IMAGE_ROOT}/texturesize_{texture_size}");
    let hand_created = format!("{MAPPED_IMAGE_ROOT}/handcreated");
    let mut catalog = MappedImageCatalog::default();

    for prefix in [selected.as_str(), hand_created.as_str()] {
        for path in ini_files_in_load_order(vfs, prefix, limits.max_definition_files)? {
            let bytes = read_definition(vfs, &path, limits.ini.max_file_bytes)?;
            let ini = parse_mapped_image_ini(&bytes, limits.ini).map_err(|error| {
                UiResourceError::Ini {
                    path: path.clone(),
                    error,
                }
            })?;
            for image in ini.images() {
                let key = image.name_bytes().to_ascii_lowercase();
                if let Some(previous) = catalog.images.get(&key) {
                    catalog.overrides.push(ImageOverride {
                        name: image.name_bytes().to_vec(),
                        previous: previous.source.clone(),
                        winner: path.clone(),
                    });
                } else if catalog.images.len() >= limits.max_catalog_definitions {
                    return Err(UiResourceError::TooManyCatalogDefinitions {
                        limit: limits.max_catalog_definitions,
                    });
                }
                catalog.images.insert(
                    key,
                    CatalogedImage {
                        image: image.clone(),
                        source: path.clone(),
                    },
                );
            }
            catalog.files.push(DefinitionFile {
                path,
                definitions: ini.images().len(),
                diagnostics: ini.diagnostics().to_vec(),
            });
        }
    }

    let loaded: Vec<&str> = catalog
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    for (path, _) in vfs.iter_resolved() {
        if is_ini_under(path, MAPPED_IMAGE_ROOT) && !loaded.contains(&path.as_str()) {
            catalog.unselected_files.push(path.clone());
        }
    }

    Ok(catalog)
}

/// The localized definition resources for one language.
#[derive(Debug)]
pub struct LocalizationResources {
    language: String,
    text: LanguageIni,
    text_files: Vec<DefinitionFile>,
    header_templates: Vec<(VirtualPath, HeaderTemplateIni)>,
    labels: Option<(VirtualPath, CsfFile)>,
}

impl LocalizationResources {
    /// Returns the selected language name.
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Returns the merged text presentation policy.
    #[must_use]
    pub const fn text(&self) -> &LanguageIni {
        &self.text
    }

    /// Returns every `Language.ini` file that contributed, in load order.
    #[must_use]
    pub fn text_files(&self) -> &[DefinitionFile] {
        &self.text_files
    }

    /// Returns every header-template file that contributed, in load order.
    #[must_use]
    pub fn header_template_files(&self) -> &[(VirtualPath, HeaderTemplateIni)] {
        &self.header_templates
    }

    /// Returns the header template with this exact name, preferring the last file to declare it.
    #[must_use]
    pub fn find_header_template(
        &self,
        name: &[u8],
    ) -> Option<(&VirtualPath, &cic_formats::HeaderTemplate)> {
        self.header_templates
            .iter()
            .rev()
            .find_map(|(path, ini)| ini.find(name).map(|template| (path, template)))
    }

    /// Returns the decoded string table and its path, absent when the language ships none.
    #[must_use]
    pub fn labels(&self) -> Option<(&VirtualPath, &CsfFile)> {
        self.labels.as_ref().map(|(path, csf)| (path, csf))
    }

    /// Returns whether the string table defines this label, matched case-insensitively.
    #[must_use]
    pub fn defines_label(&self, label: &str) -> bool {
        let Some((_, csf)) = self.labels.as_ref() else {
            return false;
        };
        csf.labels()
            .iter()
            .any(|entry| entry.name_bytes().eq_ignore_ascii_case(label.as_bytes()))
    }
}

/// Loads `Data/<Language>/` header templates, font policy, and string table.
///
/// Each of `Language.ini` and `HeaderTemplate.ini` is loaded as a file and then as a directory of
/// additional files, matching `INI::loadFileDirectory`. A resource the language does not ship is
/// simply absent, which the report renders as unresolved rather than as a failure.
///
/// # Errors
///
/// Returns an error when a definition file cannot be read or decoded, or when the language name
/// cannot form a virtual path.
pub fn load_localization_resources(
    vfs: &Vfs,
    language: &str,
    limits: UiResourceLimits,
) -> Result<LocalizationResources, UiResourceError> {
    let root = format!("data/{}", language.to_ascii_lowercase());
    let mut text = LanguageIni::default();
    let mut text_files = Vec::new();
    for path in file_then_directory(vfs, &root, "language", limits.max_definition_files)? {
        let bytes = read_definition(vfs, &path, limits.ini.max_file_bytes)?;
        let decoded =
            parse_language_ini(&bytes, limits.ini).map_err(|error| UiResourceError::Ini {
                path: path.clone(),
                error,
            })?;
        text_files.push(DefinitionFile {
            path,
            definitions: usize::from(decoded.is_declared()),
            diagnostics: decoded.diagnostics().to_vec(),
        });
        // Later files overwrite the whole policy, as the singleton is re-parsed in place and every
        // field a later file omits keeps the value an earlier file set. A field-level merge would
        // need the source's per-field store, which is not observable from the decoded value, so the
        // last declaring file wins and the earlier one stays reported.
        if decoded.is_declared() {
            text = decoded;
        }
    }

    let mut header_templates = Vec::new();
    for path in file_then_directory(vfs, &root, "headertemplate", limits.max_definition_files)? {
        let bytes = read_definition(vfs, &path, limits.ini.max_file_bytes)?;
        let decoded = parse_header_template_ini(&bytes, limits.ini).map_err(|error| {
            UiResourceError::Ini {
                path: path.clone(),
                error,
            }
        })?;
        header_templates.push((path, decoded));
    }

    let labels = match VirtualPath::new(&format!("{root}/generals.csf")) {
        Ok(path) => match vfs.resolve(&path) {
            Some(entry) => {
                let bytes = entry.read(limits.csf.maximum_file_bytes).map_err(|error| {
                    UiResourceError::Read {
                        path: path.clone(),
                        error,
                    }
                })?;
                let csf = parse_csf(&bytes, path.as_str(), limits.csf).map_err(|error| {
                    UiResourceError::Csf {
                        path: path.clone(),
                        message: error.to_string(),
                    }
                })?;
                Some((path, csf))
            }
            None => None,
        },
        Err(_) => {
            return Err(UiResourceError::InvalidLanguage {
                language: language.to_owned(),
            });
        }
    };

    Ok(LocalizationResources {
        language: language.to_owned(),
        text,
        text_files,
        header_templates,
        labels,
    })
}

/// What kind of UI resource one demand names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiResourceKind {
    /// A mapped-image name from a draw-data entry.
    MappedImage,
    /// A font family from a `FONT` record.
    Font,
    /// A header template name from a `HEADERTEMPLATE` record.
    HeaderTemplate,
    /// A localized label from a `TEXT` or `TOOLTIPTEXT` record.
    Label,
}

impl UiResourceKind {
    /// Returns the stable report row name for this kind.
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::MappedImage => "image",
            Self::Font => "font",
            Self::HeaderTemplate => "header_template",
            Self::Label => "label",
        }
    }
}

/// One place a layout named a resource.
#[derive(Debug, Clone)]
pub struct DemandSite {
    window_id: usize,
    window_name: Option<String>,
    detail: String,
}

impl DemandSite {
    /// Returns the source-order id of the window that named the resource.
    #[must_use]
    pub const fn window_id(&self) -> usize {
        self.window_id
    }

    /// Returns the window's decorated control name, when it has one.
    #[must_use]
    pub fn window_name(&self) -> Option<&str> {
        self.window_name.as_deref()
    }

    /// Returns which record named the resource.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// One distinct resource name a layout demands, with every site that named it.
#[derive(Debug, Clone)]
pub struct UiResourceDemand {
    kind: UiResourceKind,
    name: String,
    sites: Vec<DemandSite>,
}

impl UiResourceDemand {
    /// Returns which kind of resource is demanded.
    #[must_use]
    pub const fn kind(&self) -> UiResourceKind {
        self.kind
    }

    /// Returns the demanded name exactly as the layout spelled it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns every site that named it, in source order.
    #[must_use]
    pub fn sites(&self) -> &[DemandSite] {
        &self.sites
    }
}

/// Collects every UI resource a decoded layout names, deduplicated and in stable order.
///
/// A `TEXT` or `TOOLTIPTEXT` value is treated as a localized label only when it is label-shaped —
/// a non-empty `namespace:key` pair with no whitespace. Retail layouts also carry literal strings
/// in those records, which is why the parser retains the field unclassified; a literal is reported
/// as a literal rather than as a missing label.
#[must_use]
pub fn collect_ui_resource_demand(document: &WndDocument) -> Vec<UiResourceDemand> {
    let mut collector = DemandCollector::default();
    for window in document.windows() {
        collector.visit(window);
    }
    collector.finish()
}

/// Returns whether a record value is the source's explicit "no resource" placeholder.
///
/// The WND writer emits `[None]` where a layout selects nothing, the same spelling the layout block
/// uses for an absent callback. Retail spells it in both cases and cases: `MainMenu.wnd` alone
/// carries `[NONE]` on 31 controls and `[None]` on two. A placeholder is an absent demand, not a
/// missing resource, so it never reaches resolution.
#[must_use]
pub fn is_none_placeholder(value: &str) -> bool {
    value.eq_ignore_ascii_case("[None]")
}

/// Returns whether a `TEXT` value is shaped like a CSF label rather than a literal string.
#[must_use]
pub fn is_label_shaped(value: &str) -> bool {
    let mut parts = value.splitn(2, ':');
    let (Some(namespace), Some(key)) = (parts.next(), parts.next()) else {
        return false;
    };
    !namespace.is_empty()
        && !key.is_empty()
        && !value.contains(char::is_whitespace)
        && !key.contains(':')
}

#[derive(Default)]
struct DemandCollector {
    entries: BTreeMap<(UiResourceKind, String), Vec<DemandSite>>,
    order: Vec<(UiResourceKind, String)>,
}

impl DemandCollector {
    fn visit(&mut self, window: &WndWindow) {
        let site = |detail: String| DemandSite {
            window_id: window.id(),
            window_name: window.name().map(str::to_owned),
            detail,
        };
        if let Some(font) = window
            .font()
            .filter(|font| !is_none_placeholder(font.name()))
        {
            self.push(
                UiResourceKind::Font,
                font.name(),
                site(format!("FONT {} {}", font.size(), font.bold())),
            );
        }
        if let Some(template) = window
            .header_template()
            .filter(|name| !is_none_placeholder(name))
        {
            self.push(
                UiResourceKind::HeaderTemplate,
                template,
                site("HEADERTEMPLATE".to_owned()),
            );
        }
        for (field, value) in [
            ("TEXT", window.text()),
            ("TOOLTIPTEXT", window.tooltip_text()),
        ] {
            if let Some(value) = value.filter(|value| {
                !value.is_empty() && !is_none_placeholder(value) && is_label_shaped(value)
            }) {
                self.push(UiResourceKind::Label, value, site(field.to_owned()));
            }
        }
        for (slot, data) in window.draw_data() {
            for (index, entry) in data.entries().iter().enumerate() {
                if let Some(image) = entry.image() {
                    self.push(
                        UiResourceKind::MappedImage,
                        image,
                        site(format!(
                            "{} entry {index}",
                            crate::wnd_draw_slot_name(*slot)
                        )),
                    );
                }
            }
        }
        for child in window.children() {
            self.visit(child);
        }
    }

    fn push(&mut self, kind: UiResourceKind, name: &str, site: DemandSite) {
        let key = (kind, name.to_owned());
        if !self.entries.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.entries.entry(key).or_default().push(site);
    }

    fn finish(mut self) -> Vec<UiResourceDemand> {
        self.order
            .into_iter()
            .filter_map(|key| {
                let sites = self.entries.remove(&key)?;
                Some(UiResourceDemand {
                    kind: key.0,
                    name: key.1,
                    sites,
                })
            })
            .collect()
    }
}

/// How one demand resolved.
#[derive(Debug, Clone)]
pub enum UiResourceBinding {
    /// A mapped image bound to a definition, whose texture file may still be absent.
    Image {
        /// The file that declared the winning definition.
        definition: VirtualPath,
        /// The texture file the definition names.
        texture: String,
        /// The texture file's resolved virtual path, absent when no mounted file matches.
        texture_path: Option<VirtualPath>,
        /// The presentation size in pixels.
        size: (i32, i32),
    },
    /// A header template bound to a definition.
    HeaderTemplate {
        /// The file that declared it.
        definition: VirtualPath,
        /// The font family it selects.
        font: String,
        /// The point size it selects.
        point: i32,
        /// Whether it selects a bold face.
        bold: bool,
    },
    /// A font family named by the layout, matched against the language's declared families.
    Font {
        /// The role whose declaration matched, when one did.
        role: Option<&'static str>,
        /// Whether a `LocalFontFile` supplies the family as a file.
        local_font_file: Option<String>,
    },
    /// A localized label bound to a string table entry.
    Label {
        /// The string table that defines it.
        definition: VirtualPath,
    },
    /// The demand did not resolve.
    Missing,
}

/// One demand together with its binding.
#[derive(Debug, Clone)]
pub struct ResolvedUiResource {
    demand: UiResourceDemand,
    binding: UiResourceBinding,
}

impl ResolvedUiResource {
    /// Returns the demand.
    #[must_use]
    pub const fn demand(&self) -> &UiResourceDemand {
        &self.demand
    }

    /// Returns the binding.
    #[must_use]
    pub const fn binding(&self) -> &UiResourceBinding {
        &self.binding
    }

    /// Returns whether the demand resolved.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        !matches!(self.binding, UiResourceBinding::Missing)
    }
}

/// The complete resolution of one layout's UI resource demand.
#[derive(Debug)]
pub struct UiResourceResolution {
    resources: Vec<ResolvedUiResource>,
}

impl UiResourceResolution {
    /// Returns every demand with its binding, in stable demand order.
    #[must_use]
    pub fn resources(&self) -> &[ResolvedUiResource] {
        &self.resources
    }

    /// Returns how many demands of one kind resolved, and how many did not.
    #[must_use]
    pub fn counts(&self, kind: UiResourceKind) -> (usize, usize) {
        let of_kind = self
            .resources
            .iter()
            .filter(|resource| resource.demand.kind == kind);
        let mut resolved = 0;
        let mut missing = 0;
        for resource in of_kind {
            if resource.is_resolved() {
                resolved += 1;
            } else {
                missing += 1;
            }
        }
        (resolved, missing)
    }
}

/// Binds every demand against the loaded catalogs.
///
/// A font family demand resolves when the selected language declares that family for any role or
/// supplies it as a local font file. Retail ships no font files at all, so an unmatched family is
/// expected and reported rather than treated as corruption; deterministic captures substitute a
/// project-supplied face instead of a host font.
#[must_use]
pub fn resolve_ui_resources(
    demand: Vec<UiResourceDemand>,
    images: &MappedImageCatalog,
    localization: &LocalizationResources,
    texture_lookup: &dyn Fn(&str) -> Option<VirtualPath>,
) -> UiResourceResolution {
    let resources = demand
        .into_iter()
        .map(|demand| {
            let binding = match demand.kind {
                UiResourceKind::MappedImage => images.find(demand.name.as_bytes()).map_or(
                    UiResourceBinding::Missing,
                    |cataloged| {
                        let texture =
                            String::from_utf8_lossy(cataloged.image().texture_bytes()).into_owned();
                        UiResourceBinding::Image {
                            definition: cataloged.source().clone(),
                            texture_path: if texture.is_empty() {
                                None
                            } else {
                                texture_lookup(&texture)
                            },
                            texture,
                            size: cataloged.image().image_size(),
                        }
                    },
                ),
                UiResourceKind::HeaderTemplate => localization
                    .find_header_template(demand.name.as_bytes())
                    .map_or(UiResourceBinding::Missing, |(path, template)| {
                        UiResourceBinding::HeaderTemplate {
                            definition: path.clone(),
                            font: String::from_utf8_lossy(template.font_name_bytes()).into_owned(),
                            point: template.point(),
                            bold: template.bold(),
                        }
                    }),
                UiResourceKind::Font => resolve_font(&demand.name, localization),
                UiResourceKind::Label => {
                    if localization.defines_label(&demand.name) {
                        localization
                            .labels()
                            .map_or(UiResourceBinding::Missing, |(path, _)| {
                                UiResourceBinding::Label {
                                    definition: path.clone(),
                                }
                            })
                    } else {
                        UiResourceBinding::Missing
                    }
                }
            };
            ResolvedUiResource { demand, binding }
        })
        .collect();
    UiResourceResolution { resources }
}

fn resolve_font(family: &str, localization: &LocalizationResources) -> UiResourceBinding {
    let text = localization.text();
    let role = LanguageFontRole::ALL.into_iter().find(|role| {
        text.font(*role)
            .name_bytes()
            .eq_ignore_ascii_case(family.as_bytes())
    });
    let unicode_match = text
        .unicode_font_name_bytes()
        .eq_ignore_ascii_case(family.as_bytes())
        && !text.unicode_font_name_bytes().is_empty();
    let local_font_file = localization
        .text()
        .local_font_files()
        .iter()
        .find(|file| {
            let stem = file
                .split(|byte| *byte == b'.')
                .next()
                .unwrap_or(file.as_slice());
            stem.eq_ignore_ascii_case(family.as_bytes())
        })
        .map(|file| String::from_utf8_lossy(file).into_owned());
    if role.is_none() && !unicode_match && local_font_file.is_none() {
        return UiResourceBinding::Missing;
    }
    UiResourceBinding::Font {
        role: role.map(LanguageFontRole::field_name),
        local_font_file,
    }
}

fn read_definition(
    vfs: &Vfs,
    path: &VirtualPath,
    maximum_bytes: usize,
) -> Result<Vec<u8>, UiResourceError> {
    let entry = vfs
        .resolve(path)
        .ok_or_else(|| UiResourceError::Vanished { path: path.clone() })?;
    entry
        .read(maximum_bytes)
        .map_err(|error| UiResourceError::Read {
            path: path.clone(),
            error,
        })
}

/// Returns every `.ini` under `prefix` in the source's load order: files directly in the directory
/// first, then files in subdirectories, each group sorted by path.
fn ini_files_in_load_order(
    vfs: &Vfs,
    prefix: &str,
    limit: usize,
) -> Result<Vec<VirtualPath>, UiResourceError> {
    let mut direct = Vec::new();
    let mut nested = Vec::new();
    for (path, _) in vfs.iter_resolved() {
        if !is_ini_under(path, prefix) {
            continue;
        }
        let relative = &path.as_str()[prefix.len() + 1..];
        if relative.contains('/') {
            nested.push(path.clone());
        } else {
            direct.push(path.clone());
        }
        if direct.len() + nested.len() > limit {
            return Err(UiResourceError::TooManyDefinitionFiles {
                prefix: prefix.to_owned(),
                limit,
            });
        }
    }
    direct.extend(nested);
    Ok(direct)
}

/// Returns the `<root>/<stem>.ini` file followed by every `.ini` under `<root>/<stem>/`, matching
/// `INI::loadFileDirectory`.
fn file_then_directory(
    vfs: &Vfs,
    root: &str,
    stem: &str,
    limit: usize,
) -> Result<Vec<VirtualPath>, UiResourceError> {
    let mut paths = Vec::new();
    if let Ok(path) = VirtualPath::new(&format!("{root}/{stem}.ini"))
        && vfs.resolve(&path).is_some()
    {
        paths.push(path);
    }
    paths.extend(ini_files_in_load_order(
        vfs,
        &format!("{root}/{stem}"),
        limit,
    )?);
    Ok(paths)
}

fn is_ini_under(path: &VirtualPath, prefix: &str) -> bool {
    let text = path.as_str();
    // Virtual paths are already normalized, but the extension test stays case-insensitive so a
    // loose-directory mount of `.INI` files behaves like the archive it replaces.
    let is_ini = text
        .len()
        .checked_sub(4)
        .and_then(|start| text.get(start..))
        .is_some_and(|extension| extension.eq_ignore_ascii_case(".ini"));
    is_ini
        && text.len() > prefix.len() + 1
        && text.starts_with(prefix)
        && text.as_bytes().get(prefix.len()) == Some(&b'/')
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_MAPPED_IMAGE_TEXTURE_SIZE, UiResourceBinding, UiResourceKind, UiResourceLimits,
        collect_ui_resource_demand, is_label_shaped, is_none_placeholder,
        load_localization_resources, load_mapped_image_catalog, resolve_ui_resources,
    };
    use cic_formats::{WndLimits, parse_wnd};
    use cic_vfs::{Vfs, VirtualPath};

    fn path(text: &str) -> VirtualPath {
        VirtualPath::new(text).expect("virtual path")
    }

    fn synthetic_vfs() -> Vfs {
        let mut vfs = Vfs::new();
        vfs.mount_memory(
            "synthetic-ui",
            [
                (
                    path("Data/INI/MappedImages/TextureSize_512/SynthPacked.ini"),
                    b"MappedImage SynthButtonEnabled\n  Texture = SynthPage.tga\n  \
                      TextureWidth = 512\n  TextureHeight = 512\n  \
                      Coords = Left:0 Top:0 Right:64 Bottom:32\n  Status = NONE\nEnd\n\
                      MappedImage SynthShared\n  Texture = SynthPage.tga\n  \
                      Coords = Left:0 Top:0 Right:8 Bottom:8\nEnd\n"
                        .to_vec(),
                ),
                (
                    path("Data/INI/MappedImages/TextureSize_512/Nested/SynthNested.ini"),
                    b"MappedImage SynthNested\n  Texture = SynthPage.tga\nEnd\n".to_vec(),
                ),
                (
                    path("Data/INI/MappedImages/HandCreated/SynthHand.ini"),
                    b"MappedImage SynthShared\n  Texture = SynthHand.tga\nEnd\n".to_vec(),
                ),
                (
                    path("Data/INI/MappedImages/TextureSize_256/SynthOther.ini"),
                    b"MappedImage SynthUnselected\n  Texture = SynthSmall.tga\nEnd\n".to_vec(),
                ),
                (
                    path("Data/Synthlang/HeaderTemplate.ini"),
                    b"HeaderTemplate SynthHeader\n  Font = \"Synth Sans\"\n  Point = 14\n  \
                      Bold = Yes\nEnd\n"
                        .to_vec(),
                ),
                (
                    path("Data/Synthlang/Language.ini"),
                    b"Language\n  DefaultWindowFont = \"Synth Sans\" 11 No\n  \
                      LocalFontFile = SynthMono.ttf\n  ResolutionFontAdjustment = 0.7\nEnd\n"
                        .to_vec(),
                ),
                (path("Art/Textures/SynthPage.tga"), vec![0; 8]),
            ],
        )
        .expect("mount synthetic ui resources");
        vfs
    }

    #[test]
    fn the_catalog_follows_the_source_load_order_and_reports_overrides() {
        let vfs = synthetic_vfs();
        let catalog = load_mapped_image_catalog(
            &vfs,
            DEFAULT_MAPPED_IMAGE_TEXTURE_SIZE,
            UiResourceLimits::default(),
        )
        .expect("load catalog");

        // Direct files load before subdirectory files, and HandCreated loads last.
        let files: Vec<&str> = catalog
            .files()
            .iter()
            .map(|file| file.path().as_str())
            .collect();
        assert_eq!(
            files,
            [
                "data/ini/mappedimages/texturesize_512/synthpacked.ini",
                "data/ini/mappedimages/texturesize_512/nested/synthnested.ini",
                "data/ini/mappedimages/handcreated/synthhand.ini",
            ]
        );

        // The hand-created definition wins the shared name, and the override is reported.
        let shared = catalog.find(b"synthshared").expect("shared definition");
        assert_eq!(shared.image().texture_bytes(), b"SynthHand.tga");
        assert_eq!(catalog.overrides().len(), 1);
        assert_eq!(catalog.overrides()[0].name_bytes(), b"SynthShared");

        // Lookup is case-insensitive, as the source's lowercased name key is.
        assert!(catalog.find(b"SYNTHBUTTONENABLED").is_some());

        // A texture-size directory the active selection does not load stays visible.
        let unselected: Vec<&str> = catalog
            .unselected_files()
            .iter()
            .map(VirtualPath::as_str)
            .collect();
        assert_eq!(
            unselected,
            ["data/ini/mappedimages/texturesize_256/synthother.ini"]
        );
        assert!(catalog.find(b"synthunselected").is_none());
    }

    #[test]
    fn resolution_binds_images_templates_fonts_and_reports_what_is_missing() {
        let vfs = synthetic_vfs();
        let catalog = load_mapped_image_catalog(
            &vfs,
            DEFAULT_MAPPED_IMAGE_TEXTURE_SIZE,
            UiResourceLimits::default(),
        )
        .expect("load catalog");
        let localization =
            load_localization_resources(&vfs, "Synthlang", UiResourceLimits::default())
                .expect("load localization");
        assert_eq!(localization.header_template_files().len(), 1);
        assert!(localization.text().is_declared());
        assert!(localization.labels().is_none());

        let document = parse_wnd(
            b"FILE_VERSION = 2;\n\
              STARTLAYOUTBLOCK\n\
                LAYOUTINIT = \"[None]\";\n\
                LAYOUTUPDATE = \"[None]\";\n\
                LAYOUTSHUTDOWN = \"[None]\";\n\
              ENDLAYOUTBLOCK\n\
              WINDOW\n\
                WINDOWTYPE = PUSHBUTTON;\n\
                SCREENRECT = UPPERLEFT: 0 0,\n\
                             BOTTOMRIGHT: 100 40,\n\
                             CREATIONRESOLUTION: 800 600;\n\
                NAME = \"SynthMenu.wnd:ButtonSynth\";\n\
                FONT = NAME: \"Synth Sans\", SIZE: 11, BOLD: 0;\n\
                HEADERTEMPLATE = \"SynthHeader\";\n\
                TEXT = \"GUI:SynthLabel\";\n\
                TOOLTIPTEXT = \"A literal tip\";\n\
                ENABLEDDRAWDATA = IMAGE: SynthButtonEnabled, COLOR: 255 255 255 255, \
                                  BORDERCOLOR: 0 0 0 255,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: SynthMissing, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0,\n\
                  IMAGE: NoImage, COLOR: 0 0 0 0, BORDERCOLOR: 0 0 0 0;\n\
              END\n",
            WndLimits::default(),
        )
        .expect("decode synthetic layout");

        let demand = collect_ui_resource_demand(&document);
        // The literal tooltip is not label-shaped, so it is not demanded as a label.
        assert_eq!(demand.len(), 5);
        let resolution = resolve_ui_resources(demand, &catalog, &localization, &|texture| {
            let candidate = path(&format!("art/textures/{texture}"));
            vfs.resolve(&candidate).map(|_| candidate)
        });

        assert_eq!(resolution.counts(UiResourceKind::MappedImage), (1, 1));
        assert_eq!(resolution.counts(UiResourceKind::HeaderTemplate), (1, 0));
        assert_eq!(resolution.counts(UiResourceKind::Font), (1, 0));
        assert_eq!(resolution.counts(UiResourceKind::Label), (0, 1));

        let image = resolution
            .resources()
            .iter()
            .find(|resource| resource.demand().name() == "SynthButtonEnabled")
            .expect("enabled image");
        match image.binding() {
            UiResourceBinding::Image {
                texture,
                texture_path,
                size,
                ..
            } => {
                assert_eq!(texture, "SynthPage.tga");
                assert_eq!(
                    texture_path.as_ref().map(VirtualPath::as_str),
                    Some("art/textures/synthpage.tga")
                );
                assert_eq!(*size, (64, 32));
            }
            other => panic!("expected an image binding, got {other:?}"),
        }
    }

    #[test]
    fn the_none_placeholder_is_an_absent_demand_not_a_missing_resource() {
        let document = parse_wnd(
            b"FILE_VERSION = 2;\n\
              STARTLAYOUTBLOCK\n\
                LAYOUTINIT = \"[None]\";\n\
                LAYOUTUPDATE = \"[None]\";\n\
                LAYOUTSHUTDOWN = \"[None]\";\n\
              ENDLAYOUTBLOCK\n\
              WINDOW\n\
                WINDOWTYPE = PUSHBUTTON;\n\
                SCREENRECT = UPPERLEFT: 0 0,\n\
                             BOTTOMRIGHT: 10 10,\n\
                             CREATIONRESOLUTION: 800 600;\n\
                NAME = \"SynthMenu.wnd:ButtonNone\";\n\
                HEADERTEMPLATE = \"[NONE]\";\n\
                FONT = NAME: \"[None]\", SIZE: 11, BOLD: 0;\n\
                TEXT = \"[None]\";\n\
              END\n",
            WndLimits::default(),
        )
        .expect("decode placeholder layout");
        assert!(collect_ui_resource_demand(&document).is_empty());
        assert!(is_none_placeholder("[None]"));
        assert!(is_none_placeholder("[NONE]"));
        assert!(!is_none_placeholder("None"));
    }

    #[test]
    fn label_shape_separates_labels_from_literal_strings() {
        assert!(is_label_shaped("GUI:Ok"));
        assert!(is_label_shaped("SIDEBAR:ButtonCancel"));
        assert!(!is_label_shaped("A literal string"));
        assert!(!is_label_shaped("NoColonHere"));
        assert!(!is_label_shaped(":LeadingColon"));
        assert!(!is_label_shaped("Trailing:"));
        assert!(!is_label_shaped("GUI:With Space"));
    }
}

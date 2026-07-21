// Copyright (C) 2026 Commanders in Chief contributors
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::RenderError;

const MAX_TEXTURE_DIMENSION: u32 = 8_192;
const MAX_TEXTURE_BYTES: usize = 256 * 1_024 * 1_024;
const MAX_TEXTURE_RESOURCE_BYTES: usize = 512 * 1_024 * 1_024;

/// Stable index of one unique decoded texture image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextureId(usize);

impl TextureId {
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// One bounded straight-alpha RGBA8 image retained once by the renderer resource manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl TextureImage {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// Bounded CPU texture cache with stable alias lookup and content deduplication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureResourceManager {
    images: Vec<TextureImage>,
    aliases: BTreeMap<Vec<u8>, TextureId>,
    fingerprints: BTreeMap<[u8; 32], TextureId>,
    total_bytes: usize,
}

impl Default for TextureResourceManager {
    fn default() -> Self {
        let mut manager = Self {
            images: Vec::new(),
            aliases: BTreeMap::new(),
            fingerprints: BTreeMap::new(),
            total_bytes: 0,
        };
        manager
            .insert(b"__cic_white", 1, 1, vec![u8::MAX; 4])
            .expect("built-in white texture is valid");
        manager
    }
}

impl TextureResourceManager {
    /// Registers an alias and decoded image, reusing an existing image with identical dimensions
    /// and RGBA bytes.
    ///
    /// # Errors
    ///
    /// Returns a structured error for zero/excessive dimensions, byte-length disagreement, or a
    /// resource cache exceeding its aggregate allocation limit.
    pub fn insert(
        &mut self,
        alias: &[u8],
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<TextureId, RenderError> {
        let alias = normalized_alias(alias);
        if let Some(existing) = self.aliases.get(&alias) {
            return Ok(*existing);
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RenderError::TextureTooLarge)?;
        if width == 0
            || height == 0
            || width > MAX_TEXTURE_DIMENSION
            || height > MAX_TEXTURE_DIMENSION
            || expected > MAX_TEXTURE_BYTES
            || rgba.len() != expected
        {
            return Err(RenderError::InvalidTexture);
        }
        let mut hasher = Sha256::new();
        hasher.update(width.to_le_bytes());
        hasher.update(height.to_le_bytes());
        hasher.update(&rgba);
        let fingerprint: [u8; 32] = hasher.finalize().into();
        let id = if let Some(existing) = self.fingerprints.get(&fingerprint) {
            *existing
        } else {
            let total_bytes = self
                .total_bytes
                .checked_add(rgba.len())
                .ok_or(RenderError::TextureTooLarge)?;
            if total_bytes > MAX_TEXTURE_RESOURCE_BYTES {
                return Err(RenderError::TextureTooLarge);
            }
            let id = TextureId(self.images.len());
            self.images.push(TextureImage {
                width,
                height,
                rgba,
            });
            self.fingerprints.insert(fingerprint, id);
            self.total_bytes = total_bytes;
            id
        };
        self.aliases.insert(alias, id);
        Ok(id)
    }

    /// Maps another W3D name to an already registered texture without decoding or allocating it
    /// again.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidTexture`] when the texture ID is not owned by this manager.
    pub fn insert_alias(&mut self, alias: &[u8], texture: TextureId) -> Result<(), RenderError> {
        if texture.index() >= self.images.len() {
            return Err(RenderError::InvalidTexture);
        }
        self.aliases.insert(normalized_alias(alias), texture);
        Ok(())
    }

    #[must_use]
    pub fn contains_alias(&self, alias: &[u8]) -> bool {
        self.aliases.contains_key(&normalized_alias(alias))
    }

    #[must_use]
    pub fn texture(&self, alias: &[u8]) -> Option<TextureId> {
        self.aliases.get(&normalized_alias(alias)).copied()
    }

    #[must_use]
    pub const fn fallback_white(&self) -> TextureId {
        TextureId(0)
    }

    #[must_use]
    pub fn unique_image_count(&self) -> usize {
        self.images.len().saturating_sub(1)
    }

    #[must_use]
    pub fn alias_count(&self) -> usize {
        self.aliases.len().saturating_sub(1)
    }

    pub(crate) fn images(&self) -> &[TextureImage] {
        &self.images
    }
}

fn normalized_alias(alias: &[u8]) -> Vec<u8> {
    alias
        .iter()
        .map(|byte| match byte {
            b'\\' => b'/',
            _ => byte.to_ascii_lowercase(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TextureResourceManager;

    #[test]
    fn aliases_and_identical_rgba_share_one_image() {
        let mut manager = TextureResourceManager::default();
        let first = manager
            .insert(b"ART\\TEXTURES\\A.TGA", 1, 1, vec![1, 2, 3, 4])
            .expect("valid texture");
        let second = manager
            .insert(b"b.dds", 1, 1, vec![1, 2, 3, 4])
            .expect("valid duplicate texture");
        assert_eq!(first, second);
        assert_eq!(manager.texture(b"art/textures/a.tga"), Some(first));
        assert_eq!(manager.unique_image_count(), 1);
        assert_eq!(manager.alias_count(), 2);
    }

    #[test]
    fn rejects_dimension_and_payload_mismatch() {
        let mut manager = TextureResourceManager::default();
        assert!(manager.insert(b"empty", 0, 1, Vec::new()).is_err());
        assert!(manager.insert(b"short", 2, 2, vec![0; 15]).is_err());
    }
}

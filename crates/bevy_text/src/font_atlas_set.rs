use crate::{FontAtlas, FontHinting, FontSmoothing, GlyphCacheKey};
use bevy_asset::Assets;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::resource::Resource;
use bevy_image::Image;
use bevy_platform::collections::{HashMap, HashSet};

/// Identifies the font atlases for a particular font in [`FontAtlasSet`]
///
/// Allows an `f32` font size to be used as a key in a `HashMap`, by its binary representation.
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct FontAtlasKey {
    /// Font data id
    pub id: u32,
    /// Font data index
    pub index: u32,
    /// Font size via `f32::to_bits`
    pub font_size_bits: u32,
    /// Hash of normalized variation coords for this run.
    pub variations_hash: u64,
    /// Hinting
    pub hinting: FontHinting,
    /// Antialiasing method
    pub font_smoothing: FontSmoothing,
}

/// Set of rasterized fonts stored in [`FontAtlas`]es.
#[derive(Debug, Default, Resource, Deref, DerefMut)]
pub struct FontAtlasSet {
    #[deref]
    atlases: HashMap<FontAtlasKey, Vec<FontAtlas>>,
    /// The fonts, by the id of their data ([`Font::data`](crate::Font::data)), whose glyph
    /// outlines the `legacy_glyph_weight` feature scales by 64/60: Godot 4.7.2 draws only its
    /// MSDF font imports so (`ft_point2` is the MSDF rasteriser's), while a plain font, such as
    /// its default theme font, is a `FreeType` raster at its size. A font must be listed before its
    /// first glyph is rasterised, since its atlases are kept.
    pub legacy_glyph_weight_fonts: HashSet<u64>,
    /// The fonts, by the id of their data, whose glyph outlines are sheared `x + slant * y`
    /// (y up) before they are rasterised: Godot's `FontVariation.variation_transform` slant, as
    /// its default theme's italics font carries `Transform2D(1, 0.2, 0, 1, 0, 0)`. Advances are
    /// left alone, as Godot's synthetic slant leaves them. A slanted face is its own `Font`
    /// asset (its own data id), so its atlases are its own; list it before its first glyph is
    /// rasterised.
    pub font_slants: HashMap<u64, f32>,
}

impl FontAtlasSet {
    /// Checks whether the given subpixel-offset glyph is contained in any of the [`FontAtlas`]es for the font identified by the given [`FontAtlasKey`].
    pub fn has_glyph(&self, cache_key: GlyphCacheKey, font_key: &FontAtlasKey) -> bool {
        self.get(font_key)
            .is_some_and(|font_atlas| font_atlas.iter().any(|atlas| atlas.has_glyph(cache_key)))
    }

    /// Returns the total size in bytes of the image data for all fonts.
    pub fn total_bytes(&self, images: &Assets<Image>) -> u64 {
        self.values()
            .flat_map(|font_atlases| font_atlases.iter())
            .map(|font_atlas| {
                images
                    .get(&font_atlas.texture)
                    .and_then(|image| image.data.as_ref())
                    .map_or(0, |data| data.len() as u64)
            })
            .sum()
    }

    /// Clear all font atlases
    pub fn clear(&mut self) {
        self.atlases.clear();
    }
}

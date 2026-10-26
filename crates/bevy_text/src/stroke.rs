//! Stroked and emboldened glyph rasters: a text drawn as the outline of its glyphs.

use bevy_asset::Assets;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::prelude::*;
use bevy_image::Image;
use bevy_math::Vec2;
use bevy_platform::collections::HashMap;
use bevy_platform::hash::FixedHasher;
use bevy_reflect::{std_traits::ReflectDefault, Reflect};
use core::hash::BuildHasher;
use parley::PositionedLayoutItem;
use swash::FontRef;

use crate::{
    add_stroke_glyph_to_atlas, get_glyph_atlas_info, ComputedTextBlock, FontAtlas, FontAtlasKey,
    FontAtlasSet, FontHinting, FontSmoothing, GlyphCacheKey, GlyphStroke, PositionedGlyph, ScaleCx,
    TextError, TextLayoutInfo,
};

/// Draws a text's glyphs grown by half of `width_px` (a stroke of that width centred on the
/// outline, unioned with the fill), or filled (`width_px` 0), with the outline emboldened first.
///
/// [`update_stroke_glyphs`] swaps the glyphs of the entity's [`TextLayoutInfo`] for these rasters
/// after layout, so a `Text2d` with this component in an outline colour, under a copy of itself,
/// draws that copy's outline. The positions, the bounds and the wrapping stay the fill's.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default, Debug, Clone)]
pub struct StrokeGlyph {
    /// Width of the stroke in text pixels (the font size's), centred on the glyph's outline, so
    /// half of it lies outside the fill; the inside stays filled.
    pub width_px: f32,
    /// Godot's `FontVariation.variation_embolden`: stems grow by `embolden * font_size / 16`
    /// pixels in total before the stroke (`FT_Outline_Embolden`, Godot `text_server_adv.cpp`).
    pub embolden: f32,
}

impl StrokeGlyph {
    /// The raster's [`GlyphStroke`] at `font_size` physical pixels and `scale_factor` physical
    /// pixels per text pixel.
    pub fn at(&self, font_size: f32, scale_factor: f32) -> GlyphStroke {
        GlyphStroke {
            width: self.width_px * scale_factor,
            embolden: self.embolden * font_size / 16.0,
            legacy_weight: false,
            slant: 0.0,
        }
    }
}

/// Identifies the atlases of one font, size and [`GlyphStroke`] in [`StrokeAtlasSet`].
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct StrokeAtlasKey {
    /// The fill's atlas key.
    pub font: FontAtlasKey,
    /// [`GlyphStroke::width`] via `f32::to_bits`.
    pub width_bits: u32,
    /// [`GlyphStroke::embolden`] via `f32::to_bits`.
    pub embolden_bits: u32,
}

/// The stroked and emboldened rasters, beside the fills of [`FontAtlasSet`](crate::FontAtlasSet).
#[derive(Debug, Default, Resource, Deref, DerefMut)]
pub struct StrokeAtlasSet(HashMap<StrokeAtlasKey, Vec<FontAtlas>>);

/// System set in [`PostUpdate`](bevy_app::PostUpdate) where [`update_stroke_glyphs`] runs,
/// after [`Text2dUpdateSystems`](crate::Text2dUpdateSystems).
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub struct StrokeGlyphSystems;

/// Lays out `stroke`'s rasters of the glyphs of `computed`, whose layout
/// [`TextPipeline::update_text_layout_info`](crate::TextPipeline::update_text_layout_info) has
/// placed: one [`PositionedGlyph`] per glyph of that call's, in its order and at its pen
/// positions, each offset by its own raster's placement. The fonts in `fills`'
/// [`FontAtlasSet::legacy_glyph_weight_fonts`] and [`FontAtlasSet::font_slants`] are scaled and
/// sheared as their fills are.
pub fn stroke_glyph_layout(
    computed: &ComputedTextBlock,
    stroke: StrokeGlyph,
    hinting: FontHinting,
    fills: &FontAtlasSet,
    atlas_set: &mut StrokeAtlasSet,
    textures: &mut Assets<Image>,
    scale_cx: &mut ScaleCx,
) -> Result<Vec<PositionedGlyph>, TextError> {
    let layout = &computed.layout;
    let scale_factor = layout.scale();
    let mut glyphs = Vec::new();
    for (line_index, line) in layout.lines().enumerate() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let section_index = glyph_run.style().brush.section_index;
            let font_smoothing = glyph_run.style().brush.font_smoothing;
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size();
            let coords = run.normalized_coords();
            let glyph_stroke = GlyphStroke {
                legacy_weight: fills.legacy_glyph_weight_fonts.contains(&font.data.id()),
                slant: fills
                    .font_slants
                    .get(&font.data.id())
                    .copied()
                    .unwrap_or(0.0),
                ..stroke.at(font_size, scale_factor)
            };
            let key = StrokeAtlasKey {
                font: FontAtlasKey {
                    id: font.data.id() as u32,
                    index: font.index,
                    font_size_bits: font_size.to_bits(),
                    variations_hash: FixedHasher.hash_one(coords),
                    hinting,
                    font_smoothing,
                },
                width_bits: glyph_stroke.width.to_bits(),
                embolden_bits: glyph_stroke.embolden.to_bits(),
            };
            let Some(font_ref) = FontRef::from_index(font.data.as_ref(), font.index as usize)
            else {
                return Err(TextError::NoSuchFont);
            };
            let mut maybe_scaler = None;
            for glyph in glyph_run.positioned_glyphs() {
                let Ok(glyph_id) = u16::try_from(glyph.id) else {
                    continue;
                };
                let atlases = atlas_set.entry(key).or_default();
                let atlas_info = match get_glyph_atlas_info(atlases, GlyphCacheKey { glyph_id }) {
                    Some(info) => info,
                    None => {
                        if maybe_scaler.is_none() {
                            let hint = hinting.is_enabled()
                                && font_smoothing == FontSmoothing::AntiAliased;
                            maybe_scaler = Some(
                                scale_cx
                                    .0
                                    .builder_with_id(font_ref, [font.data.id(), font.index.into()])
                                    .size(font_size)
                                    .hint(hint)
                                    .normalized_coords(coords)
                                    .build(),
                            );
                        }
                        add_stroke_glyph_to_atlas(
                            atlases,
                            textures,
                            maybe_scaler.as_mut().unwrap(),
                            font_smoothing,
                            glyph_id,
                            glyph_stroke,
                        )?
                    }
                };
                let glyph_pos = Vec2::new(glyph.x, glyph.y);
                glyphs.push(PositionedGlyph {
                    position: atlas_info.rect.size() / 2.
                        + if font_smoothing == FontSmoothing::None {
                            glyph_pos.floor()
                        } else {
                            glyph_pos
                        }
                        + atlas_info.offset,
                    atlas_info,
                    section_index,
                    line_index: line_index as u32,
                });
            }
        }
    }
    Ok(glyphs)
}

/// Swaps the glyphs of every [`StrokeGlyph`] text's [`TextLayoutInfo`] for their stroked or
/// emboldened rasters whenever its layout or its stroke changes.
pub fn update_stroke_glyphs(
    mut texts: Query<(
        Ref<StrokeGlyph>,
        &ComputedTextBlock,
        Option<&FontHinting>,
        Mut<TextLayoutInfo>,
    )>,
    fills: Res<FontAtlasSet>,
    mut atlas_set: ResMut<StrokeAtlasSet>,
    // Absent in an app without images, which lays nothing out either.
    textures: Option<ResMut<Assets<Image>>>,
    mut scale_cx: ResMut<ScaleCx>,
) {
    let Some(mut textures) = textures else {
        return;
    };
    for (stroke, computed, hinting, mut info) in &mut texts {
        // A system does not see its own writes, so the swap below does not trigger it again.
        if !stroke.is_changed() && !info.is_changed() {
            continue;
        }
        match stroke_glyph_layout(
            computed,
            *stroke,
            hinting.copied().unwrap_or_default(),
            &fills,
            &mut atlas_set,
            &mut textures,
            &mut scale_cx,
        ) {
            Ok(glyphs) => info.glyphs = glyphs,
            Err(error) => bevy_log::warn_once!("stroke glyphs: {error}"),
        }
    }
}

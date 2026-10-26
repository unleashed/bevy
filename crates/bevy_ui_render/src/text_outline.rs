//! Glyph outlines on UI text: [`TextOutline`] draws a stroke of the text's glyphs under its fill.

use bevy_asset::Assets;
use bevy_camera::visibility::InheritedVisibility;
use bevy_color::{Color, LinearRgba};
use bevy_ecs::prelude::*;
use bevy_image::Image;
use bevy_math::Affine2;
use bevy_reflect::{std_traits::ReflectDefault, Reflect};
use bevy_render::Extract;
use bevy_text::{
    stroke_glyph_layout, ComputedTextBlock, EditableText, FontAtlasSet, FontHinting,
    PositionedGlyph, ScaleCx, StrokeAtlasSet, StrokeGlyph, TextLayoutInfo,
};
use bevy_ui::{
    CalculatedClip, ComputedNode, ComputedStackIndex, ComputedUiTargetCamera, UiGlobalTransform,
};
use std::mem;

use crate::{
    stack_z_offsets, text::calculate_text_scroll_clip, ExtractedGlyph, ExtractedUiItem,
    ExtractedUiNode, ExtractedUiNodes, UiCameraMap,
};

/// An outline around the glyphs of a UI `Text`: the glyphs grown by half of `width_px` (text
/// pixels; a stroke of that width centred on each glyph's outline, its inside filled) in `color`,
/// drawn after the text's shadow and before its fill.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
#[require(TextOutlineGlyphs)]
pub struct TextOutline {
    /// Width of the stroke in text pixels.
    pub width_px: f32,
    /// Colour of the stroke.
    pub color: Color,
}

impl Default for TextOutline {
    fn default() -> Self {
        Self {
            width_px: 1.0,
            color: Color::BLACK,
        }
    }
}

/// The stroked glyphs of a [`TextOutline`], laid out on the text's own pen positions, and the
/// width they were rasterised at.
#[derive(Component, Clone, Debug, Default)]
pub struct TextOutlineGlyphs {
    width_px: f32,
    /// One per glyph of the text's [`TextLayoutInfo`].
    pub glyphs: Vec<PositionedGlyph>,
}

/// Lays out the stroked glyphs of every [`TextOutline`] whose text or width changed, around the
/// text's emboldened outline when it also carries a [`StrokeGlyph`] (whose fill swap runs first).
pub fn update_text_outline_glyphs(
    mut texts: Query<(
        &TextOutline,
        Ref<TextLayoutInfo>,
        &ComputedTextBlock,
        Option<&FontHinting>,
        Option<&StrokeGlyph>,
        &mut TextOutlineGlyphs,
    )>,
    fills: Res<FontAtlasSet>,
    mut atlas_set: ResMut<StrokeAtlasSet>,
    mut textures: ResMut<Assets<Image>>,
    mut scale_cx: ResMut<ScaleCx>,
) {
    for (outline, info, computed, hinting, fill, mut glyphs) in &mut texts {
        if !info.is_changed() && glyphs.width_px == outline.width_px {
            continue;
        }
        let stroke = StrokeGlyph {
            width_px: outline.width_px,
            embolden: fill.map_or(0.0, |fill| fill.embolden),
        };
        match stroke_glyph_layout(
            computed,
            stroke,
            hinting.copied().unwrap_or_default(),
            &fills,
            &mut atlas_set,
            &mut textures,
            &mut scale_cx,
        ) {
            Ok(stroked) => {
                glyphs.width_px = outline.width_px;
                glyphs.glyphs = stroked;
            }
            Err(error) => tracing::warn!("text outline glyphs: {error}"),
        }
    }
}

/// Extracts the outline glyphs of changed [`TextOutline`] nodes at the text's depth, after its
/// shadow and before its fill (the UI phase sorts stably, so equal depths keep this order).
pub fn extract_text_outlines(
    mut commands: Commands,
    extracted_uinodes: ResMut<ExtractedUiNodes>,
    uinode_query: Extract<
        Query<(
            Entity,
            &ComputedNode,
            &ComputedStackIndex,
            &UiGlobalTransform,
            &ComputedUiTargetCamera,
            &InheritedVisibility,
            Option<&CalculatedClip>,
            &TextOutline,
            &TextOutlineGlyphs,
            Option<&EditableText>,
        )>,
    >,
    camera_map: Extract<UiCameraMap>,
) {
    let extracted_uinodes = extracted_uinodes.into_inner();
    let mut camera_mapper = camera_map.get_mapper();
    let mut glyphs = vec![];
    for (
        entity,
        uinode,
        stack_index,
        global_transform,
        target,
        inherited_visibility,
        maybe_clip,
        outline,
        outline_glyphs,
        editable_text,
    ) in extracted_uinodes
        .changed
        .iter()
        .flat_map(|main_entity| uinode_query.get(main_entity.entity()).ok())
    {
        if !inherited_visibility.get() || uinode.is_empty() {
            continue;
        }
        let Some(extracted_camera_entity) = camera_mapper.map(target) else {
            continue;
        };
        let transform = Affine2::from(*global_transform)
            * Affine2::from_translation(
                uinode.content_box().min
                    - editable_text.map_or(bevy_math::Vec2::ZERO, |text| text.viewport.offset),
            );
        let clip = calculate_text_scroll_clip(editable_text, maybe_clip, uinode, global_transform);
        let color = LinearRgba::from(outline.color);
        let all = &outline_glyphs.glyphs;
        for (i, glyph) in all.iter().enumerate() {
            glyphs.push(ExtractedGlyph {
                color,
                translation: glyph.position,
                rect: glyph.atlas_info.rect,
            });
            if all
                .get(i + 1)
                .is_none_or(|next| next.atlas_info.texture != glyph.atlas_info.texture)
            {
                extracted_uinodes
                    .uinodes
                    .entry(entity.into())
                    .or_insert_with(|| (extracted_camera_entity, Default::default()))
                    .1
                    .insert(
                        commands.spawn_empty().id(),
                        ExtractedUiNode {
                            z_order: stack_index.0 as f32 + stack_z_offsets::TEXT,
                            image: glyph.atlas_info.texture,
                            clip: clip.clone(),
                            item: ExtractedUiItem::Glyphs {
                                glyphs: mem::take(&mut glyphs),
                            },
                            transform,
                        },
                    );
            }
        }
    }
}

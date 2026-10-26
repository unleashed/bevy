use bevy_asset::{Assets, Handle, RenderAssetUsages};
use bevy_image::{prelude::*, ImageSampler, ToExtents};
use bevy_math::{UVec2, Vec2};
use bevy_platform::collections::HashMap;
use swash::scale::Scaler;
use wgpu_types::{Extent3d, TextureDimension, TextureFormat};

use crate::{FontSmoothing, GlyphAtlasInfo, GlyphAtlasLocation, TextError};

/// Padding in pixels between glyph textures and the font atlas edges.
const GLYPH_ATLAS_PADDING: u32 = 2;

/// Key identifying a glyph
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey {
    /// Id used to look up the glyph
    pub glyph_id: u16,
}

/// Rasterized glyphs are cached, stored in, and retrieved from, a `FontAtlas`.
///
/// A `FontAtlas` contains one or more textures, each of which contains one or more glyphs packed into them.
///
/// A [`FontAtlasSet`](crate::FontAtlasSet) contains a `FontAtlas` for each font size in the same font face.
///
/// For the same font face and font size, a glyph will be rasterized differently for different subpixel offsets.
/// In practice, ranges of subpixel offsets are grouped into subpixel bins to limit the number of rasterized glyphs,
/// providing a trade-off between visual quality and performance.
///
/// A [`GlyphCacheKey`] encodes all of the information of a subpixel-offset glyph and is used to
/// find that glyphs raster in a [`TextureAtlas`] through its corresponding [`GlyphAtlasLocation`].
pub struct FontAtlas {
    /// Used to update the [`TextureAtlasLayout`].
    pub dynamic_texture_atlas_builder: DynamicTextureAtlasBuilder,
    /// A mapping between subpixel-offset glyphs and their [`GlyphAtlasLocation`].
    pub glyph_to_atlas_index: HashMap<GlyphCacheKey, GlyphAtlasLocation>,
    /// The layout for the font atlas.
    pub texture_atlas: TextureAtlasLayout,
    /// The texture where this font atlas is located
    pub texture: Handle<Image>,
}

impl FontAtlas {
    /// Create a new [`FontAtlas`] with the given size, adding it to the appropriate asset collections.
    pub fn new(
        textures: &mut Assets<Image>,
        size: UVec2,
        font_smoothing: FontSmoothing,
    ) -> FontAtlas {
        let mut image = Image::new_fill(
            size.to_extents(),
            TextureDimension::D2,
            &[0, 0, 0, 0],
            TextureFormat::Rgba8UnormSrgb,
            // Need to keep this image CPU persistent in order to add additional glyphs later on
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        if font_smoothing == FontSmoothing::None {
            image.sampler = ImageSampler::nearest();
        }
        let texture = textures.add(image);
        Self {
            texture_atlas: TextureAtlasLayout::new_empty(size),
            glyph_to_atlas_index: HashMap::default(),
            dynamic_texture_atlas_builder: DynamicTextureAtlasBuilder::new(
                size,
                GLYPH_ATLAS_PADDING,
            ),
            texture,
        }
    }

    /// Get the [`GlyphAtlasLocation`] for a subpixel-offset glyph.
    pub fn get_glyph_index(&self, cache_key: GlyphCacheKey) -> Option<GlyphAtlasLocation> {
        self.glyph_to_atlas_index.get(&cache_key).copied()
    }

    /// Checks if the given subpixel-offset glyph is contained in this [`FontAtlas`].
    pub fn has_glyph(&self, cache_key: GlyphCacheKey) -> bool {
        self.glyph_to_atlas_index.contains_key(&cache_key)
    }

    /// Add a glyph to the atlas, updating both its texture and layout.
    ///
    /// The glyph is represented by `glyph`, and its image content is `glyph_texture`.
    /// This content is copied into the atlas texture, and the atlas layout is updated
    /// to store the location of that glyph into the atlas.
    ///
    /// # Returns
    ///
    /// Returns `()` if the glyph is successfully added, or [`TextError::FailedToAddGlyph`] otherwise.
    /// In that case, neither the atlas texture nor the atlas layout are
    /// modified.
    pub fn add_glyph(
        &mut self,
        textures: &mut Assets<Image>,
        key: GlyphCacheKey,
        texture: &Image,
        offset: Vec2,
        is_alpha_mask: bool,
    ) -> Result<(), TextError> {
        let mut atlas_texture = textures
            .get_mut(&self.texture)
            .ok_or(TextError::MissingAtlasTexture)?;

        if let Ok(glyph_index) = self.dynamic_texture_atlas_builder.add_texture(
            &mut self.texture_atlas,
            texture,
            &mut atlas_texture,
        ) {
            self.glyph_to_atlas_index.insert(
                key,
                GlyphAtlasLocation {
                    glyph_index,
                    offset,
                    is_alpha_mask,
                },
            );
            Ok(())
        } else {
            Err(TextError::FailedToAddGlyph(key.glyph_id))
        }
    }
}

impl core::fmt::Debug for FontAtlas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FontAtlas")
            .field("glyph_to_atlas_index", &self.glyph_to_atlas_index)
            .field("texture_atlas", &self.texture_atlas)
            .field("texture", &self.texture)
            .field("dynamic_texture_atlas_builder", &"[...]")
            .finish()
    }
}

/// Adds the given subpixel-offset glyph to the given font atlases, unscaled (outside
/// [`FontAtlasSet::legacy_glyph_weight_fonts`](crate::FontAtlasSet)).
pub fn add_glyph_to_atlas(
    font_atlases: &mut Vec<FontAtlas>,
    textures: &mut Assets<Image>,
    scaler: &mut Scaler,
    font_smoothing: FontSmoothing,
    glyph_id: u16,
) -> Result<GlyphAtlasInfo, TextError> {
    add_stroke_glyph_to_atlas(
        font_atlases,
        textures,
        scaler,
        font_smoothing,
        glyph_id,
        GlyphStroke::default(),
    )
}

/// How a glyph's outline is changed before it is rasterised, in physical pixels of the raster.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlyphStroke {
    /// Width of a stroke centred on the outline and unioned with the fill, so the raster is the
    /// glyph grown by half of it; 0 is the fill alone.
    pub width: f32,
    /// Growth of each stem in total, `FreeType`'s `FT_Outline_Embolden` strength, applied to the
    /// outline before the stroke or the fill (and before the `legacy_glyph_weight` scale).
    pub embolden: f32,
    /// Whether the `legacy_glyph_weight` feature scales the outline by 64/60 (the font is in
    /// [`FontAtlasSet::legacy_glyph_weight_fonts`](crate::FontAtlasSet)).
    pub legacy_weight: bool,
    /// The outline's shear `x + slant * y` (y up), applied first
    /// ([`FontAtlasSet::font_slants`](crate::FontAtlasSet)).
    pub slant: f32,
}

/// Adds the given glyph, stroked or emboldened as `stroke` says, to the given font atlases.
///
/// The atlases are keyed by glyph id alone, so they must hold rasters of this one `stroke`
/// (see [`StrokeAtlasSet`](crate::StrokeAtlasSet)).
pub fn add_stroke_glyph_to_atlas(
    font_atlases: &mut Vec<FontAtlas>,
    textures: &mut Assets<Image>,
    scaler: &mut Scaler,
    font_smoothing: FontSmoothing,
    glyph_id: u16,
    stroke: GlyphStroke,
) -> Result<GlyphAtlasInfo, TextError> {
    let (glyph_texture, offset, is_alpha_mask) =
        get_stroke_glyph_texture(scaler, glyph_id, font_smoothing, stroke)?;
    let mut add_char_to_font_atlas = |atlas: &mut FontAtlas| -> Result<(), TextError> {
        atlas.add_glyph(
            textures,
            GlyphCacheKey { glyph_id },
            &glyph_texture,
            offset,
            is_alpha_mask,
        )
    };
    if !font_atlases
        .iter_mut()
        .any(|atlas| add_char_to_font_atlas(atlas).is_ok())
    {
        // Find the largest dimension of the glyph, either its width or its height
        let glyph_max_size: u32 = glyph_texture
            .texture_descriptor
            .size
            .height
            .max(glyph_texture.width());
        // Returns the smallest power-of-two atlas size that fits the glyph and its
        // required padding, with a minimum size of 512 pixels.
        let containing = glyph_max_size
            .saturating_add(GLYPH_ATLAS_PADDING * 2)
            .next_power_of_two()
            .max(512);

        let mut new_atlas = FontAtlas::new(textures, UVec2::splat(containing), font_smoothing);

        new_atlas.add_glyph(
            textures,
            GlyphCacheKey { glyph_id },
            &glyph_texture,
            offset,
            is_alpha_mask,
        )?;

        font_atlases.push(new_atlas);
    }

    get_glyph_atlas_info(font_atlases, GlyphCacheKey { glyph_id })
        .ok_or(TextError::InconsistentAtlasState)
}

/// Get the texture of the glyph as a rendered image, and its offset
pub fn get_outlined_glyph_texture(
    scaler: &mut Scaler,
    glyph_id: u16,
    font_smoothing: FontSmoothing,
) -> Result<(Image, Vec2, bool), TextError> {
    get_stroke_glyph_texture(scaler, glyph_id, font_smoothing, GlyphStroke::default())
}

/// Get the texture of the glyph, stroked or emboldened as `stroke` says, and its offset
#[expect(
    clippy::identity_op,
    reason = "Alignment improves clarity during RGBA operations."
)]
pub fn get_stroke_glyph_texture(
    scaler: &mut Scaler,
    glyph_id: u16,
    font_smoothing: FontSmoothing,
    stroke: GlyphStroke,
) -> Result<(Image, Vec2, bool), TextError> {
    let mut render = swash::scale::Render::new(&[
        swash::scale::Source::ColorOutline(0),
        swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
        swash::scale::Source::Outline,
    ]);
    render.format(swash::zeno::Format::Alpha);
    // Godot shears the outline (`FT_Outline_Transform`, `text_server_adv.cpp:1363-1366` at
    // 4.7.2-stable) before any other change of it.
    let slant = swash::zeno::Transform::new(1.0, 0.0, stroke.slant, 1.0, 0.0, 0.0);
    if stroke.slant != 0.0 {
        render.transform(Some(slant));
    }
    // Godot 4.7.2's `ft_point2` divides 26.6 outline coordinates by 60.0f (fixed in 4.8 behind `gui/fonts/compatibility/msdf_legacy_scaling`).
    #[cfg(feature = "legacy_glyph_weight")]
    if stroke.legacy_weight {
        render.transform(Some(
            slant.then(&swash::zeno::Transform::scale(64.0 / 60.0, 64.0 / 60.0)),
        ));
    }
    // swash moves each edge by the strength where FreeType's `FT_Outline_EmboldenXY` halves it
    // first (swash `scale/outline.rs` `embolden`), so half of FreeType's strength grows a stem as
    // much. swash emboldens before the transform, as Godot does before `ft_point2`.
    if stroke.embolden != 0.0 {
        render.embolden(stroke.embolden / 2.0);
    }
    let fill = render
        .render(scaler, glyph_id)
        .ok_or(TextError::FailedToGetGlyphImage(glyph_id))?;
    let image = if stroke.width > 0.0 {
        // swash strokes after the transform: the 64/60 scale moves the stroke with the fill and
        // leaves its width. Round joins, as the distance band of Godot's MSDF outline has.
        render.style(swash::zeno::Stroke {
            width: stroke.width,
            join: swash::zeno::Join::Round,
            ..swash::zeno::Stroke::default()
        });
        let mut band = render
            .render(scaler, glyph_id)
            .ok_or(TextError::FailedToGetGlyphImage(glyph_id))?;
        grow_by_fill(&mut band, &fill);
        band
    } else {
        fill
    };

    let left = image.placement.left;
    let top = image.placement.top;
    let width = image.placement.width;
    let height = image.placement.height;

    let px = (width * height) as usize;
    let rgba = match image.content {
        swash::scale::image::Content::Mask => {
            let mut rgba = vec![0u8; px * 4];
            match font_smoothing {
                FontSmoothing::AntiAliased => {
                    for i in 0..px {
                        let a = image.data[i];
                        rgba[i * 4 + 0] = 255; // R
                        rgba[i * 4 + 1] = 255; // G
                        rgba[i * 4 + 2] = 255; // B
                        rgba[i * 4 + 3] = a; // A from swash
                    }
                }
                FontSmoothing::None => {
                    for i in 0..px {
                        let a = image.data[i];
                        rgba[i * 4 + 0] = 255; // R
                        rgba[i * 4 + 1] = 255; // G
                        rgba[i * 4 + 2] = 255; // B
                        rgba[i * 4 + 3] = if 127 < a { 255 } else { 0 }; // A from swash
                    }
                }
            }
            rgba
        }
        swash::scale::image::Content::Color | swash::scale::image::Content::SubpixelMask => {
            image.data
        }
    };

    Ok((
        Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        ),
        Vec2::new(left as f32, -top as f32),
        image.content == swash::scale::image::Content::Mask,
    ))
}

/// Unions the `fill` mask into the stroke `band` around it (the larger of the two coverages), so
/// the band becomes the glyph grown by half the stroke width with its inside filled, as Godot's
/// MSDF outline draws it (`canvas.glsl`, the distance test `d - 0.5 + cr`): a translucent outline
/// under a translucent fill stacks over the whole glyph, not only along its edges.
fn grow_by_fill(band: &mut swash::scale::image::Image, fill: &swash::scale::image::Image) {
    use swash::scale::image::Content;
    if band.content != Content::Mask || fill.content != Content::Mask {
        return;
    }
    let (b, f) = (band.placement, fill.placement);
    for fy in 0..f.height as i32 {
        // Rows run down from `top`, which is up from the baseline.
        let by = b.top - f.top + fy;
        for fx in 0..f.width as i32 {
            let bx = f.left - b.left + fx;
            if by < 0 || bx < 0 || by >= b.height as i32 || bx >= b.width as i32 {
                continue;
            }
            let coverage = fill.data[(fy * f.width as i32 + fx) as usize];
            let out = &mut band.data[(by * b.width as i32 + bx) as usize];
            *out = (*out).max(coverage);
        }
    }
}

/// Generates the [`GlyphAtlasInfo`] for the given subpixel-offset glyph.
pub fn get_glyph_atlas_info(
    font_atlases: &mut [FontAtlas],
    cache_key: GlyphCacheKey,
) -> Option<GlyphAtlasInfo> {
    font_atlases.iter().find_map(|atlas| {
        atlas
            .get_glyph_index(cache_key)
            .map(|location| GlyphAtlasInfo {
                offset: location.offset,
                rect: atlas.texture_atlas.textures[location.glyph_index].as_rect(),
                texture: atlas.texture.id(),
                is_alpha_mask: location.is_alpha_mask,
            })
    })
}

#[cfg(all(test, feature = "legacy_glyph_weight"))]
mod legacy_glyph_weight_tests {
    use super::*;
    use swash::{scale::ScaleContext, FontRef};

    // Only a font listed in `FontAtlasSet::legacy_glyph_weight_fonts` draws at 64/60.
    #[test]
    fn legacy_weight_scales_only_the_flagged_raster() {
        let font =
            FontRef::from_index(include_bytes!("FiraMono-subset.ttf"), 0).expect("valid test font");
        let glyph_id = font.charmap().map('M');
        let mut scale_context = ScaleContext::new();
        let mut height = |legacy_weight| {
            let mut scaler = scale_context.builder(font).size(120.0).build();
            let stroke = GlyphStroke {
                legacy_weight,
                ..GlyphStroke::default()
            };
            let (image, _, _) =
                get_stroke_glyph_texture(&mut scaler, glyph_id, FontSmoothing::AntiAliased, stroke)
                    .expect("glyph should rasterize");
            image.height() as f32
        };
        let (plain, legacy) = (height(false), height(true));
        assert!(
            (legacy / plain - 64.0 / 60.0).abs() < 0.03,
            "{plain} px plain, {legacy} px legacy"
        );
    }
}

#[cfg(test)]
mod slant_tests {
    use super::*;
    use swash::{scale::ScaleContext, FontRef};

    // A slanted raster leans right going up: x + slant * y widens the glyph by slant times its
    // height and moves its top half right of its bottom half (an "I", whose serifs match).
    #[test]
    fn slant_shears_the_raster_right_going_up() {
        let font =
            FontRef::from_index(include_bytes!("FiraMono-subset.ttf"), 0).expect("valid test font");
        let glyph_id = font.charmap().map('I');
        let mut scale_context = ScaleContext::new();
        let mut raster = |slant| {
            let mut scaler = scale_context.builder(font).size(120.0).build();
            let stroke = GlyphStroke {
                slant,
                ..GlyphStroke::default()
            };
            let (image, _, _) =
                get_stroke_glyph_texture(&mut scaler, glyph_id, FontSmoothing::AntiAliased, stroke)
                    .expect("glyph should rasterize");
            let (w, h) = (image.width() as usize, image.height() as usize);
            let data = image.data.expect("raster data");
            // Ink-weighted mean x of the rows in `rows`.
            let centroid = |rows: core::ops::Range<usize>| {
                let (mut sum, mut ink) = (0.0, 0.0);
                for y in rows {
                    for x in 0..w {
                        let a = f32::from(data[(y * w + x) * 4 + 3]);
                        sum += a * x as f32;
                        ink += a;
                    }
                }
                sum / ink
            };
            (w as f32, h as f32, centroid(0..h / 2) - centroid(h / 2..h))
        };
        let (upright, height, upright_lean) = raster(0.0);
        let (slanted, _, slanted_lean) = raster(0.2);
        assert!(
            (slanted - upright - 0.2 * height).abs() < 2.0,
            "{upright} to {slanted} px wide, {height} tall"
        );
        assert!(
            slanted_lean - upright_lean > 0.2 * height / 4.0,
            "{upright_lean} to {slanted_lean}"
        );
    }
}

#[cfg(test)]
mod allocation_regression_tests {
    use super::*;
    use swash::{scale::ScaleContext, FontRef};

    // Regression test for https://github.com/bevyengine/bevy/issues/25224.
    // Atlas sizing must account for the padding used when constructing the atlas.
    #[test]
    fn new_atlas_fits_boundary_sized_glyph_with_padding() {
        let font =
            FontRef::from_index(include_bytes!("FiraMono-subset.ttf"), 0).expect("valid test font");
        let glyph_id = font.charmap().map('M');
        let mut scale_context = ScaleContext::new();
        let mut measurement_scaler = scale_context.builder(font).size(1479.0).build();
        let (glyph_texture, _, _) = get_outlined_glyph_texture(
            &mut measurement_scaler,
            glyph_id,
            FontSmoothing::AntiAliased,
        )
        .expect("glyph should rasterize");
        assert_eq!(
            glyph_texture.width().max(glyph_texture.height()),
            1021,
            "test font no longer reproduces the atlas boundary"
        );

        let mut scaler = scale_context.builder(font).size(1479.0).build();
        let mut font_atlases = Vec::new();
        let mut textures = Assets::default();

        add_glyph_to_atlas(
            &mut font_atlases,
            &mut textures,
            &mut scaler,
            FontSmoothing::AntiAliased,
            glyph_id,
        )
        .expect("a newly created atlas should fit the glyph that requested it");
    }
}

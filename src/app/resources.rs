use super::*;

pub(crate) fn load_equipment_textures(
    ctx: &egui::Context,
    root: &std::path::Path,
    catalog: &EquipmentCatalog,
) -> HashMap<String, egui::TextureHandle> {
    let mut resource_paths = catalog
        .items
        .values()
        .map(|item| item.icon.clone())
        .collect::<Vec<_>>();
    resource_paths.sort();
    resource_paths.dedup();
    resource_paths
        .into_iter()
        .filter_map(|resource_path| {
            load_image_texture(ctx, root, &resource_path, "equipment")
                .map(|texture| (resource_path, texture))
        })
        .collect()
}

pub(crate) fn load_attribute_icons(
    ctx: &egui::Context,
    root: &std::path::Path,
) -> HashMap<String, egui::TextureHandle> {
    ATTRIBUTE_ICON_PATHS
        .into_iter()
        .filter_map(|(attribute, path)| {
            load_image_texture(ctx, root, path, "attribute-icon")
                .map(|texture| (attribute.to_owned(), texture))
        })
        .collect()
}

pub(crate) fn load_monster_textures(
    ctx: &egui::Context,
    root: &std::path::Path,
    monster_ids: &[String],
) -> HashMap<String, egui::TextureHandle> {
    let mut textures = HashMap::new();
    for monster_id in monster_ids.iter().map(String::as_str) {
        for stem in monster_image_stem_candidates(monster_id) {
            let resource_keys = monster_image_resource_keys(&stem);
            if resource_keys.iter().any(|key| textures.contains_key(key)) {
                break;
            }
            let resource_path = format!("{MONSTER_IMAGE_DIR}/{stem}.png");
            let Some(texture) = load_image_texture(ctx, root, &resource_path, "monster") else {
                continue;
            };
            for key in resource_keys {
                textures.entry(key).or_insert_with(|| texture.clone());
            }
            break;
        }
    }

    textures
}

pub(crate) fn load_damage_digit_textures(
    ctx: &egui::Context,
    root: &std::path::Path,
) -> HashMap<String, Vec<egui::TextureHandle>> {
    let mut textures = HashMap::new();
    for (key, prefix) in DAMAGE_DIGIT_TEXTURE_SETS {
        let digits = (0..=9)
            .filter_map(|digit| {
                let path = damage_digit_resource_path(prefix, digit);
                load_image_texture(ctx, root, &path, "damage-digit")
            })
            .collect::<Vec<_>>();
        if digits.len() == 10 {
            textures.insert(key.to_owned(), digits);
        }
    }
    textures
}

pub(crate) fn load_reaction_text_textures(
    ctx: &egui::Context,
    root: &std::path::Path,
) -> HashMap<u8, Vec<egui::TextureHandle>> {
    let mut textures = HashMap::new();
    let language = i18n::current_language();
    for reaction in 1..=REACTION_TEXT_IMAGE_COUNT {
        let glyphs = (1..=2)
            .filter_map(|part| {
                for path in reaction_text_resource_path_candidates(language, reaction, part) {
                    match load_reaction_text_texture(ctx, root, &path) {
                        ReactionTextTextureLoad::Loaded(texture) => return Some(texture),
                        ReactionTextTextureLoad::Blank => return None,
                        ReactionTextTextureLoad::Missing => {}
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        if !glyphs.is_empty() {
            textures.insert(reaction, glyphs);
        }
    }
    textures
}

pub(crate) fn reaction_text_resource_path(language: Language, reaction: u8, part: u8) -> String {
    format!(
        "{DAMAGE_DIGIT_IMAGE_DIR}/{}/fanying{reaction:02}_{part:02}.png",
        language.reaction_text_folder()
    )
}

fn reaction_text_resource_path_candidates(
    language: Language,
    reaction: u8,
    part: u8,
) -> Vec<String> {
    let mut paths = vec![reaction_text_resource_path(language, reaction, part)];
    if language != Language::SimplifiedChinese {
        paths.push(reaction_text_resource_path(
            Language::SimplifiedChinese,
            reaction,
            part,
        ));
    }
    paths.push(format!(
        "{DAMAGE_DIGIT_IMAGE_DIR}/fanying{reaction:02}_{part:02}.png"
    ));
    paths
}

enum ReactionTextTextureLoad {
    Loaded(egui::TextureHandle),
    Blank,
    Missing,
}

fn load_reaction_text_texture(
    ctx: &egui::Context,
    root: &std::path::Path,
    resource_path: &str,
) -> ReactionTextTextureLoad {
    let path = root.join(resource_path);
    let Ok(bytes) = std::fs::read(&path)
        .map(std::borrow::Cow::Owned)
        .or_else(|_| read_resource_bytes(Path::new(resource_path)))
    else {
        return ReactionTextTextureLoad::Missing;
    };
    let Ok(image) = image::load_from_memory(bytes.as_ref()).map(|image| image.to_rgba8()) else {
        return ReactionTextTextureLoad::Missing;
    };
    if rgba_image_is_blank(&image) {
        return ReactionTextTextureLoad::Blank;
    }
    // draw_reaction_text_images() sizes each glyph purely from its texture's
    // aspect ratio, scaled to a fixed height. The `en` art sits on a wider,
    // shorter canvas with much more transparent margin than `zh`'s (e.g. a
    // 144x58 canvas ~55% filled vs. 120x116 ~70% filled), so at the same
    // render height the actual English glyph pixels come out visibly
    // smaller. Cropping to the opaque bounding box first removes each
    // locale's baked-in padding from the aspect-ratio math, so glyph height
    // ends up consistent across languages instead of matching canvas size.
    let image = crop_to_opaque_bounds(image);
    ReactionTextTextureLoad::Loaded(load_rgba_texture(
        ctx,
        resource_path,
        "reaction-text",
        image,
    ))
}

fn crop_to_opaque_bounds(image: image::RgbaImage) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if min_x > max_x || min_y > max_y {
        return image;
    }
    image::imageops::crop_imm(&image, min_x, min_y, max_x - min_x + 1, max_y - min_y + 1).to_image()
}

pub(crate) fn damage_digit_resource_path(prefix: &str, digit: usize) -> String {
    format!("{DAMAGE_DIGIT_IMAGE_DIR}/{prefix}_{digit}.png")
}

pub(crate) fn load_character_avatars(
    ctx: &egui::Context,
    root: &std::path::Path,
    characters: &HashMap<u32, CharacterInfo>,
) -> HashMap<String, egui::TextureHandle> {
    let mut textures = HashMap::new();
    for avatar in characters
        .values()
        .filter_map(|character| character.avatar.as_deref())
    {
        if textures.contains_key(avatar) {
            continue;
        }
        if let Some(texture) = load_image_texture(ctx, root, avatar, "character-avatar") {
            textures.insert(avatar.to_owned(), texture);
        }
    }
    textures
}

pub(crate) fn fill_missing_character_colors_from_avatars(
    characters: &mut HashMap<u32, CharacterInfo>,
    root: &std::path::Path,
) {
    let mut avatar_colors = HashMap::<String, Color32>::new();
    for character in characters.values_mut() {
        if character
            .color
            .as_deref()
            .and_then(parse_hex_color)
            .is_some()
        {
            continue;
        }
        let Some(avatar) = character.avatar.as_deref() else {
            continue;
        };
        let color = avatar_colors
            .entry(avatar.to_owned())
            .or_insert_with(|| {
                avatar_accent_color(root, avatar).unwrap_or_else(|| {
                    deterministic_character_fallback_color(avatar.as_bytes(), false)
                })
            })
            .to_owned();
        character.color = Some(format!(
            "#{:02X}{:02X}{:02X}",
            color.r(),
            color.g(),
            color.b()
        ));
    }
}

pub(crate) fn avatar_accent_color(root: &std::path::Path, resource_path: &str) -> Option<Color32> {
    let path = root.join(resource_path);
    let bytes = std::fs::read(&path)
        .map(std::borrow::Cow::Owned)
        .or_else(|_| read_resource_bytes(Path::new(resource_path)))
        .ok()?;
    let image = image::load_from_memory(bytes.as_ref()).ok()?.to_rgba8();
    let mut red = 0.0_f64;
    let mut green = 0.0_f64;
    let mut blue = 0.0_f64;
    let mut total_weight = 0.0_f64;
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue;
        }
        let rf = f64::from(r) / 255.0;
        let gf = f64::from(g) / 255.0;
        let bf = f64::from(b) / 255.0;
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        let saturation = if max <= f64::EPSILON {
            0.0
        } else {
            (max - min) / max
        };
        if !(0.16..=0.96).contains(&max) || saturation < 0.16 {
            continue;
        }
        let mid_luma_weight = 1.0 - ((max - 0.58).abs() / 0.58).clamp(0.0, 0.85);
        let weight = saturation.powf(1.35) * mid_luma_weight.max(0.25) * f64::from(a) / 255.0;
        red += rf * weight;
        green += gf * weight;
        blue += bf * weight;
        total_weight += weight;
    }
    if total_weight <= f64::EPSILON {
        return None;
    }
    let mut r = red / total_weight;
    let mut g = green / total_weight;
    let mut b = blue / total_weight;
    let max = r.max(g).max(b).max(0.001);
    let min = r.min(g).min(b);
    let saturation = (max - min) / max;
    if saturation < 0.24 {
        let mean = (r + g + b) / 3.0;
        r = mean + (r - mean) * 1.45;
        g = mean + (g - mean) * 1.45;
        b = mean + (b - mean) * 1.45;
    }
    let max = r.max(g).max(b).max(0.001);
    if max < 0.46 {
        let scale = 0.46 / max;
        r *= scale;
        g *= scale;
        b *= scale;
    }
    Some(Color32::from_rgb(
        (r.clamp(0.0, 0.92) * 255.0).round() as u8,
        (g.clamp(0.0, 0.92) * 255.0).round() as u8,
        (b.clamp(0.0, 0.92) * 255.0).round() as u8,
    ))
}

pub(crate) fn deterministic_character_fallback_color(seed: &[u8], dark_mode: bool) -> Color32 {
    let hash = seed.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    let palette = theme_tokens(dark_mode, AccentColor::Zinc).dataviz;
    palette[hash as usize % palette.len()]
}

pub(crate) fn load_image_texture(
    ctx: &egui::Context,
    root: &std::path::Path,
    resource_path: &str,
    texture_namespace: &str,
) -> Option<egui::TextureHandle> {
    let path = root.join(resource_path);
    let bytes = std::fs::read(&path)
        .map(std::borrow::Cow::Owned)
        .or_else(|_| read_resource_bytes(Path::new(resource_path)))
        .ok()?;
    let image = image::load_from_memory(bytes.as_ref()).ok()?.to_rgba8();
    Some(load_rgba_texture(
        ctx,
        resource_path,
        texture_namespace,
        image,
    ))
}

fn rgba_image_is_blank(image: &image::RgbaImage) -> bool {
    image.as_raw().chunks_exact(4).all(|pixel| pixel[3] == 0)
}

fn load_rgba_texture(
    ctx: &egui::Context,
    resource_path: &str,
    texture_namespace: &str,
    image: image::RgbaImage,
) -> egui::TextureHandle {
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    // Source art (e.g. 256px avatars) is drawn much smaller (~32px), an 8x
    // minification. Plain bilinear sampling has no mip chain, so it only reads a
    // 2x2 texel neighborhood and aliases hard edges into the jagged look seen on
    // screen. Trilinear filtering with generated mipmaps samples a pre-averaged
    // level, keeping shrunken images crisp and smooth. NOTE: only egui_glow honors this
    // (it calls `generate_mipmap`); egui-wgpu 0.34 uploads with `mip_level_count: 1` and
    // ignores `mipmap_mode`, so under the wgpu backend heavy minification falls back to
    // plain bilinear and aliases a little more. Pre-downscaling the source art would
    // restore smoothness if that ever looks bad. `mipmap_mode` is kept set so the glow
    // path (or a future wgpu mip chain) still benefits.
    let texture_options = egui::TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    };
    ctx.load_texture(
        format!("{texture_namespace}:{resource_path}"),
        color_image,
        texture_options,
    )
}

pub(crate) fn pixel_aligned_rect(
    origin: egui::Pos2,
    logical_size: f32,
    pixels_per_point: f32,
) -> egui::Rect {
    let pixels_per_point = pixels_per_point.max(1.0);
    let physical_size = (logical_size * pixels_per_point).round();
    let size = physical_size / pixels_per_point;
    let min = egui::pos2(
        (origin.x * pixels_per_point).round() / pixels_per_point,
        (origin.y * pixels_per_point).round() / pixels_per_point,
    );
    egui::Rect::from_min_size(min, egui::vec2(size, size))
}

pub(crate) fn configure_style(
    ctx: &egui::Context,
    dark_mode: bool,
    theme_preset: ThemePreset,
    accent: AccentColor,
    density: UiDensity,
    reduce_motion: bool,
) {
    let tokens = theme_tokens_for_preset(theme_preset, dark_mode, accent);
    let density = density_tokens(density);
    let effective_dark = dark_mode || theme_preset == ThemePreset::Tactical;
    let mut visuals = if effective_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = tokens.bg;
    visuals.window_fill = tokens.bg_elevated;
    visuals.extreme_bg_color = tokens.bg;
    visuals.faint_bg_color = tokens.card;
    visuals.code_bg_color = tokens.card;
    visuals.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    visuals.widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, tokens.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, tokens.fg);
    visuals.widgets.inactive.bg_fill = tokens.card;
    visuals.widgets.inactive.weak_bg_fill = tokens.card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, tokens.border);
    visuals.widgets.inactive.fg_stroke = visuals.widgets.noninteractive.fg_stroke;
    visuals.widgets.hovered.bg_fill = tokens.card_hover;
    visuals.widgets.hovered.weak_bg_fill = tokens.card_hover;
    visuals.widgets.hovered.fg_stroke = visuals.widgets.noninteractive.fg_stroke;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, tokens.border_strong);
    // `active` is the pressed state of ordinary (card-colored) widgets, and
    // egui's `Visuals::strong_text_color()` reads `active.fg_stroke` — so this
    // pair must stay readable on the window background, or every default
    // `.strong()` text and hand-painted "strong" glyph goes invisible.
    // Accent-on-accent belongs in `selection.*` (used by selected buttons).
    visuals.widgets.active.bg_fill = tokens.muted;
    visuals.widgets.active.weak_bg_fill = tokens.muted;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, tokens.border_strong);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, tokens.fg);
    visuals.window_stroke = Stroke::new(1.0_f32, tokens.border);
    visuals.window_shadow = match theme_preset {
        ThemePreset::HighContrast => egui::epaint::Shadow::NONE,
        ThemePreset::Zinc | ThemePreset::Tactical => egui::epaint::Shadow {
            offset: [0, 8],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(if effective_dark { 110 } else { 48 }),
        },
    };
    visuals.popup_shadow = match theme_preset {
        ThemePreset::HighContrast => egui::epaint::Shadow::NONE,
        ThemePreset::Zinc | ThemePreset::Tactical => egui::epaint::Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(if effective_dark { 140 } else { 58 }),
        },
    };
    visuals.selection.bg_fill = tokens.accent;
    visuals.selection.stroke = Stroke::new(1.0_f32, tokens.accent_fg);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.animation_time = motion::duration(reduce_motion, motion::dur::FAST);
    style.interaction.selectable_labels = false;
    style.spacing.item_spacing = density.item_spacing;
    style.spacing.interact_size.y = density.interact_height;
    style.spacing.button_padding = density.button_padding;
    for (text_style, base_size) in [
        (egui::TextStyle::Small, 10.0),
        (egui::TextStyle::Body, 14.0),
        (egui::TextStyle::Button, 14.0),
        (egui::TextStyle::Heading, 20.0),
        (egui::TextStyle::Monospace, 13.0),
    ] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size = base_size * density.font_scale;
        }
    }
    let mut scroll = egui::style::ScrollStyle::solid();
    scroll.bar_width = 8.0;
    scroll.handle_min_length = 32.0;
    scroll.bar_inner_margin = 4.0;
    scroll.bar_outer_margin = 2.0;
    scroll.foreground_color = true;
    style.spacing.scroll = scroll;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.hovered.expansion = 0.0;
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    style.visuals.widgets.active.expansion = 0.0;
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_reaction_texture_is_blank() {
        let transparent = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 255, 255, 0]));
        assert!(rgba_image_is_blank(&transparent));

        let visible = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 1]));
        assert!(!rgba_image_is_blank(&visible));
    }

    #[test]
    fn crop_to_opaque_bounds_trims_transparent_margin() {
        // A 10x10 canvas with a 4x2 opaque glyph off-center, mimicking the
        // padded `en` reaction-text art versus the tightly-cropped `zh` art.
        let mut image = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
        for y in 3..5 {
            for x in 2..6 {
                image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let cropped = crop_to_opaque_bounds(image);
        assert_eq!(cropped.dimensions(), (4, 2));
        assert_eq!(*cropped.get_pixel(0, 0), image::Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn crop_to_opaque_bounds_is_noop_for_fully_opaque_image() {
        let image = image::RgbaImage::from_pixel(5, 3, image::Rgba([1, 2, 3, 255]));
        let cropped = crop_to_opaque_bounds(image.clone());
        assert_eq!(cropped.dimensions(), image.dimensions());
    }
}

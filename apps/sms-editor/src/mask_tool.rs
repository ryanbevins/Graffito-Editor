//! The Mask Tool: authoring washable goop masks on enemy actor models.
//!
//! # The effect being authored
//!
//! Washable goop is a per-pixel comparison the game runs on wired actors:
//!
//! ```text
//! visible = mask(goop UV) > K0_A
//! ```
//!
//! `K0_A` is a scalar the enemy's class drives (hit points, for StayPakkun),
//! sweeping from full coverage to clean as FLUDD washes it. The mask supplies
//! the *shape*: because the comparison is a hard threshold, texels cross it in
//! order of their mask value, so the coating recedes with a crisp edge tracing
//! the mask's gradient. Bright mask paint clings longest; dark clears first.
//! A separate colour texture supplies the goop's look. Both ride one UV set.
//!
//! Retail authored that UV as a **front projection normalised to bounds** --
//! measuring StayPakkun's real goop UV puts it exactly on the unit square, and
//! front and back of the body share it (the coating is symmetric, so sharing is
//! intended). [`front_projection_bounds`] reproduces that. The goop UV stays a
//! front projection however the preview camera is orbited, so what is authored
//! never drifts with the view.
//!
//! # This module
//!
//! - An actor sampler over the loaded stage's own hierarchy.
//! - An orbitable model preview, shaded the way the stage viewport shades: the
//!   geometry's own resolved combine mode, vertex colours and material colour,
//!   with smooth interpolated normals.
//! - A **UV inspector** drawing either UV set over the mask, so a layout can be
//!   read directly -- this is how retail's goop UV was identified as a front
//!   projection in the first place.
//! - Assignable goop **colour** and **mask** sources: generated content, or any
//!   texture the model already carries.
//! - **Play full cycle** sweeps `K0_A` so the wash recedes as it does in game.
//!
//! Painting strokes onto the mask, and writing the authored UV and mask back
//! into a model's material, are the phases after this one.

use super::*;

/// What the viewport shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskView {
    /// The model, orbitable.
    Model,
    /// The selected UV layout, drawn over the mask.
    Uv,
}

/// Which UV layer the preview composites and the inspector draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskUvLayer {
    /// The model's own body UV.
    Body,
    /// The goop layer's front-projected UV.
    Goop,
}

/// Where a goop texture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaskTextureSource {
    /// Generated content: the rainbow colour map, or the borrowed mask.
    Generated,
    /// A texture the model already carries, by index.
    Model(usize),
    /// A retail goop style from the goop tool's catalog -- the same chocolate,
    /// oil, pink and electric surfaces the goop tool paints with.
    GoopStyle(usize),
}

/// One placed enemy the Mask Tool can target.
struct MaskActorChoice {
    object_id: String,
    label: String,
    model_path: String,
    /// The loader flags this actor's model is read with. Actors and map
    /// geometry resolve their materials differently, so reading an actor with
    /// the map defaults drops the colours it renders with in the stage.
    load_flags: u32,
}

/// A loaded actor: its geometry, plus the goop UV authored for it.
pub(super) struct MaskPreview {
    object_id: String,
    geometry: sms_formats::J3dGeometryPreview,
    /// Front-projected UV per triangle corner, in `[0, 1]`.
    front_uv: Vec<[f32; 2]>,
    center: [f32; 3],
    radius: f32,
    triangle_count: usize,
    /// The material that binds each texture slot, so a triangle can find the
    /// TEV program that shades it. Triangles carry no material index here.
    material_for_texture: Vec<Option<usize>>,
}

impl MaskPreview {
    /// Texture names the model carries, for the assignment dropdowns.
    fn texture_names(&self) -> Vec<(usize, String)> {
        self.geometry
            .textures
            .iter()
            .enumerate()
            .map(|(index, texture)| {
                (
                    index,
                    format!("{} ({}x{})", texture.name, texture.width, texture.height),
                )
            })
            .collect()
    }
}

/// Projects points onto the front plane and normalises to the unit square.
///
/// This is Blender's "Project from View (Bounds)" and the layout retail's goop
/// UV uses: `x` and `y` become `u` and `v`, then the projected extent is fitted
/// to `[0, 1]` so the mask always covers the whole canvas. A degenerate axis
/// collapses to the middle of the range rather than dividing by zero.
pub(super) fn front_projection_bounds(points: &[[f32; 3]]) -> Vec<[f32; 2]> {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    let span = [max[0] - min[0], max[1] - min[1]];
    points
        .iter()
        .map(|point| {
            std::array::from_fn(|axis| {
                if span[axis] > f32::EPSILON {
                    ((point[axis] - min[axis]) / span[axis]).clamp(0.0, 1.0)
                } else {
                    0.5
                }
            })
        })
        .collect()
}

/// Whether goop shows at a texel: the game's own comparison.
pub(super) fn goop_is_visible(mask_value: u8, threshold: u8) -> bool {
    mask_value > threshold
}

/// Orbits a point around the model's centre.
fn orbit(point: [f32; 3], yaw: f32, pitch: f32) -> [f32; 3] {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let x = point[0] * cos_yaw + point[2] * sin_yaw;
    let z = -point[0] * sin_yaw + point[2] * cos_yaw;
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    let y = point[1] * cos_pitch - z * sin_pitch;
    let z = point[1] * sin_pitch + z * cos_pitch;
    [x, y, z]
}

/// Bilinear sample of a mask, so the wash edge follows a smooth gradient
/// instead of stepping across a low-resolution mask's texels.
pub(super) fn sample_mask_bilinear(mask: &[u8], size: usize, u: f32, v: f32) -> u8 {
    if size == 0 || mask.len() < size * size {
        return 0;
    }
    let x = (u.clamp(0.0, 1.0) * (size - 1) as f32).max(0.0);
    let y = ((1.0 - v.clamp(0.0, 1.0)) * (size - 1) as f32).max(0.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(size - 1);
    let y1 = (y0 + 1).min(size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |x: usize, y: usize| mask[y * size + x] as f32;
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
}

/// A rainbow stand-in colour map, so a generated coating is obviously custom.
fn rainbow_goop(u: f32, v: f32) -> [u8; 4] {
    let hue = (u * 0.75 + v * 0.25).fract() * 6.0;
    let sector = hue.floor() as i32;
    let f = hue - hue.floor();
    let (r, g, b) = match sector.rem_euclid(6) {
        0 => (1.0, f, 0.0),
        1 => (1.0 - f, 1.0, 0.0),
        2 => (0.0, 1.0, f),
        3 => (0.0, 1.0 - f, 1.0),
        4 => (f, 0.0, 1.0),
        _ => (1.0, 0.0, 1.0 - f),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]
}

/// A procedural mask used when the stage carries no retail mask to borrow.
fn procedural_mask(size: usize) -> (usize, Vec<u8>) {
    let mut values = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let blobs = (u * 8.0).sin() * (v * 6.0).cos() + (u * 3.0 + v * 5.0).sin() * 0.6;
            let normalised = ((blobs + 1.6) / 3.2).clamp(0.0, 1.0);
            values[y * size + x] = (normalised * 255.0) as u8;
        }
    }
    (size, values)
}

/// One channel of a TEV colour combine, in GX's fixed point.
///
/// Ported from the viewport shader's `tev_regular_channel` so the preview
/// shades a model the same way the stage does: inputs read the low eight bits
/// of a register, C expands 0..255 to 0..256, the scale is folded into both
/// terms, and add and subtract round differently.
struct TevOp {
    op: u8,
    bias: u8,
    scale: u8,
    clamp_on: bool,
}

fn tev_channel(a: f32, b: f32, c: f32, d: f32, operation: &TevOp) -> f32 {
    let TevOp {
        op,
        bias,
        scale,
        clamp_on,
    } = *operation;
    let to_s10 = |value: f32| (value * 255.0).round().clamp(-1024.0, 1023.0) as i32;
    let input = |value: f32| to_s10(value) & 255;
    let (a, b, c, d) = (input(a), input(b), input(c), to_s10(d));
    let bias = match bias {
        1 => 128,
        2 => -128,
        _ => 0,
    };
    let lerp_numerator = a * 256 + (b - a) * (c + (c >> 7));
    let subtract = op == 1;
    let result = if scale == 3 {
        let lerp = lerp_numerator >> 8;
        let sum = if subtract {
            d + bias - lerp
        } else {
            d + bias + lerp
        };
        sum >> 1
    } else {
        let factor = match scale {
            1 => 2,
            2 => 4,
            _ => 1,
        };
        let rounding = if subtract { 127 } else { 128 };
        let lerp = (lerp_numerator * factor + rounding) >> 8;
        if subtract {
            (d + bias) * factor - lerp
        } else {
            (d + bias) * factor + lerp
        }
    };
    let clamped = if clamp_on {
        result.clamp(0, 255)
    } else {
        result.clamp(-1024, 1023)
    };
    clamped as f32 / 255.0
}

/// A TEV colour input selector, ported from the shader's `color_arg`.
fn tev_colour_arg(
    selector: u8,
    previous: [f32; 4],
    registers: [[f32; 4]; 3],
    texture: [f32; 4],
    raster: [f32; 4],
    konst: [f32; 3],
) -> [f32; 3] {
    let splat = |value: f32| [value, value, value];
    match selector {
        0 => [previous[0], previous[1], previous[2]],
        1 => splat(previous[3]),
        2 => [registers[0][0], registers[0][1], registers[0][2]],
        3 => splat(registers[0][3]),
        4 => [registers[1][0], registers[1][1], registers[1][2]],
        5 => splat(registers[1][3]),
        6 => [registers[2][0], registers[2][1], registers[2][2]],
        7 => splat(registers[2][3]),
        8 => [texture[0], texture[1], texture[2]],
        9 => splat(texture[3]),
        10 => [raster[0], raster[1], raster[2]],
        11 => splat(raster[3]),
        12 => splat(1.0),
        13 => splat(0.5),
        14 => konst,
        16 => splat(texture[0]),
        17 => splat(texture[1]),
        18 => splat(texture[2]),
        _ => splat(0.0),
    }
}

/// A konst colour selector, ported from the shader's `konst_color`.
fn tev_konst_colour(selector: u8, konst_colours: &[[u8; 4]; 4]) -> [f32; 3] {
    let splat = |value: f32| [value, value, value];
    match selector {
        0 => splat(1.0),
        1 => splat(0.875),
        2 => splat(0.75),
        3 => splat(0.625),
        4 => splat(0.5),
        5 => splat(0.375),
        6 => splat(0.25),
        7 => splat(0.125),
        12..=15 => {
            let colour = konst_colours[(selector - 12) as usize];
            [
                colour[0] as f32 / 255.0,
                colour[1] as f32 / 255.0,
                colour[2] as f32 / 255.0,
            ]
        }
        16..=31 => {
            let index = ((selector - 16) & 3) as usize;
            let channel = ((selector - 16) >> 2) as usize;
            splat(konst_colours[index][channel] as f32 / 255.0)
        }
        _ => splat(1.0),
    }
}

/// Whether a texture is a toon shading ramp rather than surface colour.
fn is_toon_ramp(name: &str) -> bool {
    name.to_ascii_lowercase().contains("toon")
}

/// Whether a pixel survives a material's alpha test.
///
/// Cutout geometry -- a leaf, a fin, a frond -- is a flat quad whose shape
/// comes entirely from discarding texels that fail this test. Without it the
/// quad renders solid and the shape is lost.
pub(super) fn alpha_compare_passes(compare: &sms_formats::J3dAlphaCompare, alpha: u8) -> bool {
    let test = |function: u8, reference: u8| match function {
        0 => false,
        1 => alpha < reference,
        2 => alpha == reference,
        3 => alpha <= reference,
        4 => alpha > reference,
        5 => alpha != reference,
        6 => alpha >= reference,
        _ => true,
    };
    let first = test(compare.comp0, compare.ref0);
    let second = test(compare.comp1, compare.ref1);
    match compare.op {
        0 => first && second,
        1 => first || second,
        2 => first ^ second,
        3 => !(first ^ second),
        _ => true,
    }
}

/// Runs a material's TEV program for one pixel.
///
/// This is the same pipeline the stage viewport evaluates in its shader, which
/// is why an actor whose colour lives in a TEV register -- a toon-shaded body
/// sampling a greyscale ramp, say -- comes out in its own colour here rather
/// than grey. `sample` supplies a texture for a stage's texture map.
pub(super) fn evaluate_tev(
    material: &sms_formats::J3dMaterial,
    raster: [f32; 4],
    sample: &dyn Fn(usize, Option<usize>) -> [f32; 4],
) -> [u8; 4] {
    let register = |colour: [i16; 4]| {
        [
            colour[0] as f32 / 255.0,
            colour[1] as f32 / 255.0,
            colour[2] as f32 / 255.0,
            colour[3] as f32 / 255.0,
        ]
    };
    let mut previous = [0.0f32; 4];
    let mut registers = [
        register(material.tev_colors[0]),
        register(material.tev_colors[1]),
        register(material.tev_colors[2]),
    ];

    for stage in &material.tev_stages {
        let texture = match stage.order.tex_map {
            Some(map) => sample(map as usize, stage.order.tex_coord.map(usize::from)),
            None => [1.0; 4],
        };
        let konst = tev_konst_colour(stage.konst_color, &material.tev_k_colors);
        let arg =
            |selector: u8| tev_colour_arg(selector, previous, registers, texture, raster, konst);
        let (a, b, c, d) = (
            arg(stage.color_args[0]),
            arg(stage.color_args[1]),
            arg(stage.color_args[2]),
            arg(stage.color_args[3]),
        );
        let clamp_on = stage.color_clamp != 0;
        let colour: [f32; 3] = if stage.color_op <= 1 {
            std::array::from_fn(|channel| {
                tev_channel(
                    a[channel],
                    b[channel],
                    c[channel],
                    d[channel],
                    &TevOp {
                        op: stage.color_op,
                        bias: stage.color_bias,
                        scale: stage.color_scale,
                        clamp_on,
                    },
                )
            })
        } else {
            // Comparison stages gate C on a test of A against B.
            let to_u8 = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u32;
            let passes = match stage.color_op {
                8 => to_u8(a[0]) > to_u8(b[0]),
                9 => to_u8(a[0]) == to_u8(b[0]),
                10 => to_u8(a[0]) | (to_u8(a[1]) << 8) > to_u8(b[0]) | (to_u8(b[1]) << 8),
                11 => to_u8(a[0]) | (to_u8(a[1]) << 8) == to_u8(b[0]) | (to_u8(b[1]) << 8),
                _ => to_u8(a[0]) > to_u8(b[0]),
            };
            std::array::from_fn(|channel| d[channel] + if passes { c[channel] } else { 0.0 })
        };

        let target = &mut match stage.color_register {
            0 => &mut previous,
            other => &mut registers[(other as usize - 1).min(2)],
        }[..3];
        target.copy_from_slice(&colour);
    }

    // The pixel engine keeps the low eight bits of the final register.
    std::array::from_fn(|channel| {
        let value = previous[channel];
        ((value * 255.0).round().clamp(-1024.0, 1023.0) as i32 & 255) as u8
    })
}

/// Nearest-neighbour sample of a decoded preview texture, with wrapping.
fn sample_texture(texture: &sms_formats::J3dTexturePreview, u: f32, v: f32) -> Option<[u8; 4]> {
    let width = texture.width as usize;
    let height = texture.height as usize;
    if width == 0 || height == 0 || texture.rgba.len() < width * height * 4 {
        return None;
    }
    let x = ((u.rem_euclid(1.0)) * width as f32) as usize % width;
    let y = ((v.rem_euclid(1.0)) * height as f32) as usize % height;
    let base = (y * width + x) * 4;
    Some([
        texture.rgba[base],
        texture.rgba[base + 1],
        texture.rgba[base + 2],
        texture.rgba[base + 3],
    ])
}

/// The model a placed actor renders with.
///
/// Resolution follows the viewport's own order: an explicit preview asset hint
/// first, then the catalog's actor preview, then an inferred hint. Checking
/// only the catalog missed actors placed from the content browser, which carry
/// their model as a hint.
fn mask_model_path(
    document: &sms_scene::StageDocument,
    object: &sms_scene::SceneObject,
) -> Option<String> {
    let hint = |role: sms_scene::AssetRole| {
        object
            .asset_hints
            .iter()
            .find(|asset| asset.role == role)
            .map(|asset| asset.path.clone())
    };
    hint(sms_scene::AssetRole::PreviewModel)
        .or_else(|| {
            document
                .actor_preview(object)
                .map(|preview| preview.model_path.clone())
        })
        .or_else(|| hint(sms_scene::AssetRole::InferredPreviewModel))
}

const CANVAS: usize = 384;

impl SmsEditorApp {
    /// Enemy actors placed in the loaded stage, as sampler choices.
    fn mask_actor_choices(&self) -> Vec<MaskActorChoice> {
        let (Some(document), Some(registry)) = (self.document.as_ref(), self.registry.as_ref())
        else {
            return Vec::new();
        };
        let mut choices = Vec::new();
        for object in &document.objects {
            if registry.find_enemy_actor(&object.factory_name).is_none() {
                continue;
            }
            let Some(model_path) = mask_model_path(document, object) else {
                continue;
            };
            let load_flags = document
                .actor_preview(object)
                .map(|preview| preview.load_flags)
                .unwrap_or_else(|| model_loader_flags_for_path(&model_path));
            choices.push(MaskActorChoice {
                object_id: object.id.clone(),
                label: format!("{} \u{2014} {}", object.factory_name, object.id),
                model_path,
                load_flags,
            });
        }
        choices.sort_by(|left, right| left.label.cmp(&right.label));
        choices
    }

    /// Loads the chosen actor's geometry and authors its goop UV.
    fn build_mask_preview(&mut self, choice: &MaskActorChoice) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let bytes = match document.read_asset_bytes(&choice.model_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not read the model: {error}"));
                return;
            }
        };
        let model = match sms_formats::J3dFile::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not parse the model: {error}"));
                return;
            }
        };
        let geometry = match model.geometry_preview_with_loader_flags(choice.load_flags) {
            Ok(geometry) => geometry,
            Err(error) => {
                self.log
                    .push(format!("Mask Tool could not read the geometry: {error}"));
                return;
            }
        };
        if geometry.triangles.is_empty() {
            self.log
                .push("That model has no triangles to preview.".to_string());
            return;
        }

        let corners = geometry
            .triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .collect::<Vec<_>>();
        let front_uv = front_projection_bounds(&corners);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for point in &corners {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        let center = std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5);
        let radius = (0..3)
            .map(|axis| (max[axis] - min[axis]) * 0.5)
            .fold(0.0f32, f32::max)
            .max(f32::EPSILON);

        let mut material_for_texture = vec![None; geometry.textures.len()];
        for (index, material) in geometry.materials.iter().enumerate() {
            let Some(slot) = material.texture_indices.iter().flatten().next().copied() else {
                continue;
            };
            if slot < material_for_texture.len() && material_for_texture[slot].is_none() {
                material_for_texture[slot] = Some(index);
            }
        }

        let triangle_count = geometry.triangles.len();
        self.mask_preview = Some(MaskPreview {
            object_id: choice.object_id.clone(),
            geometry,
            front_uv,
            center,
            radius,
            triangle_count,
            material_for_texture,
        });
        self.mask_yaw = 0.0;
        self.mask_pitch = 0.0;
        self.log.push(format!(
            "Mask Tool loaded {} ({triangle_count} triangles).",
            choice.label
        ));
    }

    /// Seeds generated goop content.
    fn generate_mask_content(&mut self) {
        let borrowed = self.retail_polmask();
        let (size, values) = borrowed.unwrap_or_else(|| procedural_mask(32));
        self.mask_mask_size = size;
        self.mask_mask = values;
        self.mask_generated = true;
    }

    /// StayPakkun's pollution mask, if the loaded stage carries pakun.bmd.
    fn retail_polmask(&mut self) -> Option<(usize, Vec<u8>)> {
        let candidates = self
            .document
            .as_ref()?
            .assets
            .iter()
            .filter(|asset| {
                asset
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .replace('\\', "/")
                    .ends_with("pakun.bmd")
            })
            .map(|asset| asset.path.clone())
            .collect::<Vec<_>>();
        for path in candidates {
            let document = self.document.as_ref()?;
            let Ok(bytes) = document.read_asset_bytes(&path) else {
                continue;
            };
            let Ok(model) = sms_formats::J3dFile::parse(&bytes) else {
                continue;
            };
            let Ok(geometry) = model.geometry_preview() else {
                continue;
            };
            let Some(texture) = geometry
                .textures
                .iter()
                .find(|texture| texture.name.to_ascii_lowercase().contains("polmask"))
            else {
                continue;
            };
            let size = texture.width.min(texture.height) as usize;
            if size == 0 {
                continue;
            }
            let mut values = vec![0u8; size * size];
            for y in 0..size {
                for x in 0..size {
                    let source = (y * texture.width as usize + x) * 4;
                    values[y * size + x] = texture.rgba.get(source).copied().unwrap_or(0);
                }
            }
            self.log.push(format!(
                "Borrowed the retail mask '{}' ({size}x{size}).",
                texture.name
            ));
            return Some((size, values));
        }
        None
    }

    /// Decodes a retail goop style's surface texture, so the coating can use
    /// the same look the goop tool paints the ground with.
    ///
    /// A pollution model carries the stage's coverage mask first, then the goop
    /// material, then a shared edge map; the material at index 1 is the one
    /// that reads as the goop.
    fn load_goop_style(&mut self, index: usize) {
        let Some(template) = self.retail_goop_templates.get(index) else {
            return;
        };
        let label = crate::goop::goop_template_label(template, true);
        let archive_path = template.archive_path.clone();
        let layer_index = template.layer_index;
        let model_path = format!("map/pollution/pollution{layer_index:02}.bmd");

        let bytes = match std::fs::read(&archive_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.log
                    .push(format!("Could not read the goop style archive: {error}"));
                return;
            }
        };
        let archive = match sms_scene::SourceFreeStageArchive::parse(&bytes) {
            Ok(archive) => archive,
            Err(error) => {
                self.log
                    .push(format!("Could not open the goop style archive: {error}"));
                return;
            }
        };
        let Some(resource) = archive.resources().iter().find(|resource| {
            String::from_utf8_lossy(&resource.raw_path)
                .replace('\\', "/")
                .eq_ignore_ascii_case(&model_path)
        }) else {
            self.log
                .push(format!("Goop style '{label}' has no {model_path}."));
            return;
        };
        let sms_scene::StageResourceDocument::Model(model) = &resource.document else {
            self.log.push(format!(
                "Goop style '{label}' does not store {model_path} as a model."
            ));
            return;
        };
        let encoded = match model.to_bytes() {
            Ok(encoded) => encoded,
            Err(error) => {
                self.log
                    .push(format!("Could not encode the goop style model: {error}"));
                return;
            }
        };
        let Ok(parsed) = sms_formats::J3dFile::parse(&encoded) else {
            self.log.push(format!(
                "Could not parse the goop style model for '{label}'."
            ));
            return;
        };
        let Ok(geometry) = parsed.geometry_preview() else {
            self.log.push(format!(
                "Could not read the goop style geometry for '{label}'."
            ));
            return;
        };
        // Index 1 is the goop material; index 0 is the stage's coverage mask.
        let texture = geometry
            .textures
            .get(1)
            .or_else(|| geometry.textures.first());
        let Some(texture) = texture else {
            self.log
                .push(format!("Goop style '{label}' carries no texture."));
            return;
        };
        self.mask_goop_image = Some((
            texture.width as usize,
            texture.height as usize,
            texture.rgba.clone(),
        ));
        self.log.push(format!(
            "Goop style '{label}' using '{}' ({}x{}).",
            texture.name, texture.width, texture.height
        ));
    }

    /// The mask currently in use, as a square intensity field.
    fn active_mask(&self) -> Option<(usize, Vec<u8>)> {
        match self.mask_mask_source {
            MaskTextureSource::Generated | MaskTextureSource::GoopStyle(_) => self
                .mask_generated
                .then(|| (self.mask_mask_size, self.mask_mask.clone())),
            MaskTextureSource::Model(index) => {
                let texture = self.mask_preview.as_ref()?.geometry.textures.get(index)?;
                let size = texture.width.min(texture.height) as usize;
                if size == 0 {
                    return None;
                }
                let mut values = vec![0u8; size * size];
                for y in 0..size {
                    for x in 0..size {
                        let source = (y * texture.width as usize + x) * 4;
                        values[y * size + x] = texture.rgba.get(source).copied().unwrap_or(0);
                    }
                }
                Some((size, values))
            }
        }
    }

    /// The goop colour at a UV, from whichever source is assigned.
    fn goop_colour(&self, u: f32, v: f32) -> [u8; 4] {
        match self.mask_colour_source {
            MaskTextureSource::Generated => rainbow_goop(u, v),
            MaskTextureSource::GoopStyle(_) => self
                .mask_goop_image
                .as_ref()
                .and_then(|(width, height, rgba)| {
                    if *width == 0 || *height == 0 || rgba.len() < width * height * 4 {
                        return None;
                    }
                    let x = ((u.rem_euclid(1.0)) * *width as f32) as usize % width;
                    let y = ((v.rem_euclid(1.0)) * *height as f32) as usize % height;
                    let base = (y * width + x) * 4;
                    Some([rgba[base], rgba[base + 1], rgba[base + 2], 255])
                })
                .unwrap_or_else(|| rainbow_goop(u, v)),
            MaskTextureSource::Model(index) => self
                .mask_preview
                .as_ref()
                .and_then(|preview| preview.geometry.textures.get(index))
                .and_then(|texture| sample_texture(texture, u, v))
                .unwrap_or_else(|| rainbow_goop(u, v)),
        }
    }

    /// Rasterises the model at the current orbit, returning shaded colour and
    /// the goop UV per pixel.
    ///
    /// Shading follows the geometry's own resolved decisions rather than a
    /// guess: each triangle carries the combine mode, vertex colours and flat
    /// colour the stage viewport uses, so an actor whose colour lives in its
    /// vertices or material (rather than a texture) reads the same here.
    /// Normals are interpolated across the triangle, so curved surfaces read as
    /// curved instead of faceted.
    #[allow(clippy::type_complexity)]
    fn rasterize_model(&self) -> Option<(Vec<Option<[u8; 4]>>, Vec<[f32; 2]>)> {
        use sms_formats::J3dPreviewCombineMode as Combine;

        let preview = self.mask_preview.as_ref()?;
        let geometry = &preview.geometry;
        let mut base = vec![None; CANVAS * CANVAS];
        let mut goop_uv = vec![[0.0f32; 2]; CANVAS * CANVAS];
        let mut depth = vec![f32::INFINITY; CANVAS * CANVAS];

        for (index, triangle) in geometry.triangles.iter().enumerate() {
            // Screen space follows the orbit; the goop UV stays the front
            // projection, so authoring never drifts with the camera.
            let view: [[f32; 3]; 3] = std::array::from_fn(|corner| {
                let point = triangle.vertices[corner];
                let relative = std::array::from_fn(|axis| point[axis] - preview.center[axis]);
                orbit(relative, self.mask_yaw, self.mask_pitch)
            });
            let screen: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                [
                    (view[corner][0] / preview.radius * 0.45 + 0.5) * (CANVAS - 1) as f32,
                    (0.5 - view[corner][1] / preview.radius * 0.45) * (CANVAS - 1) as f32,
                ]
            });
            let uv = [
                preview.front_uv[index * 3],
                preview.front_uv[index * 3 + 1],
                preview.front_uv[index * 3 + 2],
            ];
            let normals: Option<[[f32; 3]; 3]> = triangle.normals.map(|normals| {
                std::array::from_fn(|corner| orbit(normals[corner], self.mask_yaw, self.mask_pitch))
            });

            let area = (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                - (screen[2][0] - screen[0][0]) * (screen[1][1] - screen[0][1]);
            if area.abs() < f32::EPSILON {
                continue;
            }
            let min_x = screen
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_x = (screen
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(CANVAS - 1);
            let min_y = screen
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.0) as usize;
            let max_y = (screen
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as usize)
                .min(CANVAS - 1);

            let alpha_compare = triangle.alpha_compare.or_else(|| {
                triangle
                    .texture_index
                    .and_then(|slot| preview.material_for_texture.get(slot).copied().flatten())
                    .or(triangle.material_index)
                    .and_then(|index| geometry.materials.get(index))
                    .map(|material| material.alpha_compare)
            });
            let texture = triangle
                .texture_index
                .and_then(|slot| geometry.textures.get(slot))
                .or_else(|| {
                    triangle
                        .material_index
                        .and_then(|material| geometry.materials.get(material))
                        .and_then(|material| material.texture_indices.iter().flatten().next())
                        .and_then(|slot| geometry.textures.get(*slot))
                });
            let tex_coords = triangle.tex_coords.or(triangle.tex_coord_sets[0]);

            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    let w0 = ((screen[1][0] - screen[0][0]) * (point[1] - screen[0][1])
                        - (point[0] - screen[0][0]) * (screen[1][1] - screen[0][1]))
                        / area;
                    let w1 = ((point[0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                        - (screen[2][0] - screen[0][0]) * (point[1] - screen[0][1]))
                        / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let pixel_depth = view[0][2] * w2 + view[1][2] * w1 + view[2][2] * w0;
                    let slot = y * CANVAS + x;
                    if pixel_depth >= depth[slot] {
                        continue;
                    }
                    depth[slot] = pixel_depth;
                    goop_uv[slot] = [
                        uv[0][0] * w2 + uv[1][0] * w1 + uv[2][0] * w0,
                        uv[0][1] * w2 + uv[1][1] * w1 + uv[2][1] * w0,
                    ];

                    // Smooth shading: interpolate the normal, not the face.
                    let shade = match normals {
                        Some(normals) => {
                            let n: [f32; 3] = std::array::from_fn(|axis| {
                                normals[0][axis] * w2
                                    + normals[1][axis] * w1
                                    + normals[2][axis] * w0
                            });
                            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2])
                                .sqrt()
                                .max(f32::EPSILON);
                            (0.35 + 0.65 * (n[2] / length).abs()).clamp(0.0, 1.0)
                        }
                        None => 0.85,
                    };

                    let sampled = texture.zip(tex_coords).and_then(|(texture, set)| {
                        let u = set[0][0] * w2 + set[1][0] * w1 + set[2][0] * w0;
                        let v = set[0][1] * w2 + set[1][1] * w1 + set[2][1] * w0;
                        sample_texture(texture, u, v)
                    });
                    // Cutout shapes live in the texture's alpha: a leaf is a
                    // quad until the failing texels are discarded.
                    if let Some(compare) = alpha_compare {
                        let alpha = sampled.map(|sample| sample[3]).unwrap_or(255);
                        if !alpha_compare_passes(&compare, alpha) {
                            continue;
                        }
                    }
                    let vertex = triangle.vertex_colors.or(triangle.color_channels[0]).map(
                        |colors| -> [u8; 4] {
                            std::array::from_fn(|channel| {
                                (colors[0][channel] as f32 * w2
                                    + colors[1][channel] as f32 * w1
                                    + colors[2][channel] as f32 * w0)
                                    .clamp(0.0, 255.0) as u8
                            })
                        },
                    );
                    // An actor's flat colour can come from the triangle's own
                    // resolved colour or, failing that, its material -- only
                    // then fall back to neutral, so a coloured actor is never
                    // rendered plain grey.
                    let flat = triangle
                        .color
                        .or_else(|| {
                            triangle
                                .material_index
                                .and_then(|index| geometry.materials.get(index))
                                .map(|material| material.material_colors[0])
                                .filter(|colour| colour[3] > 12)
                        })
                        .unwrap_or([220, 220, 225, 255]);

                    let modulate = |a: [u8; 4], b: [u8; 4]| -> [u8; 4] {
                        std::array::from_fn(|channel| {
                            ((a[channel] as u32 * b[channel] as u32) / 255) as u8
                        })
                    };
                    // Running the material's own TEV program is what makes a
                    // toon-shaded actor come out in its colour: the body
                    // samples a greyscale ramp and the colour lives in a TEV
                    // register, so no combination of texture and material
                    // colour alone can reproduce it.
                    let material = triangle
                        .texture_index
                        .and_then(|slot| preview.material_for_texture.get(slot).copied().flatten())
                        .or(triangle.material_index)
                        .and_then(|index| geometry.materials.get(index))
                        .filter(|material| !material.tev_stages.is_empty());
                    let colour = match material {
                        Some(material) => {
                            // The raster colour is the lit vertex colour;
                            // with none stored the surface's own lighting
                            // stands in, so stages that modulate by it are not
                            // handed flat white and blown out.
                            let raster = vertex
                                .map(|colour| colour.map(|channel| channel as f32 / 255.0))
                                .unwrap_or([shade, shade, shade, 1.0]);
                            evaluate_tev(material, raster, &|map, coord| {
                                // A stage names a texmap slot, and the material
                                // maps that slot to a texture. Reading the
                                // model's texture list by the slot instead
                                // samples a different image: Petey's body names
                                // slot 0, which his material maps to his skin,
                                // while texture 0 is the leaf sheet.
                                let texture = material
                                    .texture_indices
                                    .get(map)
                                    .copied()
                                    .flatten()
                                    .and_then(|index| geometry.textures.get(index));
                                texture
                                    // A toon ramp is a lookup table indexed by
                                    // a coordinate the material generates from
                                    // the normal. This preview builds only
                                    // stored coordinates, so reading a ramp
                                    // through one samples the wrong place and
                                    // blows the model out. Skipping it leaves
                                    // the stage's other input to speak.
                                    .filter(|texture| !is_toon_ramp(&texture.name))
                                    // The stage also names which stored
                                    // coordinate set it reads.
                                    .zip(
                                        coord
                                            .and_then(|index| {
                                                triangle.tex_coord_sets.get(index).copied().flatten()
                                            })
                                            .or(tex_coords),
                                    )
                                    .and_then(|(texture, set)| {
                                        let u = set[0][0] * w2 + set[1][0] * w1 + set[2][0] * w0;
                                        let v = set[0][1] * w2 + set[1][1] * w1 + set[2][1] * w0;
                                        sample_texture(texture, u, v)
                                    })
                                    .map(|sample| sample.map(|c| c as f32 / 255.0))
                                    .unwrap_or([1.0; 4])
                            })
                        }
                        // Without a TEV program, fall back to the combine the
                        // geometry already resolved.
                        None => match triangle.combine_mode {
                            Combine::TextureOnly => sampled.unwrap_or(flat),
                            Combine::TextureModulateMaterial => match sampled {
                                Some(sample) => modulate(sample, flat),
                                None => flat,
                            },
                            Combine::TextureModulateVertex => match (sampled, vertex) {
                                (Some(sample), Some(vertex)) => modulate(sample, vertex),
                                (Some(sample), None) => sample,
                                (None, Some(vertex)) => vertex,
                                (None, None) => flat,
                            },
                            Combine::MaterialOnly => flat,
                            Combine::VertexOnly => vertex.unwrap_or(flat),
                        },
                    };

                    base[slot] = Some([
                        (colour[0] as f32 * shade) as u8,
                        (colour[1] as f32 * shade) as u8,
                        (colour[2] as f32 * shade) as u8,
                        255,
                    ]);
                }
            }
        }
        Some((base, goop_uv))
    }

    /// Draws the selected UV layout over the mask, so a layout can be read.
    fn render_uv_inspector(&self) -> Option<egui::ColorImage> {
        let preview = self.mask_preview.as_ref()?;
        let mask = self.active_mask();
        let mut pixels = vec![[26u8, 26, 32, 255]; CANVAS * CANVAS];
        if let Some((size, values)) = &mask {
            for y in 0..CANVAS {
                for x in 0..CANVAS {
                    let u = x as f32 / (CANVAS - 1) as f32;
                    let v = 1.0 - y as f32 / (CANVAS - 1) as f32;
                    let value = sample_mask_bilinear(values, *size, u, v);
                    let shade = (value as f32 * 0.6) as u8;
                    pixels[y * CANVAS + x] = [shade, shade, shade.saturating_add(14), 255];
                }
            }
        }

        let mut line = |from: [f32; 2], to: [f32; 2]| {
            let steps = ((to[0] - from[0]).abs().max((to[1] - from[1]).abs()) as usize).max(1);
            for step in 0..=steps {
                let t = step as f32 / steps as f32;
                let x = (from[0] + (to[0] - from[0]) * t).round();
                let y = (from[1] + (to[1] - from[1]) * t).round();
                if x >= 0.0 && y >= 0.0 && (x as usize) < CANVAS && (y as usize) < CANVAS {
                    pixels[y as usize * CANVAS + x as usize] = [80, 255, 120, 255];
                }
            }
        };
        for (index, triangle) in preview.geometry.triangles.iter().enumerate() {
            let uv: [[f32; 2]; 3] = match self.mask_uv_layer {
                MaskUvLayer::Goop => {
                    std::array::from_fn(|corner| preview.front_uv[index * 3 + corner])
                }
                MaskUvLayer::Body => match triangle.tex_coords.or(triangle.tex_coord_sets[0]) {
                    Some(set) => set,
                    None => continue,
                },
            };
            let screen: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                [
                    uv[corner][0].clamp(0.0, 1.0) * (CANVAS - 1) as f32,
                    (1.0 - uv[corner][1].clamp(0.0, 1.0)) * (CANVAS - 1) as f32,
                ]
            });
            line(screen[0], screen[1]);
            line(screen[1], screen[2]);
            line(screen[2], screen[0]);
        }

        let flat = pixels.into_iter().flatten().collect::<Vec<_>>();
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [CANVAS, CANVAS],
            &flat,
        ))
    }

    /// Composites the model preview at the current wash phase.
    fn mask_preview_image(&self) -> Option<egui::ColorImage> {
        if self.mask_view == MaskView::Uv {
            return self.render_uv_inspector();
        }
        let (base, goop_uv) = self.rasterize_model()?;
        let threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mask = self.active_mask();
        let mut pixels = Vec::with_capacity(CANVAS * CANVAS * 4);
        for slot in 0..CANVAS * CANVAS {
            let Some(base_colour) = base[slot] else {
                pixels.extend_from_slice(&[24, 24, 30, 255]);
                continue;
            };
            let mut colour = base_colour;
            if self.mask_uv_layer == MaskUvLayer::Goop {
                if let Some((size, values)) = &mask {
                    let [u, v] = goop_uv[slot];
                    let mask_value = sample_mask_bilinear(values, *size, u, v);
                    if goop_is_visible(mask_value, threshold) {
                        // The coating is opaque: the model underneath must not
                        // read through it, or a mask cannot be judged.
                        colour = self.goop_colour(u, v);
                    }
                }
            }
            pixels.extend_from_slice(&colour);
        }
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [CANVAS, CANVAS],
            &pixels,
        ))
    }

    /// The inspector panel for the Mask Tool.
    pub(super) fn mask_tool_panel(&mut self, ui: &mut egui::Ui) {
        // The goop catalog is indexed lazily by whoever needs it first; the
        // colour list offers those styles, so make sure they are loaded.
        self.ensure_goop_templates_indexed();
        ui.heading("Mask Tool");
        ui.label(
            egui::RichText::new("Author washable goop on the enemies placed in this stage")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.separator();

        let choices = self.mask_actor_choices();
        if choices.is_empty() {
            ui.label(
                "No placed enemy actor in this stage has a previewable model. Place one, or open \
                 a stage that has one.",
            );
            return;
        }

        let selected_label = self
            .mask_selected_actor
            .as_ref()
            .and_then(|id| choices.iter().find(|choice| &choice.object_id == id))
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| "Choose an actor".to_string());
        let mut picked = None;
        egui::ComboBox::from_label("Actor")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for choice in &choices {
                    let active = self.mask_selected_actor.as_deref() == Some(&choice.object_id);
                    if ui.selectable_label(active, &choice.label).clicked() {
                        picked = Some(choice.object_id.clone());
                    }
                }
            });
        if let Some(id) = picked {
            self.mask_selected_actor = Some(id.clone());
            if let Some(choice) = choices.iter().find(|choice| choice.object_id == id) {
                self.build_mask_preview(choice);
            }
        }
        if self.mask_selected_actor.is_none() {
            ui.separator();
            ui.label("Pick an actor to load its model.");
            return;
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("View:");
            ui.selectable_value(&mut self.mask_view, MaskView::Model, "Model");
            ui.selectable_value(&mut self.mask_view, MaskView::Uv, "UV inspector");
        });
        ui.horizontal(|ui| {
            ui.label("UV layer:");
            ui.selectable_value(&mut self.mask_uv_layer, MaskUvLayer::Body, "Body (UV0)");
            ui.selectable_value(&mut self.mask_uv_layer, MaskUvLayer::Goop, "Goop");
        });
        if self.mask_view == MaskView::Model {
            ui.label(
                egui::RichText::new("Drag in the viewport to orbit.")
                    .small()
                    .color(egui::Color32::GRAY),
            );
        }

        ui.separator();
        ui.heading("Goop textures");
        let textures = self
            .mask_preview
            .as_ref()
            .map(|preview| preview.texture_names())
            .unwrap_or_default();

        let goop_styles = self
            .retail_goop_templates
            .iter()
            .enumerate()
            .filter(|(_, template)| template.compatible)
            .map(|(index, template)| (index, crate::goop::goop_template_label(template, true)))
            .collect::<Vec<_>>();
        let colour_label = match self.mask_colour_source {
            MaskTextureSource::Generated => "Rainbow (generated)".to_string(),
            MaskTextureSource::Model(index) => textures
                .iter()
                .find(|(slot, _)| *slot == index)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Rainbow (generated)".to_string()),
            MaskTextureSource::GoopStyle(index) => goop_styles
                .iter()
                .find(|(slot, _)| *slot == index)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Goop style".to_string()),
        };
        let mut picked_style = None;
        egui::ComboBox::from_label("Colour")
            .selected_text(colour_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.mask_colour_source,
                    MaskTextureSource::Generated,
                    "Rainbow (generated)",
                );
                for (index, name) in &textures {
                    ui.selectable_value(
                        &mut self.mask_colour_source,
                        MaskTextureSource::Model(*index),
                        name,
                    );
                }
                if !goop_styles.is_empty() {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Goop styles")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    for (index, name) in &goop_styles {
                        if ui
                            .selectable_label(
                                self.mask_colour_source == MaskTextureSource::GoopStyle(*index),
                                name,
                            )
                            .clicked()
                        {
                            picked_style = Some(*index);
                        }
                    }
                }
            });
        if let Some(index) = picked_style {
            self.mask_colour_source = MaskTextureSource::GoopStyle(index);
            self.load_goop_style(index);
            self.mask_uv_layer = MaskUvLayer::Goop;
        }

        let mask_label = match self.mask_mask_source {
            MaskTextureSource::Generated | MaskTextureSource::GoopStyle(_) => {
                "Generated / borrowed".to_string()
            }
            MaskTextureSource::Model(index) => textures
                .iter()
                .find(|(slot, _)| *slot == index)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Generated / borrowed".to_string()),
        };
        egui::ComboBox::from_label("Mask")
            .selected_text(mask_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.mask_mask_source,
                    MaskTextureSource::Generated,
                    "Generated / borrowed",
                );
                for (index, name) in &textures {
                    ui.selectable_value(
                        &mut self.mask_mask_source,
                        MaskTextureSource::Model(*index),
                        name,
                    );
                }
            });

        ui.horizontal(|ui| {
            if ui
                .button("Generate goop map + mask")
                .on_hover_text(
                    "Seed a rainbow colour map and the retail StayPakkun mask (or a procedural \
                     stand-in if this stage has none).",
                )
                .clicked()
            {
                self.generate_mask_content();
                self.mask_colour_source = MaskTextureSource::Generated;
                self.mask_mask_source = MaskTextureSource::Generated;
                self.mask_uv_layer = MaskUvLayer::Goop;
            }
            if ui
                .button("Create goop UV (front projection)")
                .on_hover_text(
                    "The preview already projects this way -- retail's own goop UV is a front \
                     projection fitted to the [0,1] canvas.",
                )
                .clicked()
            {
                self.mask_view = MaskView::Uv;
                self.mask_uv_layer = MaskUvLayer::Goop;
                self.log.push(
                    "Showing the front-projected goop UV; writing it into the model's material \
                     is the authoring phase."
                        .to_string(),
                );
            }
        });

        ui.separator();
        ui.heading("Brush");
        ui.add(
            egui::Slider::new(&mut self.mask_brush_radius, 1.0..=64.0)
                .text("Radius")
                .clamping(egui::SliderClamping::Always),
        );
        ui.add(
            egui::Slider::new(&mut self.mask_brush_opacity, 0.0..=1.0)
                .text("Opacity")
                .clamping(egui::SliderClamping::Always),
        );
        ui.label(
            egui::RichText::new("Painting strokes onto the mask is the next phase.")
                .small()
                .color(egui::Color32::GRAY),
        );

        ui.separator();
        ui.heading("Wash");
        self.mask_wash_controls(ui);

        ui.separator();
        if let Some(preview) = self.mask_preview.as_ref() {
            ui.label(
                egui::RichText::new(format!(
                    "{} \u{2014} {} triangles, shown in the viewport",
                    preview.object_id, preview.triangle_count
                ))
                .small()
                .color(egui::Color32::GRAY),
            );
        }
    }

    /// The Mask Tool's viewport: the model or its UV layout, filling the view.
    pub(super) fn mask_tool_viewport(&mut self, ui: &mut egui::Ui) {
        let Some(image) = self.mask_preview_image() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Pick an actor in the Mask Tool panel to load its model.")
                        .color(egui::Color32::GRAY),
                );
            });
            return;
        };
        let texture = self.mask_texture.get_or_insert_with(|| {
            ui.ctx()
                .load_texture("mask-tool-preview", image.clone(), Default::default())
        });
        texture.set(image, Default::default());
        let available = ui.available_size();
        let side = available.x.min(available.y).max(64.0);
        let response = ui.centered_and_justified(|ui| {
            ui.add(
                egui::Image::new(&*texture)
                    .fit_to_exact_size(egui::vec2(side, side))
                    .sense(egui::Sense::drag()),
            )
        });
        // Orbit applies to the model; a UV layout is flat, so it does not turn.
        if self.mask_view == MaskView::Model && response.inner.dragged() {
            let delta = response.inner.drag_delta();
            self.mask_yaw += delta.x * 0.01;
            self.mask_pitch = (self.mask_pitch + delta.y * 0.01).clamp(-1.5, 1.5);
        }
    }

    /// The wash-cycle preview: sweeps the threshold the mask is compared to.
    fn mask_wash_controls(&mut self, ui: &mut egui::Ui) {
        const CYCLE_SECONDS: f32 = 4.0;

        if self.mask_wash_playing {
            let dt = ui.input(|input| input.stable_dt).clamp(0.0, 0.1);
            self.mask_wash_phase -= dt / CYCLE_SECONDS;
            if self.mask_wash_phase <= 0.0 {
                self.mask_wash_phase = 0.0;
                self.mask_wash_playing = false;
            } else {
                ui.ctx().request_repaint();
            }
        }

        ui.horizontal(|ui| {
            let label = if self.mask_wash_playing {
                "Stop"
            } else {
                "Play full cycle"
            };
            if ui
                .button(label)
                .on_hover_text("Sweep the wash threshold from fully coated to clean")
                .clicked()
            {
                if self.mask_wash_playing {
                    self.mask_wash_playing = false;
                } else {
                    self.mask_wash_phase = 1.0;
                    self.mask_wash_playing = true;
                }
            }
            if ui.button("Reset").clicked() {
                self.mask_wash_playing = false;
                self.mask_wash_phase = 1.0;
            }
        });

        ui.add(
            egui::Slider::new(&mut self.mask_wash_phase, 0.0..=1.0)
                .text("Coverage")
                .clamping(egui::SliderClamping::Always),
        );
        ui.label(
            egui::RichText::new(format!(
                "threshold K0_A \u{2248} {}  (mask > this stays coated)",
                (self.mask_wash_phase * 255.0).round() as u16
            ))
            .small()
            .color(egui::Color32::GRAY),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated UV must fill the canvas exactly, the way retail's does.
    #[test]
    fn front_projection_fits_the_unit_square() {
        let points = [
            [-100.0, 40.0, 5.0],
            [300.0, 40.0, -5.0],
            [100.0, 240.0, 0.0],
        ];
        let uv = front_projection_bounds(&points);
        let us = uv.iter().map(|p| p[0]).collect::<Vec<_>>();
        let vs = uv.iter().map(|p| p[1]).collect::<Vec<_>>();
        assert!(us.iter().cloned().fold(f32::INFINITY, f32::min).abs() < 1e-6);
        assert!((us.iter().cloned().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-6);
        assert!(vs.iter().cloned().fold(f32::INFINITY, f32::min).abs() < 1e-6);
        assert!((vs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) - 1.0).abs() < 1e-6);
    }

    /// A flat axis must not divide by zero; it collapses to the middle.
    #[test]
    fn a_degenerate_axis_collapses_instead_of_dividing_by_zero() {
        let points = [[10.0, 7.0, 0.0], [20.0, 7.0, 0.0]];
        let uv = front_projection_bounds(&points);
        assert!(uv.iter().all(|p| p[1] == 0.5));
        assert!(uv.iter().all(|p| p[0].is_finite()));
    }

    /// The wash is the game's comparison: bright mask clings, dark clears.
    #[test]
    fn goop_recedes_in_mask_order_as_the_threshold_sweeps() {
        assert!(goop_is_visible(40, 0));
        assert!(goop_is_visible(200, 0));
        assert!(!goop_is_visible(40, 128));
        assert!(goop_is_visible(200, 128));
        assert!(!goop_is_visible(40, 255));
        assert!(!goop_is_visible(200, 255));
    }

    /// Bilinear sampling is what keeps a low-resolution mask from washing off
    /// in visible blocks: between two texels the value ramps rather than steps.
    #[test]
    fn mask_sampling_ramps_between_texels() {
        let mask = vec![0, 255, 0, 255];
        let left = sample_mask_bilinear(&mask, 2, 0.0, 0.0);
        let middle = sample_mask_bilinear(&mask, 2, 0.5, 0.0);
        let right = sample_mask_bilinear(&mask, 2, 1.0, 0.0);
        assert_eq!(left, 0);
        assert_eq!(right, 255);
        assert!(
            middle > 100 && middle < 155,
            "midpoint should interpolate, got {middle}"
        );
    }

    /// Cutout geometry depends on the alpha test: a leaf quad is only a leaf
    /// once its transparent texels are discarded.
    #[test]
    fn alpha_test_discards_transparent_texels() {
        // The common cutout setup: keep texels at or above half alpha.
        let compare = sms_formats::J3dAlphaCompare {
            comp0: 6,
            ref0: 128,
            op: 0,
            comp1: 7,
            ref1: 0,
        };
        assert!(alpha_compare_passes(&compare, 255));
        assert!(alpha_compare_passes(&compare, 128));
        assert!(!alpha_compare_passes(&compare, 127));
        assert!(!alpha_compare_passes(&compare, 0));

        // GX_ALWAYS keeps everything, which is what an opaque body wants.
        let always = sms_formats::J3dAlphaCompare {
            comp0: 7,
            ref0: 0,
            op: 1,
            comp1: 7,
            ref1: 0,
        };
        assert!(alpha_compare_passes(&always, 0));
    }

    /// Orbiting must not distort the model: a rotation preserves length.
    #[test]
    fn orbit_preserves_length() {
        let point = [3.0, 4.0, 12.0];
        let rotated = orbit(point, 0.7, -0.4);
        let length = |p: [f32; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((length(point) - length(rotated)).abs() < 1e-3);
    }
}

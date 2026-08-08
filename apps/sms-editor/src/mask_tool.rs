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
//! Masks are painted in a modelling tool rather than here: the glTF round
//! trip carries the model out with its goop coordinate and brings the painted
//! result back.

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
    /// A texture the model already carries, by index. The pickers no longer
    /// offer these -- a coating is a goop map, not whatever the model happens
    /// to carry -- but the sources still resolve one.
    #[allow(dead_code)]
    Model(usize),
    /// A retail goop style from the goop tool's catalog -- the same chocolate,
    /// oil, pink and electric surfaces the goop tool paints with.
    GoopStyle(usize),
}

/// Whether a TEV stage is the wash's comparison: `mask(goop UV) > K0_A`.
///
/// A compare op on its own is not enough to say so. A toon ramp compares too,
/// and reading one as a wash sends the bake down the "set the level on the
/// wash this model already has" path for an actor that has no wash at all --
/// nothing gets authored, the level lands in a konst the material was already
/// using, and the readback names the ramp as the goop mask. The wash is the
/// comparison that takes its level from a konst register's alpha, which is the
/// register the coverage slider drives.
fn stage_is_wash_comparison(stage: &sms_formats::J3dTevStage) -> bool {
    // A compare op is the whole test, as it always was. Requiring the stage to
    // read a konst-alpha register as well looked tighter and was wrong: retail
    // washes do not all select their level that way, so HamuKuri's own layer
    // stopped being recognised and the tool fell back to a borrowed mask over a
    // front projection while the stage renderer carried on drawing the real
    // coating perfectly well.
    stage.color_op >= 8 || stage.alpha_op >= 8
}

/// One placed enemy the Mask Tool can target.
struct MaskActorChoice {
    /// Where the object stands in the stage, to find its model when the
    /// preview registered it under something other than the object id.
    position: [f32; 3],
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

/// A stage texture's texel at a UV, honouring its wrap modes.
fn sample_preview_texel(texture: &PreviewTexture, u: f32, v: f32) -> egui::Color32 {
    let [width, height] = texture.image.size;
    if width == 0 || height == 0 {
        return egui::Color32::WHITE;
    }
    let wrap = |value: f32, mode: u8, size: usize| -> usize {
        let coordinate = match mode {
            0 => value.clamp(0.0, 1.0),
            2 => {
                let folded = value.rem_euclid(2.0);
                if folded > 1.0 {
                    2.0 - folded
                } else {
                    folded
                }
            }
            _ => value.rem_euclid(1.0),
        };
        ((coordinate * size as f32) as usize).min(size - 1)
    };
    let x = wrap(u, texture.wrap_s, width);
    let y = wrap(v, texture.wrap_t, height);
    texture.image.pixels[y * width + x]
}

/// The alpha of a stage texture at a UV.
fn sample_preview_alpha(texture: &PreviewTexture, u: f32, v: f32) -> u8 {
    sample_preview_texel(texture, u, v).a()
}

/// What the coating pass resolves at one canvas pixel: the coat UV, the
/// sweep UV, and the actor's own mask value where it authored one.
type GoopPixelUv = ([f32; 2], [f32; 2], Option<u8>);

/// Where an actor's wash-mask texture lives.
#[derive(Clone, Copy)]
enum AuthoredMaskTexture<'a> {
    /// Resolved by the stage preview.
    Stage(&'a PreviewTexture),
    /// Carried by the actor's own model. Several stage load paths drop the
    /// mask binding, so the model the Mask Tool itself loads stands in.
    Model(&'a sms_formats::J3dTexturePreview),
}

impl AuthoredMaskTexture<'_> {
    /// The wash value at a UV, in the mask's own image space.
    ///
    /// A layer this tool bakes packs its coating into the colour channels and
    /// its mask into the alpha, so that one is read from alpha. Everything else
    /// -- retail masks especially -- carries the value in the image itself, and
    /// reading those from alpha returns a constant, which is a mask that cannot
    /// recede however the coverage moves.
    fn value(&self, u: f32, v: f32) -> u8 {
        match self {
            Self::Stage(texture) => sample_preview_texel(texture, u, v).r(),
            Self::Model(texture) => sample_texture(texture, u, v)
                .map(|texel| {
                    if texture.name == GOOP_LAYER_TEXTURE {
                        texel[3]
                    } else {
                        texel[0]
                    }
                })
                .unwrap_or(0),
        }
    }
}

/// Whether goop shows at a texel: the game's own comparison.
pub(super) fn goop_is_visible(mask_value: u8, level: u8) -> bool {
    mask_value <= level
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
/// Which konst register a wash comparison sweeps.
/// Which konst register a wash comparison reads, where it reads one at all.
///
/// Answering zero for a comparison that selects no konst says the actor's
/// coating can be driven when it cannot: HamuKuri's goop is permanent in
/// retail, so nothing in its material responds, and the tool would sit writing
/// a register the comparison never looks at while suppressing its own overlay
/// in favour of a wash that never happens.
fn wash_konst_index(stage: &sms_formats::J3dTevStage) -> Option<usize> {
    match stage.konst_color {
        12..=15 => Some((stage.konst_color - 12) as usize),
        16..=31 => Some(((stage.konst_color - 16) & 3) as usize),
        _ => None,
    }
}

/// A renderable preview built from the model the Mask Tool itself loaded,
/// for actors the stage preview does not carry -- manager-spawned enemies
/// load per spawn, so nothing stands in the stage to isolate.
fn build_mask_model_preview(geometry: &sms_formats::J3dGeometryPreview) -> ModelPreview {
    let mut preview = model_assets::empty_authored_model_preview();
    let texture_base = preview_assets::push_preview_textures(&mut preview.textures, geometry);
    let material_base =
        preview_assets::push_preview_materials(&mut preview.materials, geometry, texture_base);
    preview
        .material_animation_bindings
        .resize_with(preview.materials.len(), Vec::new);
    for triangle in &geometry.triangles {
        preview.triangles.push(PreviewTriangle {
            vertices: triangle.vertices,
            normals: triangle.normals,
            color_channels: triangle.color_channels,
            tex_coord_sets: triangle.tex_coord_sets,
            material_index: triangle
                .material_index
                .map(|index| material_base + index)
                .filter(|index| *index < preview.materials.len()),
            packet_index: triangle.packet_index,
            model_index: 0,
            render_layer: PreviewRenderLayer::Main,
            color: triangle.color,
            vertex_colors: triangle.vertex_colors,
            combine_mode: triangle.combine_mode,
            tex_coords: triangle.tex_coords,
            texture_index: triangle
                .texture_index
                .map(|index| texture_base + index)
                .filter(|index| *index < preview.textures.len()),
            mask_tex_coords: triangle.mask_tex_coords,
            mask_texture_index: triangle
                .mask_texture_index
                .map(|index| texture_base + index)
                .filter(|index| *index < preview.textures.len()),
            cull_mode: triangle.cull_mode,
            alpha_compare: triangle.alpha_compare,
            blend_mode: triangle.blend_mode,
            z_mode: triangle.z_mode,
            billboard: triangle.billboard,
            particle_type: None,
            particle_pivot: None,
            particle_direction: None,
            particle_color_mode: None,
            particle_environment_color: None,
            particle_extra_texture: None,
        });
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in preview
        .triangles
        .iter()
        .flat_map(|triangle| triangle.vertices)
    {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex[axis]);
            max[axis] = max[axis].max(vertex[axis]);
        }
    }
    preview.bounds_min = min;
    preview.bounds_max = max;
    preview.camera_bounds_min = min;
    preview.camera_bounds_max = max;
    preview.loaded_models = 1;
    preview
}

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
    let resolved = hint(sms_scene::AssetRole::PreviewModel)
        .or_else(|| {
            document
                .actor_preview(object)
                .map(|preview| preview.model_path.clone())
        })
        .or_else(|| hint(sms_scene::AssetRole::InferredPreviewModel))?;
    Some(layer_pool_model_path(document, object).unwrap_or(resolved))
}

/// The model an actor spawned from a goop layer's own pool actually wears.
///
/// The goop tool styles a layer, binds managers to it, and gives each layer its
/// own copy of the actor's folder suffixed with the layer number -- `hamukuri`
/// becomes `hamukuri00` -- so the layer's own map rides with it. Every copy
/// holds a file called `default.bmd`, so a hint that names one says nothing
/// about which, and reading the wrong copy shows a coating the actor does not
/// wear.
fn layer_pool_model_path(
    document: &sms_scene::StageDocument,
    object: &sms_scene::SceneObject,
) -> Option<String> {
    let base = {
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
            .or_else(|| hint(sms_scene::AssetRole::InferredPreviewModel))?
    };
    let normalized = base.replace(char::from(92), "/");
    let (directory, file) = normalized.rsplit_once('/')?;
    let (parent, folder) = directory.rsplit_once('/')?;
    // The layer is named in the actor's manager -- `...マネージャー_L02` is
    // layer two -- and that is what picks the copy. Taking the first numbered
    // folder that exists instead gave every instance `00`, so four actors on
    // four layers all read one layer's map.
    let manager = object.raw_param("manager_name")?;
    let (_, digits) = manager.rsplit_once("_L")?;
    if digits.len() != 2 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let candidate = format!("{parent}/{folder}{digits}/{file}");
    document
        .assets
        .iter()
        .any(|asset| asset.path.to_string_lossy().replace(char::from(92), "/") == candidate)
        .then_some(candidate)
}

/// A triangle's own normal, for meshes that store none.
fn face_normal(vertices: [[f32; 3]; 3]) -> [f32; 3] {
    let edge0: [f32; 3] = std::array::from_fn(|axis| vertices[1][axis] - vertices[0][axis]);
    let edge1: [f32; 3] = std::array::from_fn(|axis| vertices[2][axis] - vertices[0][axis]);
    let normal = [
        edge0[1] * edge1[2] - edge0[2] * edge1[1],
        edge0[2] * edge1[0] - edge0[0] * edge1[2],
        edge0[0] * edge1[1] - edge0[1] * edge1[0],
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length > f32::EPSILON {
        std::array::from_fn(|axis| normal[axis] / length)
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// A glTF's JSON and the bytes its first buffer resolves to.
///
/// Blender writes `.glb` by default and `.gltf` beside a `.bin` when asked for
/// separate files, so both are read. An embedded base64 buffer is refused by
/// name rather than silently producing nothing.
fn read_gltf_document(path: &std::path::Path) -> Result<(serde_json::Value, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read the file: {error}"))?;
    if bytes.starts_with(b"glTF") {
        // A GLB is a header and then length-tagged chunks: JSON first, binary
        // second where it is present at all.
        let mut json = None;
        let mut binary = Vec::new();
        let mut offset = 12usize;
        while offset + 8 <= bytes.len() {
            let length = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            let kind = &bytes[offset + 4..offset + 8];
            let start = offset + 8;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| "A chunk runs past the end of the file.".to_string())?;
            if kind == b"JSON" {
                json = Some(
                    serde_json::from_slice(&bytes[start..end])
                        .map_err(|error| format!("The glTF's JSON is malformed: {error}"))?,
                );
            } else if kind == b"BIN\0" {
                binary = bytes[start..end].to_vec();
            }
            offset = end + (4 - end % 4) % 4;
        }
        let json = json.ok_or_else(|| "That GLB carries no JSON chunk.".to_string())?;
        return Ok((json, binary));
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The glTF's JSON is malformed: {error}"))?;
    let uri = json["buffers"][0]["uri"]
        .as_str()
        .ok_or_else(|| "That glTF names no buffer.".to_string())?;
    if uri.starts_with("data:") {
        return Err(
            "That glTF embeds its buffer as base64. Export it as glTF Separate or Binary."
                .to_string(),
        );
    }
    let directory = path.parent().unwrap_or(std::path::Path::new("."));
    let buffer = std::fs::read(directory.join(uri))
        .map_err(|error| format!("Could not read '{uri}': {error}"))?;
    Ok((json, buffer))
}

/// Reads a float accessor, honouring the byte stride its view may carry.
///
/// Values are widened to three so one reader serves `VEC2` and `VEC3`.
fn read_gltf_floats(
    json: &serde_json::Value,
    buffer: &[u8],
    accessor: usize,
    components: usize,
) -> Result<Vec<[f32; 3]>, String> {
    let accessor = &json["accessors"][accessor];
    if accessor["componentType"].as_u64() != Some(5126) {
        return Err("That mesh stores a coordinate in something other than floats.".to_string());
    }
    let count = accessor["count"]
        .as_u64()
        .ok_or_else(|| "An accessor carries no count.".to_string())? as usize;
    let view = &json["bufferViews"][accessor["bufferView"].as_u64().unwrap_or(0) as usize];
    let base = view["byteOffset"].as_u64().unwrap_or(0) as usize
        + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = view["byteStride"].as_u64().unwrap_or(0) as usize;
    let stride = if stride == 0 { components * 4 } else { stride };
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let mut value = [0.0f32; 3];
        for (component, axis) in value.iter_mut().enumerate().take(components) {
            let at = base + index * stride + component * 4;
            let bytes = buffer
                .get(at..at + 4)
                .ok_or_else(|| "An accessor runs past its buffer.".to_string())?;
            *axis = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        values.push(value);
    }
    Ok(values)
}

/// A position rounded to a hundredth, which is what a coordinate is matched on.
fn gltf_position_key(position: [f32; 3]) -> [i32; 3] {
    std::array::from_fn(|axis| (position[axis] * 100.0).round() as i32)
}

const CANVAS: usize = 384;

/// The texture an authored goop layer carries its coating and mask in.
const GOOP_LAYER_TEXTURE: &str = "graffito_goop";

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
                position: object.transform.translation,
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
        let posed = self.mask_pose_animation(choice);
        let geometry = match match posed.as_ref() {
            Some((animation, frame)) => {
                model.geometry_preview_with_pose_frame(choice.load_flags, animation, *frame)
            }
            None => model.geometry_preview_with_loader_flags(choice.load_flags),
        } {
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
        self.refresh_mask_edit_state(choice);
        let object_id = choice.object_id.clone();
        let object_position = choice.position;
        self.rebuild_mask_gpu_scene(&object_id, object_position, &geometry);
        self.mask_preview = Some(MaskPreview {
            object_id,
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
    /// The texture this actor's wash mask lives in, and its name where one
    /// can be recovered.
    ///
    /// The stage copy names the mask per triangle when the binding survives
    /// its load path; the actor's own model carries the binding regardless,
    /// so it stands in when the stage copy lost it -- which it does for
    /// StayPakkun and BossGesso. An actor with neither is coated through the
    /// borrowed StayPakkun mask and a front projection.
    fn authored_mask(&self) -> Option<(AuthoredMaskTexture<'_>, Option<String>)> {
        // The wash comparison names its own mask: `mask(goop UV) > K<n>_A` says
        // which texture it samples. Ask it first. The stage renderer's mask
        // binding is a guess -- it takes the material's first intensity
        // texture, which on LandGesso is a toon ramp, so the tool drew a ramp
        // as the mask and could not name it.
        if let Some(texture) = self
            .authored_goop_binding()
            .and_then(|(index, _)| self.mask_preview.as_ref()?.geometry.textures.get(index))
        {
            return Some((
                AuthoredMaskTexture::Model(texture),
                Some(texture.name.clone()),
            ));
        }
        if let Some(texture) = self
            .mask_gpu_triangles
            .iter()
            .find_map(|triangle| triangle.mask_texture_index)
            .and_then(|index| self.mask_gpu_textures.get(index))
        {
            // The stage texture list keeps no names; the same image inside
            // the actor's own model does. A few texels tell same-size
            // textures apart.
            let [width, height] = texture.image.size;
            let name = self.mask_preview.as_ref().and_then(|preview| {
                preview
                    .geometry
                    .textures
                    .iter()
                    .find(|candidate| {
                        candidate.width as usize == width
                            && candidate.height as usize == height
                            && {
                                let texels = (width * height).clamp(1, 16);
                                (0..texels).all(|step| {
                                    let texel = step * (width * height) / texels;
                                    let pixel = texture.image.pixels[texel];
                                    candidate.rgba.get(texel * 4..texel * 4 + 4).is_some_and(
                                        |rgba| {
                                            (rgba[0] as i32 - pixel.r() as i32).abs() <= 8
                                                && (rgba[3] as i32 - pixel.a() as i32).abs() <= 8
                                        },
                                    )
                                })
                            }
                    })
                    .map(|matched| matched.name.clone())
            });
            return Some((AuthoredMaskTexture::Stage(texture), name));
        }
        if let Some(texture) = self.model_mask_texture() {
            return Some((
                AuthoredMaskTexture::Model(texture),
                Some(texture.name.clone()),
            ));
        }
        None
    }

    /// The wash-mask texture the actor's own model carries.
    fn model_mask_texture(&self) -> Option<&sms_formats::J3dTexturePreview> {
        let preview = self.mask_preview.as_ref()?;
        let index = preview
            .geometry
            .triangles
            .iter()
            .find_map(|triangle| triangle.mask_texture_index)?;
        preview.geometry.textures.get(index)
    }

    /// Which konst register the authored wash compares against.
    ///
    /// The bake claims whichever register the material has spare, so it is
    /// often not K0 -- LandGesso already spends K0 and K1 and its layer lands
    /// on K2. Anything driving the wash from outside the editor, a DOL patch
    /// especially, has to write the register the layer actually reads, so the
    /// panel names it rather than assuming.
    fn authored_wash_konst(&self) -> Option<u8> {
        let preview = self.mask_preview.as_ref()?;
        for material in &preview.geometry.materials {
            for stage in &material.tev_stages {
                // Only a K0..K3 alpha selector names a register; anything else
                // is a comparison that takes its level elsewhere, and
                // subtracting from it reported nonsense like "K232".
                if stage_is_wash_comparison(stage) {
                    return (0x1c..=0x1f)
                        .contains(&stage.konst_alpha)
                        .then(|| stage.konst_alpha - 0x1c);
                }
            }
        }
        None
    }

    /// The mask texture and coordinate set the wash comparison names.
    ///
    /// Washable goop is a hard threshold the game evaluates per pixel, wired
    /// as a TEV comparison stage. Its order carries the authored binding --
    /// the actor's own mask texture and the UV set unwrapped for it -- so
    /// the binding is read from the material even when no preview path
    /// resolved it into the triangle fields.
    fn authored_goop_binding(&self) -> Option<(usize, usize)> {
        let preview = self.mask_preview.as_ref()?;
        for material in &preview.geometry.materials {
            for stage in &material.tev_stages {
                if !stage_is_wash_comparison(stage) {
                    continue;
                }
                let Some(map) = stage.order.tex_map else {
                    continue;
                };
                let Some(coord) = stage.order.tex_coord else {
                    continue;
                };
                let Some(texture_index) = material
                    .texture_indices
                    .get(map as usize)
                    .copied()
                    .flatten()
                else {
                    continue;
                };
                if preview.geometry.textures.get(texture_index).is_some() {
                    // The comparison names a texgen slot; the vertices store
                    // the set the slot generates from. BossGesso's wash reads
                    // coord 2, which his material generates from stored UV1
                    // through a texture matrix.
                    let stored = material
                        .tex_gens
                        .get(coord as usize)
                        .map(|gen| gen.source)
                        .filter(|source| (4..=11).contains(source))
                        .map(|source| (source - 4) as usize)
                        .unwrap_or(coord as usize);
                    return Some((texture_index, stored));
                }
            }
        }
        None
    }

    /// The coordinate set the wash comparison reads, where a model authors
    /// one.
    fn authored_goop_coord(&self) -> Option<usize> {
        self.authored_goop_binding().map(|(_, coord)| coord)
    }

    /// Reads a model texture as a wash mask.
    fn mask_from_texture(texture: &sms_formats::J3dTexturePreview) -> Option<(usize, Vec<u8>)> {
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

    fn active_mask(&self) -> Option<(usize, Vec<u8>)> {
        match self.mask_mask_source {
            MaskTextureSource::Generated | MaskTextureSource::GoopStyle(_) => self
                .mask_generated
                .then(|| (self.mask_mask_size, self.mask_mask.clone())),
            MaskTextureSource::Model(index) => {
                let texture = self.mask_preview.as_ref()?.geometry.textures.get(index)?;
                Self::mask_from_texture(texture)
            }
        }
    }

    /// How a mask value reads for the wash.
    ///
    /// Inverting turns the recede inside out: what the coating clears first
    /// becomes what it holds longest. The mask is flipped rather than the
    /// comparison, because a model carries its comparison fixed once written,
    /// so flipping the values is what keeps a baked coating behaving the way
    /// the preview showed it.
    fn mask_reading(&self, value: u8) -> u8 {
        if self.mask_wash_invert {
            255 - value
        } else {
            value
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
    fn rasterize_model(&self) -> Option<(Vec<Option<[u8; 4]>>, Vec<Option<[f32; 2]>>)> {
        use sms_formats::J3dPreviewCombineMode as Combine;

        let preview = self.mask_preview.as_ref()?;
        let geometry = &preview.geometry;
        let authored_coord = self.authored_goop_coord();
        let actor_authored = self.authored_mask().is_some();
        // The wash drives the comparison's konst here too, so the base render
        // carries the goop at the slider's coverage itself.
        let wash_threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        let washed_materials: Vec<sms_formats::J3dMaterial> = geometry
            .materials
            .iter()
            .map(|material| {
                let Some(stage) = material
                    .tev_stages
                    .iter()
                    .find(|stage| stage_is_wash_comparison(stage))
                else {
                    return material.clone();
                };
                let Some(konst) = wash_konst_index(stage) else {
                    return material.clone();
                };
                let mut washed = material.clone();
                washed.tev_k_colors[konst] = [wash_threshold; 4];
                washed
            })
            .collect();
        let mut base = vec![None; CANVAS * CANVAS];
        let mut goop_uv: Vec<Option<[f32; 2]>> = vec![None; CANVAS * CANVAS];
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
            // The goop rides the actor's own authored set. On a wired
            // actor, a surface without that set carries no goop at all --
            // HamuKuri's wash lives on his cap material alone. Only an actor
            // that authored nothing is front projected.
            let wired = triangle.mask_tex_coords.or_else(|| {
                authored_coord
                    .and_then(|coord| triangle.tex_coord_sets.get(coord).copied().flatten())
            });
            let uv = match (wired, actor_authored) {
                (Some(set), _) => Some(set),
                (None, true) => None,
                (None, false) => Some([
                    preview.front_uv[index * 3],
                    preview.front_uv[index * 3 + 1],
                    preview.front_uv[index * 3 + 2],
                ]),
            };
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
                    goop_uv[slot] = uv.map(|uv| {
                        [
                            uv[0][0] * w2 + uv[1][0] * w1 + uv[2][0] * w0,
                            uv[0][1] * w2 + uv[1][1] * w1 + uv[2][1] * w0,
                        ]
                    });

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
                        .and_then(|index| washed_materials.get(index))
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
                                                triangle
                                                    .tex_coord_sets
                                                    .get(index)
                                                    .copied()
                                                    .flatten()
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
        let body_slot = self.body_uv_slot();
        let body_texture = self
            .body_texture_index()
            .and_then(|index| preview.geometry.textures.get(index));

        let authored = (self.mask_uv_layer == MaskUvLayer::Goop)
            .then(|| self.authored_mask())
            .flatten();
        // A layer this tool bakes stores its coordinate in the space the front
        // projection produced it in, where V climbs with the screen; a retail
        // or pool map stores image space, where V falls. Drawing both without
        // a flip stands our own bakes on their heads while leaving HamuKuri
        // and BossGesso upright, which is what baking appeared to break.
        let projected = authored
            .as_ref()
            .is_some_and(|(_, name)| name.as_deref() == Some(GOOP_LAYER_TEXTURE));
        let authored_background = authored.map(|(texture, _)| texture);
        if let Some(texture) = authored_background {
            // The map the wash actually reads, in its own orientation.
            for y in 0..CANVAS {
                for x in 0..CANVAS {
                    let u = x as f32 / (CANVAS - 1) as f32;
                    let row = y as f32 / (CANVAS - 1) as f32;
                    let v = if projected { 1.0 - row } else { row };
                    let value = texture.value(u, v);
                    let shade = (value as f32 * 0.6) as u8;
                    pixels[y * CANVAS + x] = [shade, shade, shade.saturating_add(14), 255];
                }
            }
        } else if let (Some((size, values)), MaskUvLayer::Goop) = (&mask, self.mask_uv_layer) {
            for y in 0..CANVAS {
                for x in 0..CANVAS {
                    let u = x as f32 / (CANVAS - 1) as f32;
                    let v = 1.0 - y as f32 / (CANVAS - 1) as f32;
                    let value = sample_mask_bilinear(values, *size, u, v);
                    let shade = (value as f32 * 0.6) as u8;
                    pixels[y * CANVAS + x] = [shade, shade, shade.saturating_add(14), 255];
                }
            }
        } else if let Some(texture) = body_texture {
            // Dim, so the atlas over it stays the thing being read.
            for y in 0..CANVAS {
                for x in 0..CANVAS {
                    let u = x as f32 / (CANVAS - 1) as f32;
                    let v = y as f32 / (CANVAS - 1) as f32;
                    let Some(texel) = sample_texture(texture, u, v) else {
                        continue;
                    };
                    pixels[y * CANVAS + x] = [
                        (texel[0] as f32 * 0.45) as u8,
                        (texel[1] as f32 * 0.45) as u8,
                        (texel[2] as f32 * 0.45) as u8,
                        255,
                    ];
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
        // A model that authored its goop UV is inspected through it, in the
        // image space its mask ships in; the projection is only for models
        // with no authored layer.
        let authored_coord = self.authored_goop_coord();
        let authored_set = |triangle: &sms_formats::J3dTriangle| {
            triangle.mask_tex_coords.or_else(|| {
                authored_coord
                    .and_then(|coord| triangle.tex_coord_sets.get(coord).copied().flatten())
            })
        };
        let authored_layout = self.mask_uv_layer == MaskUvLayer::Goop
            && self.authored_mask().is_some()
            && preview
                .geometry
                .triangles
                .iter()
                .any(|triangle| authored_set(triangle).is_some());
        if authored_layout {
            for triangle in &preview.geometry.triangles {
                let Some(uv) = authored_set(triangle) else {
                    continue;
                };
                let screen: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                    let row = uv[corner][1].clamp(0.0, 1.0);
                    [
                        uv[corner][0].clamp(0.0, 1.0) * (CANVAS - 1) as f32,
                        if projected { 1.0 - row } else { row } * (CANVAS - 1) as f32,
                    ]
                });
                for corner in 0..3 {
                    line(screen[corner], screen[(corner + 1) % 3]);
                }
            }
        }
        for (index, triangle) in preview.geometry.triangles.iter().enumerate() {
            if authored_layout {
                break;
            }
            let uv: [[f32; 2]; 3] = match self.mask_uv_layer {
                MaskUvLayer::Goop => {
                    std::array::from_fn(|corner| preview.front_uv[index * 3 + corner])
                }
                MaskUvLayer::Body => {
                    match body_slot.and_then(|slot| triangle.tex_coord_sets[slot]) {
                        Some(set) => set,
                        None => continue,
                    }
                }
            };
            let screen: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                let row = uv[corner][1].clamp(0.0, 1.0);
                // The goop tab draws the front projection, which is screen
                // space; the body tab draws a set the model stores, which is
                // image space. Flipping both put the body atlas on its head.
                let row = match self.mask_uv_layer {
                    MaskUvLayer::Goop => 1.0 - row,
                    MaskUvLayer::Body => row,
                };
                [
                    uv[corner][0].clamp(0.0, 1.0) * (CANVAS - 1) as f32,
                    row * (CANVAS - 1) as f32,
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
    /// Records whether this actor's archive edit is stored, and whether
    /// reading its model back returns one that carries a wash.
    ///
    /// Worked out once when the actor loads. It parses the model, which is far
    /// too much to repeat for every frame the panel is drawn.
    fn refresh_mask_edit_state(&mut self, choice: &MaskActorChoice) {
        let Some(document) = self.document.as_ref() else {
            self.mask_edit_state = String::new();
            return;
        };
        let Some(raw) = document.archive_resource_path_for_asset(&choice.model_path) else {
            self.mask_edit_state = "this actor has no archive path".to_string();
            return;
        };
        let stored = document
            .archive_edits
            .models
            .iter()
            .any(|edit| edit.raw_resource_path == raw);
        let reads_back = document
            .read_asset_bytes(&choice.model_path)
            .ok()
            .and_then(|bytes| sms_formats::J3dFile::parse(&bytes).ok())
            .and_then(|model| {
                model
                    .geometry_preview_with_loader_flags(choice.load_flags)
                    .ok()
            })
            .is_some_and(|geometry| {
                geometry
                    .materials
                    .iter()
                    .any(|material| material.tev_stages.iter().any(stage_is_wash_comparison))
            });
        self.mask_edit_state = format!(
            "edit stored: {}   model reads back washable: {}",
            if stored { "yes" } else { "no" },
            if reads_back { "yes" } else { "no" }
        );
    }

    /// Isolates the chosen actor out of the stage preview and hands it to the
    /// stage viewport's renderer.
    ///
    /// The actor is already in the stage's preview, materials, textures and
    /// all, so there is nothing to rebuild: keeping only the triangles of its
    /// model leaves exactly what the stage draws for it.
    fn rebuild_mask_gpu_scene(
        &mut self,
        object_id: &str,
        position: [f32; 3],
        geometry: &sms_formats::J3dGeometryPreview,
    ) {
        self.mask_gpu_scene = None;
        self.mask_gpu_bounds = None;
        self.mask_gpu_triangles.clear();
        self.mask_gpu_textures.clear();
        self.mask_gpu_preview = None;
        self.mask_wash_materials.clear();
        self.mask_wash_konst = None;
        let Some(target_format) = self.gpu_target_format else {
            return;
        };
        let Some(preview) = self.model_preview.as_ref() else {
            return;
        };
        // Manager-spawned enemies can register their model under a key that is
        // not the placed object's id. The object still stands somewhere, so
        // fall back to the model whose geometry is nearest that spot.
        let model_index = preview
            .object_model_indices
            .get(object_id)
            .copied()
            .or_else(|| {
                let mut bounds: std::collections::BTreeMap<usize, ([f32; 3], [f32; 3])> =
                    std::collections::BTreeMap::new();
                for triangle in &preview.triangles {
                    // The map terrain and the sky contain every placed
                    // object; neither can be the object's own model. Without
                    // this, HamuKuri resolved to a slab of the map.
                    if preview
                        .goop_surface_model_indices
                        .contains(&triangle.model_index)
                        || triangle.render_layer != PreviewRenderLayer::Main
                    {
                        continue;
                    }
                    let entry = bounds
                        .entry(triangle.model_index)
                        .or_insert(([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]));
                    for vertex in triangle.vertices {
                        for (axis, component) in vertex.iter().enumerate() {
                            entry.0[axis] = entry.0[axis].min(*component);
                            entry.1[axis] = entry.1[axis].max(*component);
                        }
                    }
                }
                // Only a model the object actually stands inside can be its
                // own. Taking the nearest without that bound handed HamuKuri
                // BossPakkun's model when his own was not in the preview.
                bounds
                    .into_iter()
                    .filter_map(|(index, (min, max))| {
                        let margin = (0..3)
                            .map(|axis| max[axis] - min[axis])
                            .fold(0.0f32, f32::max)
                            .mul_add(0.1, 50.0);
                        let inside = (0..3).all(|axis| {
                            position[axis] >= min[axis] - margin
                                && position[axis] <= max[axis] + margin
                        });
                        let distance = (0..3)
                            .map(|axis| {
                                let centre = (min[axis] + max[axis]) * 0.5;
                                (centre - position[axis]).powi(2)
                            })
                            .sum::<f32>()
                            .sqrt();
                        inside.then_some((index, distance))
                    })
                    .min_by(|left, right| left.1.total_cmp(&right.1))
                    .map(|(index, _)| index)
            });
        // The stage instance can only carry the pose when it is the same walk of
        // the same triangles the tool posed. It is not always: an actor whose
        // stage model index gathers more than the body -- BossPakkun brings its
        // tornado and pollution balls along -- counts differently, and pairing
        // two walks of different lengths would corrupt the geometry. Those
        // actors draw from the tool's own posed preview instead, so the pose
        // applies to every actor rather than only the single-model ones.
        let stage_carries_pose = model_index.is_some_and(|model_index| {
            preview
                .triangles
                .iter()
                .filter(|triangle| triangle.model_index == model_index)
                .count()
                == geometry.triangles.len()
        });
        let mut isolated = if let Some(model_index) = model_index.filter(|_| stage_carries_pose) {
            let mut isolated = preview.clone();
            // The stage instance carries the stage's pose baked into its vertices,
            // so the tool drew whatever the idle happened to be doing while the
            // bake projected a pose of its own. Take the positions from the
            // geometry this tool posed, keeping the stage's materials and textures
            // so the shading stays what it was. Same model, same walk, so the
            // triangles line up one for one; if they ever do not, leave the stage's
            // alone rather than pair them up wrongly.
            // This is the tool's own clone: the stage viewport and the runtime are
            // untouched and get whatever the bake writes. Only this viewport
            // follows the pose, which is what the projection is judged against.
            // Counts already agree, so the walks pair one for one: take the tool's
            // posed positions and keep the stage's materials, so only the pose
            // changes and the shading stays as the stage renders it.
            {
                let mut posed = geometry.triangles.iter();
                for triangle in isolated
                    .triangles
                    .iter_mut()
                    .filter(|triangle| triangle.model_index == model_index)
                {
                    if let Some(source) = posed.next() {
                        triangle.vertices = source.vertices;
                    }
                }
            }
            isolated
                .triangles
                .retain(|triangle| triangle.model_index == model_index);
            // The model index covers more than the body: effect meshes ride along
            // -- PoiHana's sleep Zs, billboarded sprites, particle quads. They
            // float away from the actor, so left in they inflate the bounds the
            // camera frames and the span the mask projects across, and the goop
            // paints past the body. Keep the drawn body alone.
            isolated.triangles.retain(|triangle| {
                triangle.billboard.is_none()
                    && triangle.particle_type.is_none()
                    && triangle.render_layer == PreviewRenderLayer::Main
            });
            if isolated.triangles.is_empty() {
                return;
            }
            // Effect geometry that slips the filters still sits away from the
            // dense cluster of body triangles; trim by distance from it.
            let centroids: Vec<[f32; 3]> = isolated
                .triangles
                .iter()
                .map(|triangle| {
                    std::array::from_fn(|axis| {
                        (triangle.vertices[0][axis]
                            + triangle.vertices[1][axis]
                            + triangle.vertices[2][axis])
                            / 3.0
                    })
                })
                .collect();
            let mean: [f32; 3] = std::array::from_fn(|axis| {
                centroids.iter().map(|centroid| centroid[axis]).sum::<f32>()
                    / centroids.len() as f32
            });
            let distances: Vec<f32> = centroids
                .iter()
                .map(|centroid| {
                    (0..3)
                        .map(|axis| (centroid[axis] - mean[axis]).powi(2))
                        .sum::<f32>()
                        .sqrt()
                })
                .collect();
            let mut sorted = distances.clone();
            sorted.sort_by(f32::total_cmp);
            let median = sorted[sorted.len() / 2].max(f32::EPSILON);
            let mut keep = distances
                .into_iter()
                .map(|distance| distance <= median * 4.0);
            isolated.triangles.retain(|_| keep.next().unwrap_or(true));
            isolated
        } else {
            // Nothing of this actor stands in the stage preview. Render the
            // model the tool itself loaded, in its own space.
            build_mask_model_preview(geometry)
        };
        if isolated.triangles.is_empty() {
            return;
        }
        // The stage preview holds the actor at its placed position, which the
        // model's own bounds know nothing about. Aiming the camera with the
        // local bounds points it at the stage's origin instead of the actor.
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in isolated
            .triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
        {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }
        let center = std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5);
        let radius = (0..3)
            .map(|axis| (max[axis] - min[axis]) * 0.5)
            .fold(0.0f32, f32::max)
            .max(f32::EPSILON);
        self.mask_gpu_bounds = Some((center, radius));
        self.mask_gpu_triangles = isolated.triangles.clone();
        self.mask_gpu_textures = isolated.textures.clone();
        // The stage animates some models at draw time. The coating works on
        // the triangles as they stand, so strip the animation bindings rather
        // than letting the renderer walk the actor out from under it.
        isolated.animated_models.clear();
        isolated.animated_flags.clear();
        isolated.rotating_models.clear();
        isolated.level_transform_models.clear();
        // The wash is the comparison's konst. Driving it dissolves the baked
        // goop itself -- the way the game drives it with hit points -- rather
        // than painting a second coating over the top of it.
        // Only the materials this actor's own triangles reference count: the
        // isolated clone keeps the whole stage's material list, and another
        // actor's wash comparison must not mark this one as authored --
        // BossPakkun lost his borrowed coating to BossGesso's materials.
        let used_materials: std::collections::BTreeSet<usize> = isolated
            .triangles
            .iter()
            .filter_map(|triangle| triangle.material_index)
            .collect();
        // A comparison can take its level from a constant rather than a
        // register -- HamuKuri's reads GX's fixed one-half, which is why its
        // goop is permanent in retail and why writing any register leaves it
        // untouched. Point it at a spare register in this clone so the renderer
        // can wash it like any other. Only the preview is affected; what ships
        // still carries the constant unless the layer is rebaked washable.
        for index in used_materials.iter().copied() {
            let Some(material) = isolated.materials.get_mut(index) else {
                continue;
            };
            let claimed: Vec<usize> = material
                .tev_stages
                .iter()
                .filter_map(wash_konst_index)
                .collect();
            let Some(spare) = (0..4).find(|register| !claimed.contains(register)) else {
                continue;
            };
            for stage in material.tev_stages.iter_mut() {
                if stage.color_op < 8 && stage.alpha_op < 8 {
                    continue;
                }
                if wash_konst_index(stage).is_some() {
                    continue;
                }
                // K<n>'s alpha, broadcast into the colour channel, which is how
                // a wash comparison reads its level.
                stage.konst_color = 0x1c + spare as u8;
                stage.konst_alpha = 0x1c + spare as u8;
            }
        }
        self.mask_wash_materials = isolated
            .materials
            .iter()
            .enumerate()
            .filter(|(index, _)| used_materials.contains(index))
            .filter_map(|(index, material)| {
                // Only materials whose comparison actually reads a konst can be
                // washed by moving one. A material without one keeps its
                // coating whatever the slider does, so leaving it out is what
                // lets the tool draw its own preview over the top instead.
                material.tev_stages.iter().find_map(|stage| {
                    if !stage_is_wash_comparison(stage) {
                        return None;
                    }
                    wash_konst_index(stage).map(|konst| (index, konst))
                })
            })
            .collect();
        // An actor whose coating cannot be driven -- HamuKuri's goop is
        // permanent in retail, its comparison reads no konst -- keeps drawing
        // that coating whatever the tool previews, so the preview lands on top
        // of it and both show at once. Take the model's own coating out of the
        // preview scene, leaving the tool's the only one drawn.
        if self.mask_wash_materials.is_empty() {
            if let Some(index) = isolated
                .triangles
                .iter()
                .find_map(|triangle| triangle.mask_texture_index)
            {
                if let Some(texture) = isolated.textures.get_mut(index) {
                    // The coating shows where the mask does not exceed the
                    // comparison's level, so a mask at the top of its range
                    // shows none of it.
                    for pixel in texture.image.pixels.iter_mut() {
                        *pixel = egui::Color32::WHITE;
                    }
                }
            }
        }
        self.mask_gpu_scene = Some(gpu_viewport::GpuViewportScene::from_preview(
            &isolated,
            target_format,
        ));
        self.mask_gpu_preview = Some(isolated);
        self.push_mask_wash();
    }

    /// Pushes the wash slider into the renderer's materials.
    ///
    /// Full coverage puts the konst at zero, so every masked texel passes the
    /// comparison and the actor wears its authored goop whole. Sliding down
    /// raises the konst and the coating dissolves in order of the mask's
    /// values -- the crisp recede the game gets from the same konst.
    fn push_mask_wash(&mut self) {
        if self.mask_wash_materials.is_empty() {
            return;
        }
        let threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.mask_wash_konst == Some(threshold) {
            return;
        }
        let mut indices = Vec::new();
        if let Some(preview) = self.mask_gpu_preview.as_mut() {
            for (material_index, konst_index) in &self.mask_wash_materials {
                if let Some(material) = preview.materials.get_mut(*material_index) {
                    material.tev_k_colors[*konst_index] = [threshold; 4];
                    indices.push(*material_index);
                }
            }
        }
        self.mask_wash_konst = Some(threshold);
        if let (Some(scene), Some(preview)) =
            (self.mask_gpu_scene.as_ref(), self.mask_gpu_preview.as_ref())
        {
            scene.update_materials(preview, &indices);
        }
    }

    /// A camera matching the one the goop pass rasterizes with.
    ///
    /// That pass projects orthographically, so the perspective camera is placed
    /// far enough back for the divergence to fall under a pixel and given the
    /// focal length that reproduces its scale. Both then frame the actor
    /// identically, and the goop overlay lands where the model is.
    fn mask_gpu_frame(&self, rect: egui::Rect) -> Option<gpu_viewport::GpuViewportFrame> {
        let (center, radius) = self.mask_gpu_bounds?;
        let side = rect.width().min(rect.height()).max(1.0);
        // The same basis the stage camera builds, so the winding the renderer
        // culls against is the winding it expects. A mirrored basis flips
        // every triangle and the model comes out inside out.
        let (sin_yaw, cos_yaw) = self.mask_yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.mask_pitch.sin_cos();
        let forward = [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch];
        let right = [-cos_yaw, 0.0, sin_yaw];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let distance = radius * 8.0;
        let camera_position = std::array::from_fn(|axis| center[axis] - forward[axis] * distance);
        let lighting = self
            .document
            .as_ref()
            .and_then(|document| document.lighting.object_lighting());
        Some(gpu_viewport::GpuViewportFrame {
            camera_position,
            right,
            up,
            forward,
            // The goop pass scales by 0.45 of the radius across the canvas.
            focal: distance * 0.45 * side / radius,
            viewport_size: [rect.width().max(1.0), rect.height().max(1.0)],
            viewport_pan: [0.0; 2],
            near: VIEWPORT_NEAR_CLIP,
            animation_seconds: self.animation_started_at.elapsed().as_secs_f32(),
            light_position: lighting
                .map(|lighting| lighting.position)
                .unwrap_or([200_000.0, 500_000.0, 200_000.0]),
            light_color: lighting
                .map(|lighting| gpu_viewport::color_u8_to_f32(lighting.color))
                .unwrap_or([1.0; 4]),
            ambient_color: lighting.map(|lighting| gpu_viewport::color_u8_to_f32(lighting.ambient)),
            object_light_position: lighting
                .map(|lighting| lighting.position)
                .unwrap_or([200_000.0, 500_000.0, 200_000.0]),
            object_light_color: lighting
                .map(|lighting| gpu_viewport::color_u8_to_f32(lighting.color))
                .unwrap_or([1.0; 4]),
            object_ambient_color: lighting
                .map(|lighting| gpu_viewport::color_u8_to_f32(lighting.ambient)),
            show_grid: false,
            death_barrier_y: None,
        })
    }

    /// Coverage and goop UV for the actor as the renderer draws it.
    ///
    /// This runs over the stage's own triangles with the camera
    /// [`Self::mask_gpu_frame`] builds, so the coating and the model agree
    /// pixel for pixel. It resolves no colour: the renderer draws the model,
    /// and this says only where the coating sits.
    ///
    /// A model that carries an authored goop UV is coated through that. Only a
    /// model without one is front-projected, which is what retail authored for
    /// StayPakkun and what [`front_projection_bounds`] reproduces.
    fn rasterize_goop(&self) -> Option<Vec<Option<GoopPixelUv>>> {
        let (center, radius) = self.mask_gpu_bounds?;
        if self.mask_gpu_triangles.is_empty() {
            return None;
        }
        // The same basis the stage camera builds, so the winding the renderer
        // culls against is the winding it expects. A mirrored basis flips
        // every triangle and the model comes out inside out.
        let (sin_yaw, cos_yaw) = self.mask_yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.mask_pitch.sin_cos();
        let forward = [sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch];
        let right = [-cos_yaw, 0.0, sin_yaw];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let distance = radius * 8.0;
        let camera: [f32; 3] = std::array::from_fn(|axis| center[axis] - forward[axis] * distance);
        // The focal length the frame uses, carried into canvas pixels. The
        // viewport's own width cancels, so the two framings agree at any size.
        let scale = distance * 0.45 * CANVAS as f32 / radius;
        let half = CANVAS as f32 * 0.5;

        // Where the model has no authored goop UV, project it from the front,
        // measured over the actor as the stage holds it.
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for vertex in self
            .mask_gpu_triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
        {
            for axis in 0..2 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }
        let span = [max[0] - min[0], max[1] - min[1]];

        // The mask the authored coordinates sample, from the actor's own
        // model, for triangles whose stage copy lost the binding. The wash
        // comparison's own binding stands in where no preview path resolved
        // one at all.
        let model_mask = self.model_mask_texture().or_else(|| {
            self.authored_goop_binding().and_then(|(texture_index, _)| {
                self.mask_preview
                    .as_ref()
                    .and_then(|preview| preview.geometry.textures.get(texture_index))
            })
        });
        let authored_coord = self.authored_goop_coord();
        // Whether this actor authored a goop layer at all. On one that did, a
        // surface without the layer stays clean rather than front projected.
        let actor_authored = model_mask.is_some();
        let mut uv = vec![None; CANVAS * CANVAS];
        let mut depth = vec![f32::INFINITY; CANVAS * CANVAS];
        for triangle in &self.mask_gpu_triangles {
            let mut screen = [[0.0f32; 3]; 3];
            let mut behind = false;
            for (corner, point) in screen.iter_mut().zip(triangle.vertices) {
                let relative: [f32; 3] = std::array::from_fn(|axis| point[axis] - camera[axis]);
                let x = (0..3).map(|axis| relative[axis] * right[axis]).sum::<f32>();
                let y = (0..3).map(|axis| relative[axis] * up[axis]).sum::<f32>();
                let z = (0..3)
                    .map(|axis| relative[axis] * forward[axis])
                    .sum::<f32>();
                if z <= 1.0 {
                    behind = true;
                    break;
                }
                *corner = [half + x * scale / z, half - y * scale / z, z];
            }
            if behind {
                continue;
            }
            let front: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                std::array::from_fn(|axis| {
                    if span[axis] > f32::EPSILON {
                        ((triangle.vertices[corner][axis] - min[axis]) / span[axis]).clamp(0.0, 1.0)
                    } else {
                        0.5
                    }
                })
            });
            // A triangle carrying the authored UV is coated through it. The
            // mask texture comes from the stage copy when the binding
            // survived its load path, and from the actor's own model when it
            // did not.
            let authored_set = triangle.mask_tex_coords.or_else(|| {
                authored_coord
                    .and_then(|coord| triangle.tex_coord_sets.get(coord).copied().flatten())
            });
            let authored = authored_set.and_then(|set| {
                triangle
                    .mask_texture_index
                    .and_then(|index| self.mask_gpu_textures.get(index))
                    .map(AuthoredMaskTexture::Stage)
                    .or_else(|| model_mask.map(AuthoredMaskTexture::Model))
                    .map(|texture| (set, texture))
            });

            // The renderer cuts shapes out of quads with the texture's
            // alpha. Run the same test here, or the coating covers texels the
            // model never drew and pokes past the silhouette.
            let cutout = triangle.alpha_compare.and_then(|compare| {
                let texture = self.mask_gpu_textures.get(triangle.texture_index?)?;
                let body = triangle.tex_coords.or(triangle.tex_coord_sets[0])?;
                Some((compare, texture, body))
            });

            let area = (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                - (screen[2][0] - screen[0][0]) * (screen[1][1] - screen[0][1]);
            if area.abs() < f32::EPSILON {
                continue;
            }
            let bound = |pick: &dyn Fn([f32; 3]) -> f32, low: bool| -> usize {
                let values = screen.iter().map(|corner| pick(*corner));
                if low {
                    values.fold(f32::INFINITY, f32::min).floor().max(0.0) as usize
                } else {
                    (values.fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize)
                        .min(CANVAS - 1)
                }
            };
            let min_x = bound(&|corner| corner[0], true);
            let max_x = bound(&|corner| corner[0], false);
            let min_y = bound(&|corner| corner[1], true);
            let max_y = bound(&|corner| corner[1], false);

            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    let edge = |a: [f32; 3], b: [f32; 3]| {
                        (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])
                    };
                    let w0 = edge(screen[0], screen[1]) / area;
                    let w1 = edge(screen[1], screen[2]) / area;
                    let w2 = edge(screen[2], screen[0]) / area;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = screen[0][2] * w1 + screen[1][2] * w2 + screen[2][2] * w0;
                    let slot = y * CANVAS + x;
                    // Nearest wins, so the coating sits on the surface facing
                    // the camera rather than on the far side of the model.
                    if z >= depth[slot] {
                        continue;
                    }
                    depth[slot] = z;
                    let at = |set: [[f32; 2]; 3]| {
                        [
                            set[0][0] * w1 + set[1][0] * w2 + set[2][0] * w0,
                            set[0][1] * w1 + set[1][1] * w2 + set[2][1] * w0,
                        ]
                    };
                    if let Some((compare, texture, body)) = &cutout {
                        let [u, v] = at(*body);
                        let alpha = sample_preview_alpha(texture, u, v);
                        if !alpha_compare_passes(compare, alpha) {
                            continue;
                        }
                    }
                    uv[slot] = match (authored, actor_authored) {
                        (Some((set, texture)), _) => {
                            let coat_uv = at(set);
                            Some((
                                coat_uv,
                                at(front),
                                Some(texture.value(coat_uv[0], coat_uv[1])),
                            ))
                        }
                        // A wired actor's un-wired surface carries no goop;
                        // the triangle still wrote depth, so it occludes any
                        // coating behind it.
                        (None, true) => None,
                        (None, false) => Some((at(front), at(front), None)),
                    };
                }
            }
        }
        Some(uv)
    }

    /// The coating alone, transparent everywhere it does not show, so it can be
    /// laid over a model the GPU drew.
    fn mask_goop_overlay_image(&self) -> Option<egui::ColorImage> {
        let goop_uv = self.rasterize_goop()?;
        let threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        // An actor that authored a wash mask is judged exactly the way the
        // game judges it: its own mask, through its own UV, against the
        // threshold. The borrowed gradient only drives actors that never
        // authored one.
        let borrowed = self.active_mask();
        let mut pixels = Vec::with_capacity(CANVAS * CANVAS * 4);
        for coated in goop_uv {
            // Coverage comes from the same pass, so the coating stops at the
            // model's silhouette rather than floating over the background.
            let shows = coated.is_some_and(|(_, [front_u, front_v], own_mask)| match own_mask {
                Some(value) => goop_is_visible(self.mask_reading(value), threshold),
                None => borrowed.as_ref().is_some_and(|(size, values)| {
                    goop_is_visible(
                        self.mask_reading(sample_mask_bilinear(values, *size, front_u, front_v)),
                        threshold,
                    )
                }),
            });
            match (shows, coated) {
                (true, Some(([u, v], _, _))) => pixels.extend_from_slice(&self.goop_colour(u, v)),
                _ => pixels.extend_from_slice(&[0, 0, 0, 0]),
            }
        }
        Some(egui::ColorImage::from_rgba_unmultiplied(
            [CANVAS, CANVAS],
            &pixels,
        ))
    }

    fn mask_preview_image(&self) -> Option<egui::ColorImage> {
        if self.mask_view == MaskView::Uv {
            return self.render_uv_inspector();
        }
        let (base, goop_uv) = self.rasterize_model()?;
        let threshold = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mask = self.active_mask();
        let authored = self.authored_mask();
        let mut pixels = Vec::with_capacity(CANVAS * CANVAS * 4);
        for slot in 0..CANVAS * CANVAS {
            let Some(base_colour) = base[slot] else {
                pixels.extend_from_slice(&[24, 24, 30, 255]);
                continue;
            };
            let mut colour = base_colour;
            if self.mask_uv_layer == MaskUvLayer::Goop && authored.is_none() {
                if let Some([u, v]) = goop_uv[slot] {
                    let mask_value = match &authored {
                        // The actor's own mask, in its own image space.
                        Some((texture, _)) => Some(texture.value(u, v)),
                        None => mask
                            .as_ref()
                            .map(|(size, values)| sample_mask_bilinear(values, *size, u, v)),
                    };
                    if mask_value
                        .is_some_and(|value| goop_is_visible(self.mask_reading(value), threshold))
                    {
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

    /// The actor currently selected in the Mask Tool.
    /// The goop style the actor's own layer was given, as an index into the
    /// retail template catalog.
    ///
    /// The goop tool styles a layer -- chocolate, oil -- binds managers to it,
    /// and gives each layer its own copy of the actor's folder suffixed with the
    /// layer number. So the actor already names its layer: the copy it draws
    /// from is `hamukuri00`, and 00 is the layer whose style it wears. Defaulting
    /// the colour to a generated rainbow instead offers a coating the actor can
    /// never have.
    fn actor_layer_style(&self, choice: &MaskActorChoice) -> Option<usize> {
        let path = choice.model_path.replace(char::from(92), "/");
        let (directory, _) = path.rsplit_once('/')?;
        let folder = directory.rsplit('/').next()?;
        let digits = folder.get(folder.len().checked_sub(2)?..)?;
        if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let layer: usize = digits.parse().ok()?;
        let style = self
            .document
            .as_ref()?
            .goop_authoring
            .as_ref()?
            .layers
            .get(layer)?
            .style_source
            .as_ref()?;
        self.retail_goop_templates.iter().position(|template| {
            template.stage_id == style.stage_id && template.layer_index == style.layer_index
        })
    }

    /// The pose a goop layer is authored in: the model's rest pose, or its idle
    /// animation held at the frame the slider names.
    ///
    /// A goop coordinate is a projection over posed vertices, so the pose is
    /// part of the result -- the same actor bent differently unwraps
    /// differently. The preview and the bake read this one choice so the
    /// coating lands where it was shown rather than in whatever pose the stage
    /// happened to be drawing.
    fn mask_pose_animation(
        &self,
        choice: &MaskActorChoice,
    ) -> Option<(sms_formats::J3dJointAnimation, f32)> {
        if self.mask_bake_tpose {
            return None;
        }
        let document = self.document.as_ref()?;
        let object = document
            .objects
            .iter()
            .find(|object| object.id == choice.object_id)?;
        let (animation, rate) =
            crate::preview_assets::starting_joint_animation(document, object, &choice.model_path)?;
        let frame = animation.playback_frame(self.mask_idle_frame * rate);
        Some((animation, frame))
    }

    /// The stored set that carries the body's own UV: the widest one that is
    /// not the goop set.
    ///
    /// Which set that is cannot be assumed. PoiHana keeps only 70 of its 476
    /// triangles in slot 0 -- the eye and the mouth -- and draws its body from
    /// a generated coordinate, so slot 0 is not "the body UV" in general.
    fn body_uv_slot(&self) -> Option<usize> {
        let preview = self.mask_preview.as_ref()?;
        let goop = self.authored_goop_coord();
        let coverage = |slot: usize| {
            preview
                .geometry
                .triangles
                .iter()
                .filter(|triangle| triangle.tex_coord_sets[slot].is_some())
                .count()
        };
        (0..8)
            .filter(|slot| Some(*slot) != goop)
            .max_by_key(|slot| coverage(*slot))
            .filter(|slot| coverage(*slot) > 0)
    }

    /// The texture the body slot feeds.
    ///
    /// A material generates from `GX_TG_TEX0 + slot`; the stage that reads that
    /// generator names the map.
    fn body_texture_index(&self) -> Option<usize> {
        let preview = self.mask_preview.as_ref()?;
        let slot = self.body_uv_slot()?;
        let source = slot as u8 + 4;
        preview.geometry.materials.iter().find_map(|material| {
            let generator = material
                .tex_gens
                .iter()
                .position(|generator| generator.source == source)?;
            let map = material.tev_stages.iter().find_map(|stage| {
                (stage.order.tex_coord == Some(generator as u8))
                    .then_some(stage.order.tex_map)
                    .flatten()
            })?;
            material
                .texture_indices
                .get(map as usize)
                .copied()
                .flatten()
        })
    }

    fn selected_mask_choice(&self) -> Option<MaskActorChoice> {
        let selected = self.mask_selected_actor.as_deref()?;
        self.mask_actor_choices()
            .into_iter()
            .find(|choice| choice.object_id == selected)
    }

    /// Writes the actor as glTF, carrying both UV sets and the textures that
    /// ride each.
    ///
    /// Not wired yet. The intent is a file that opens in Blender ready to
    /// paint: `TEXCOORD_0` with the body texture on it, `TEXCOORD_1` with the
    /// goop mask on it -- the model's authored goop set where it has one, the
    /// front projection where it does not -- so the unwrap being painted is
    /// the unwrap the wash reads.
    fn export_mask_gltf(&mut self) {
        let Some(preview) = self.mask_preview.as_ref() else {
            self.log
                .push("Select an actor before exporting.".to_string());
            return;
        };
        let stem = self
            .selected_mask_choice()
            .and_then(|choice| {
                let path = choice.model_path.replace(char::from(92), "/");
                let mut parts = path.rsplit('/');
                parts.next();
                parts.next().map(str::to_string)
            })
            .unwrap_or_else(|| "actor".to_string());
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export glTF")
            .add_filter("glTF", &["gltf"])
            .set_file_name(format!("{stem}.gltf"))
            .save_file()
        else {
            return;
        };

        // The goop set the wash actually reads, or the projection where the
        // model has none, so what opens in Blender is the unwrap the wash
        // reads rather than a second guess at one.
        let goop_coord = self.authored_goop_coord();
        let body_slot = self.body_uv_slot();
        // glTF and PNG are both image space, where V falls. A body UV is
        // stored that way already, which is why it came out right while the
        // goop UV did not: a layer this tool bakes stores a projection, where
        // V climbs, and so does the front projection used where a model has no
        // layer at all. Flip those on the way out, and flip the map with them
        // so the two still agree.
        let goop_is_projected = match self.authored_mask() {
            Some((_, name)) => name.as_deref() == Some(GOOP_LAYER_TEXTURE),
            None => true,
        };
        let body_texture = self
            .body_texture_index()
            .and_then(|index| preview.geometry.textures.get(index));
        let goop_texture = self
            .authored_goop_binding()
            .and_then(|(index, _)| preview.geometry.textures.get(index));

        let mut positions: Vec<u8> = Vec::new();
        let mut normals: Vec<u8> = Vec::new();
        let mut coordinates: Vec<u8> = Vec::new();
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut count = 0usize;
        for (index, triangle) in preview.geometry.triangles.iter().enumerate() {
            let body = body_slot
                .and_then(|slot| triangle.tex_coord_sets[slot])
                .unwrap_or([[0.0, 0.0]; 3]);
            let goop = triangle
                .mask_tex_coords
                .or_else(|| goop_coord.and_then(|coord| triangle.tex_coord_sets[coord]))
                .unwrap_or_else(|| {
                    std::array::from_fn(|corner| preview.front_uv[index * 3 + corner])
                });
            let goop: [[f32; 2]; 3] = std::array::from_fn(|corner| {
                if goop_is_projected {
                    [goop[corner][0], 1.0 - goop[corner][1]]
                } else {
                    goop[corner]
                }
            });
            // A face normal stands in where the model carries none, so the mesh
            // still shades once it is open.
            let fallback = face_normal(triangle.vertices);
            for corner in 0..3 {
                let vertex = triangle.vertices[corner];
                for axis in 0..3 {
                    min[axis] = min[axis].min(vertex[axis]);
                    max[axis] = max[axis].max(vertex[axis]);
                    positions.extend_from_slice(&vertex[axis].to_le_bytes());
                }
                let normal = triangle
                    .normals
                    .map(|normals| normals[corner])
                    .unwrap_or(fallback);
                for value in normal {
                    normals.extend_from_slice(&value.to_le_bytes());
                }
                for value in body[corner] {
                    coordinates.extend_from_slice(&value.to_le_bytes());
                }
                for value in goop[corner] {
                    coordinates.extend_from_slice(&value.to_le_bytes());
                }
                count += 1;
            }
        }
        if count == 0 {
            self.log
                .push("That actor has no geometry to export.".to_string());
            return;
        }

        let directory = path.parent().map(|parent| parent.to_path_buf());
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or(stem);
        let beside = |suffix: &str| {
            let file = format!("{name}{suffix}");
            match &directory {
                Some(directory) => directory.join(file),
                None => std::path::PathBuf::from(file),
            }
        };

        let write_texture = |texture: Option<&sms_formats::J3dTexturePreview>,
                             suffix: &str,
                             channel: Option<usize>,
                             flip: bool|
         -> Option<String> {
            let texture = texture?;
            // A layer this tool bakes carries its coating in the colour
            // channels and its mask in alpha. Exporting the bytes as they sit
            // hands Blender a rainbow with the mask hidden inside it; the mask
            // is the thing being painted, so it is written as an image in its
            // own right and the coating goes to a file of its own.
            let pixels = match channel {
                Some(channel) => texture
                    .rgba
                    .chunks_exact(4)
                    .flat_map(|texel| {
                        let value = texel[channel];
                        [value, value, value, 255]
                    })
                    .collect(),
                None => texture
                    .rgba
                    .chunks_exact(4)
                    .flat_map(|texel| [texel[0], texel[1], texel[2], 255])
                    .collect(),
            };
            let image = image::RgbaImage::from_raw(
                u32::from(texture.width),
                u32::from(texture.height),
                pixels,
            )?;
            let image = if flip {
                image::imageops::flip_vertical(&image)
            } else {
                image
            };
            image.save(beside(suffix)).ok()?;
            Some(format!("{name}{suffix}"))
        };
        let body_image = write_texture(body_texture, "_body.png", None, false);
        let goop_is_ours = goop_texture.is_some_and(|texture| texture.name == GOOP_LAYER_TEXTURE);
        let goop_image = write_texture(
            goop_texture,
            "_goop.png",
            Some(if goop_is_ours { 3 } else { 0 }),
            goop_is_projected,
        );
        if goop_is_ours {
            write_texture(goop_texture, "_coat.png", None, goop_is_projected);
        }

        // glTF carries no node graph, so the comparison cannot ride in the
        // file. It rides beside it: a script that builds the same test Blender
        // can show live -- coated where the mask loses to the coverage, which
        // is what the TEV stage does per pixel.
        let coat_file = format!("{name}_coat.png");
        let level = self.mask_wash_phase.clamp(0.0, 1.0);
        let invert = self.mask_wash_invert;
        let script = format!(
            r#"# Rebuilds this actor's wash as Blender nodes, so painting the goop mask
# shows what the game draws. Run from Blender's Text Editor with the mesh
# selected, then scrub the "Coverage" value.
#
# Coated where mask <= coverage: the mask is a rank map, not a shape -- each
# texel says how late the goop reaches it.
import bpy, os

HERE = r"{here}"
COVERAGE = {level}
INVERT = {invert}

mesh = bpy.context.object
material = mesh.data.materials[0]
material.use_nodes = True
tree = material.node_tree
tree.nodes.clear()

def image(name, colour):
    node = tree.nodes.new("ShaderNodeTexImage")
    path = os.path.join(HERE, name)
    node.image = bpy.data.images.load(path, check_existing=True) if os.path.exists(path) else None
    if node.image and not colour:
        node.image.colorspace_settings.name = "Non-Color"
    return node

def uv(node, index):
    source = tree.nodes.new("ShaderNodeUVMap")
    layers = mesh.data.uv_layers
    source.uv_map = layers[min(index, len(layers) - 1)].name
    source.location = (node.location[0] - 260, node.location[1] - 70)
    tree.links.new(source.outputs["UV"], node.inputs["Vector"])

body = image("{body}", True)
coat = image("{coat}", True)
mask = image("{mask}", False)
body.location = (-1120, 140)
coat.location = (-1120, 560)
mask.location = (-1120, -300)
uv(body, 0)
uv(coat, 1)
uv(mask, 1)

# Inverting the mask before the test rather than the result after it keeps the
# control a blend: Invert's Fac runs 0..1, so it slides between the two
# directions instead of snapping between them.
invert = tree.nodes.new("ShaderNodeInvert")
invert.label = "Invert"
invert.inputs["Fac"].default_value = 1.0 if INVERT else 0.0
invert.location = (-800, -320)
tree.links.new(mask.outputs["Color"], invert.inputs["Color"])

coverage = tree.nodes.new("ShaderNodeValue")
coverage.label = "Coverage"
coverage.outputs[0].default_value = COVERAGE
coverage.location = (-800, -520)

# Less Than, not Greater Than: with the invert ahead of it, Fac 0 leaves this
# reading exactly what the game reads -- coated where the mask loses to the
# coverage. Comparing the other way against an inverted mask tests
# mask < 1 - coverage, which looks close but is a different threshold.
compare = tree.nodes.new("ShaderNodeMath")
compare.operation = "LESS_THAN"
compare.location = (-540, -360)
tree.links.new(invert.outputs["Color"], compare.inputs[0])
tree.links.new(coverage.outputs[0], compare.inputs[1])

mix = tree.nodes.new("ShaderNodeMix")
mix.data_type = "RGBA"
mix.location = (-240, 260)
tree.links.new(compare.outputs[0], mix.inputs["Factor"])
tree.links.new(body.outputs["Color"], mix.inputs[6])
tree.links.new(coat.outputs["Color"], mix.inputs[7])

# Straight into Surface with no Principled in the way, so the colour reads as
# emission and the coating stays legible while it is being painted.
output = tree.nodes.new("ShaderNodeOutputMaterial")
output.location = (60, 260)
tree.links.new(mix.outputs[2], output.inputs["Surface"])
print("wash rebuilt. Scrub Coverage; Invert's Fac slides between directions.")
"#,
            here = directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().replace(char::from(92), "/"))
                .unwrap_or_default(),
            level = level,
            invert = if invert { "True" } else { "False" },
            body = body_image.clone().unwrap_or_default(),
            coat = coat_file,
            mask = goop_image.clone().unwrap_or_default(),
        );
        if let Err(error) = std::fs::write(beside("_wash.py"), script) {
            self.log
                .push(format!("Could not write the Blender script: {error}"));
        }

        let mut buffer = positions;
        let normals_offset = buffer.len();
        buffer.extend_from_slice(&normals);
        let coordinates_offset = buffer.len();
        buffer.extend_from_slice(&coordinates);
        if let Err(error) = std::fs::write(beside(".bin"), &buffer) {
            self.log
                .push(format!("Could not write the buffer: {error}"));
            return;
        }

        let mut images = Vec::new();
        let mut textures = Vec::new();
        let mut material = serde_json::json!({
            "pbrMetallicRoughness": {"metallicFactor": 0, "roughnessFactor": 1},
            "doubleSided": true,
        });
        if let Some(file) = body_image {
            images.push(serde_json::json!({"uri": file}));
            textures.push(serde_json::json!({"source": images.len() - 1}));
            material["pbrMetallicRoughness"]["baseColorTexture"] =
                serde_json::json!({"index": textures.len() - 1, "texCoord": 0});
        }
        if let Some(file) = goop_image {
            // Bound to the second set, so the file opens with the goop map
            // already sitting on the coordinate the wash reads.
            images.push(serde_json::json!({"uri": file}));
            textures.push(serde_json::json!({"source": images.len() - 1}));
            material["emissiveTexture"] =
                serde_json::json!({"index": textures.len() - 1, "texCoord": 1});
            material["emissiveFactor"] = serde_json::json!([1.0, 1.0, 1.0]);
        }

        let document = serde_json::json!({
            "asset": {"version": "2.0", "generator": "graffito"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "name": name}],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2, "TEXCOORD_1": 3},
                "material": 0,
                "mode": 4,
            }]}],
            "materials": [material],
            "textures": textures,
            "images": images,
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": count, "type": "VEC3",
                 "min": min, "max": max},
                {"bufferView": 1, "componentType": 5126, "count": count, "type": "VEC3"},
                {"bufferView": 2, "byteOffset": 0, "componentType": 5126, "count": count,
                 "type": "VEC2"},
                {"bufferView": 2, "byteOffset": 8, "componentType": 5126, "count": count,
                 "type": "VEC2"},
            ],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": normals_offset, "target": 34962},
                {"buffer": 0, "byteOffset": normals_offset,
                 "byteLength": coordinates_offset - normals_offset, "target": 34962},
                {"buffer": 0, "byteOffset": coordinates_offset,
                 "byteLength": buffer.len() - coordinates_offset, "byteStride": 16,
                 "target": 34962},
            ],
            "buffers": [{"uri": format!("{name}.bin"), "byteLength": buffer.len()}],
        });
        match serde_json::to_vec(&document)
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| error.to_string()))
        {
            Ok(()) => self.log.push(format!(
                "Exported {} triangles to {}. TEXCOORD_0 is the body UV, TEXCOORD_1 the goop \
                 UV the wash reads.",
                count / 3,
                path.display()
            )),
            Err(error) => self.log.push(format!("Could not write the glTF: {error}")),
        }
    }

    /// Takes a re-unwrapped goop UV back out of an edited glTF.
    ///
    /// Not wired yet. The carrier is the glTF's own `TEXCOORD_n`, which keeps
    /// per-vertex correspondence with positions and indices, so the new set
    /// can be matched back onto the model's vertices by position. Re-unwrap
    /// without remodelling and the match is exact.
    fn reimport_mask_uv(&mut self) {
        let Some(choice) = self.selected_mask_choice() else {
            self.log.push("Select an actor first.".to_string());
            return;
        };
        let Some(slot) = self.authored_goop_coord() else {
            self.log.push(
                "This actor has no goop layer to re-unwrap. Bake one first, then export it."
                    .to_string(),
            );
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Reimport Goop UV")
            .add_filter("glTF", &["gltf", "glb"])
            .pick_file()
        else {
            return;
        };
        let (json, buffer) = match read_gltf_document(&path) {
            Ok(document) => document,
            Err(error) => {
                self.log.push(error);
                return;
            }
        };
        let attributes = &json["meshes"][0]["primitives"][0]["attributes"];
        let (Some(position_accessor), Some(goop_accessor)) = (
            attributes["POSITION"].as_u64(),
            attributes["TEXCOORD_1"].as_u64(),
        ) else {
            self.log.push(
                "That mesh carries no TEXCOORD_1. The goop unwrap is the second UV set -- keep \
                 it when exporting from Blender."
                    .to_string(),
            );
            return;
        };
        let positions = match read_gltf_floats(&json, &buffer, position_accessor as usize, 3) {
            Ok(values) => values,
            Err(error) => {
                self.log.push(error);
                return;
            }
        };
        let coordinates = match read_gltf_floats(&json, &buffer, goop_accessor as usize, 2) {
            Ok(values) => values,
            Err(error) => {
                self.log.push(error);
                return;
            }
        };
        if positions.len() != coordinates.len() {
            self.log.push(
                "That mesh has more positions than coordinates, which it should not.".to_string(),
            );
            return;
        }

        // Matched on position rather than on order: Blender renumbers vertices
        // freely, and welding drops some outright, but where a vertex sits is
        // what it was exported as.
        let mut unwrapped: std::collections::HashMap<[i32; 3], [f32; 2]> =
            std::collections::HashMap::with_capacity(positions.len());
        for (position, coordinate) in positions.iter().zip(&coordinates) {
            unwrapped.insert(gltf_position_key(*position), [coordinate[0], coordinate[1]]);
        }

        let (raw_path, bytes) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            let Some(raw_path) = document.archive_resource_path_for_asset(&choice.model_path)
            else {
                self.log.push(
                    "This actor's model lives outside the stage archive, so it cannot be \
                     re-unwrapped here yet."
                        .to_string(),
                );
                return;
            };
            match document.read_asset_bytes(&choice.model_path) {
                Ok(bytes) => (raw_path, bytes),
                Err(error) => {
                    self.log.push(format!("Could not read the model: {error}"));
                    return;
                }
            }
        };
        let mut model = match sms_formats::J3dRebuildDocument::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.log.push(format!("Could not parse the model: {error}"));
                return;
            }
        };
        let parsed = match sms_formats::J3dFile::parse(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.log.push(format!("Could not read the model: {error}"));
                return;
            }
        };
        let posed = self.mask_pose_animation(&choice);
        let matrices = match parsed.preview_draw_matrices(
            choice.load_flags,
            posed.as_ref().map(|(animation, frame)| (animation, *frame)),
            &[],
        ) {
            Ok(matrices) => matrices,
            Err(error) => {
                self.log.push(format!("Could not pose the model: {error}"));
                return;
            }
        };
        let walk = match model.posed_display_list_vertices(&matrices) {
            Ok(walk) => walk,
            Err(error) => {
                self.log.push(format!("Could not walk the model: {error}"));
                return;
            }
        };

        // The export put the coordinate into image space, where glTF and PNG
        // both keep V. It goes back the way it came.
        let projected = match self.authored_mask() {
            Some((_, name)) => name.as_deref() == Some(GOOP_LAYER_TEXTURE),
            None => true,
        };
        let mut missed = 0usize;
        let mut matched = Vec::with_capacity(walk.len());
        for position in &walk {
            let key = gltf_position_key(*position);
            // A hundredth apart in the round trip is still the same vertex, so
            // the neighbouring cells are tried before giving up on one.
            let found = unwrapped.get(&key).copied().or_else(|| {
                (-1..=1).find_map(|x| {
                    (-1..=1).find_map(|y| {
                        (-1..=1).find_map(|z| {
                            unwrapped
                                .get(&[key[0] + x, key[1] + y, key[2] + z])
                                .copied()
                        })
                    })
                })
            });
            match found {
                Some(coordinate) => matched.push(if projected {
                    [coordinate[0], 1.0 - coordinate[1]]
                } else {
                    coordinate
                }),
                None => {
                    missed += 1;
                    matched.push([0.0, 0.0]);
                }
            }
        }
        if missed > 0 {
            self.log.push(format!(
                "{missed} of {} vertices had no match in that mesh, so it is not the model that \
                 was exported -- or it was moved rather than re-unwrapped.",
                walk.len()
            ));
            return;
        }

        if let Err(error) = model.replace_texcoord_values(slot as u8, &matched) {
            self.log
                .push(format!("Could not store the unwrap: {error}"));
            return;
        }
        // Rebuilding proves the edit before it is kept.
        if let Err(error) = model.to_bytes() {
            self.log
                .push(format!("The re-unwrapped model would not rebuild: {error}"));
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let before = document.archive_edits.clone();
        document.archive_edits.replace_model(raw_path, model);
        self.finish_mask_author_edit(
            before,
            format!(
                "Re-unwrapped this actor's goop coordinate from {} across {} vertices.",
                path.display(),
                walk.len()
            ),
        );
        self.build_mask_preview(&choice);
    }

    /// Writes the coverage on the slider into the model's own wash, so the
    /// actor ships coated at that level.
    ///
    /// Washable goop is a comparison the model runs per pixel: the mask is
    /// tested against a konst, and the coating shows where it wins. Baking
    /// sets that konst. The stage viewport reads it back through the archive
    /// edit, and so does the game.
    ///
    /// Retail drives the same konst from an enemy's hit points, so a wired
    /// enemy takes its class's level once it spawns and the baked value is
    /// only its starting state. An actor with nothing driving it keeps what
    /// is written here.
    fn bake_mask_goop_layer(&mut self) {
        let Some(choice) = self.selected_mask_choice() else {
            self.log.push("Pick an actor first.".to_string());
            return;
        };
        // The wash compares the mask against this konst and coats where the
        // mask does not exceed it, so the konst rises with coverage: retail
        // drives it from hit points, full health fully coated.
        // The coverage slider is the threshold either way: it is the konst the
        // comparison is baked against, so it decides how much coating the actor
        // wears the moment it spawns. Washable does not override that -- it is
        // where the runtime starts counting down from, so authoring a
        // half-coated actor gives a half-length wash rather than a full one.
        let level = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;

        // The materials that run a comparison, named, so the model can be
        // asked about them directly.
        let materials: Vec<String> = self
            .mask_preview
            .as_ref()
            .map(|preview| {
                preview
                    .geometry
                    .materials
                    .iter()
                    .filter(|material| material.tev_stages.iter().any(stage_is_wash_comparison))
                    .map(|material| material.name.clone())
                    .collect()
            })
            .unwrap_or_default();

        let (raw_path, bytes) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            let Some(raw_path) = document.archive_resource_path_for_asset(&choice.model_path)
            else {
                self.log.push(
                    "This actor's model lives outside the stage archive, so it cannot be \
                     baked here yet."
                        .to_string(),
                );
                return;
            };
            let bytes = match document.read_asset_bytes(&choice.model_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.log.push(format!("Could not read the model: {error}"));
                    return;
                }
            };
            (raw_path, bytes)
        };
        let mut model = match sms_formats::J3dRebuildDocument::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.log.push(format!("Could not parse the model: {error}"));
                return;
            }
        };

        // A model that runs no comparison has no coverage to set, so give it
        // the layer first and set the level as part of authoring it.
        if materials.is_empty() {
            let parsed = match sms_formats::J3dFile::parse(&bytes) {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.log.push(format!("Could not read the model: {error}"));
                    return;
                }
            };
            let posed = self.mask_pose_animation(&choice);
            match self.author_goop_layer(
                &mut model,
                &parsed,
                choice.load_flags,
                level,
                posed.as_ref().map(|(animation, frame)| (animation, *frame)),
            ) {
                Ok((count, konst)) => {
                    self.remember_wash_konst(konst);
                    self.remember_wash_vtable(&choice);
                    let Some(document) = self.document.as_mut() else {
                        return;
                    };
                    let before = document.archive_edits.clone();
                    document.archive_edits.replace_model(raw_path, model);
                    self.finish_mask_author_edit(
                        before,
                        format!(
                            "Gave this actor a goop layer across {count} material(s) at coverage \
                             {:.0}%; the stage carries it now.",
                            self.mask_wash_phase * 100.0
                        ),
                    );
                    self.build_mask_preview(&choice);
                }
                Err(error) => self.log.push(error),
            }
            return;
        }

        let mut written = 0usize;
        for name in &materials {
            let Some(register) = model.material_wash_konst_register(name) else {
                continue;
            };
            match model.set_material_konst_alpha(name, register, level) {
                Ok(count) => written += count,
                Err(error) => {
                    self.log
                        .push(format!("Could not bake into '{name}': {error}"));
                    return;
                }
            }
        }
        if written == 0 {
            self.log.push(
                "The model's wash konst could not be located; nothing was baked.".to_string(),
            );
            return;
        }
        // Rebuilding proves the edit before it is kept.
        if let Err(error) = model.to_bytes() {
            self.log
                .push(format!("The baked model would not rebuild: {error}"));
            return;
        }

        let Some(document) = self.document.as_mut() else {
            return;
        };
        let before = document.archive_edits.clone();
        document.archive_edits.replace_model(raw_path, model);
        self.finish_mask_author_edit(
            before,
            format!(
                "Baked coverage {:.0}% into {written} material(s) as wash level {level}; the \
                 stage carries it now.",
                self.mask_wash_phase * 100.0
            ),
        );
        self.build_mask_preview(&choice);
    }

    /// Builds a goop layer and writes it into every material the actor draws.
    ///
    /// The coating is the selected goop colour with the current mask in its
    /// alpha, and the coordinate is a front projection over the model's own
    /// bounds -- the layout retail authored for the enemies that ship with
    /// goop, and the one the preview has been drawing all along.
    fn author_goop_layer(
        &self,
        model: &mut sms_formats::J3dRebuildDocument,
        model_file: &sms_formats::J3dFile,
        load_flags: u32,
        level: u8,
        posed: Option<(&sms_formats::J3dJointAnimation, f32)>,
    ) -> Result<(usize, u8), String> {
        let preview = self
            .mask_preview
            .as_ref()
            .ok_or_else(|| "No actor is loaded.".to_string())?;
        let (size, mask) = self
            .active_mask()
            .ok_or_else(|| "Generate or assign a goop mask before baking.".to_string())?;

        // The coating carries the goop map at its own resolution, with the
        // mask sampled up into the alpha. Authoring at the mask's size
        // instead -- masks are small, often thirty two square -- reduces the
        // goop to a smear, which is nothing like what the preview composites.
        let native = match self.mask_goop_image.as_ref() {
            Some((width, height, _)) if *width > 0 && *height > 0 => (*width).max(*height),
            _ => 256,
        };
        // GX wants sides that are multiples of four, and there is no gain
        // beyond the goop map's own detail.
        let resolution = native.next_multiple_of(4).clamp(size.max(32), 512);
        let mut pixels = Vec::with_capacity(resolution * resolution * 4);
        for y in 0..resolution {
            for x in 0..resolution {
                let u = (x as f32 + 0.5) / resolution as f32;
                // No flip: the stored coordinate puts v zero at the model's
                // foot, and GX reads v zero from the first row written, so the
                // rows have to run the way the preview samples them. Flipping
                // here has the model read the goop map upside down.
                let v = (y as f32 + 0.5) / resolution as f32;
                let colour = self.goop_colour(u, v);
                pixels.extend_from_slice(&[
                    colour[0],
                    colour[1],
                    colour[2],
                    // The wash stops at level zero, and the coating shows where
                    // the mask loses or ties -- so a texel of zero ties at zero
                    // and stays coated however long it is sprayed. Keeping the
                    // mask off the floor reserves zero for "already clean", so
                    // the last of the coating can actually leave.
                    self.mask_reading(sample_mask_bilinear(&mask, size, u, v))
                        .max(1),
                ]);
            }
        }
        let width =
            u16::try_from(resolution).map_err(|_| "The coating is too large.".to_string())?;
        let image = sms_formats::RgbaImage::new(width, width, pixels)
            .map_err(|error| format!("Could not stage the coating: {error}"))?;
        let encoded = sms_formats::GxEncodedTexture::encode_rgba(
            GOOP_LAYER_TEXTURE,
            &image,
            sms_formats::GxTextureEncodeOptions::default(),
        )
        .map_err(|error| format!("Could not encode the coating: {error}"))?;
        let texture = encoded
            .to_bti()
            .map_err(|error| format!("Could not encode the coating: {error}"))?;

        // The goop coordinate is stored in the vertex data rather than
        // generated, which is how retail carries one: a generated coordinate
        // reads the vertex position in whichever joint's space it lives, and
        // an actor whose parts sit in different joints cannot be served by a
        // single matrix.
        // What the preview shows is what the materials sample, and a model can
        // store a coordinate nothing reads -- LandGesso keeps a UV1 for its
        // BTK while no stage samples it. Authoring over such a slot is refused
        // by the writer, so take the union of what is sampled and what is
        // actually stored.
        let mut used = preview
            .geometry
            .triangles
            .iter()
            .fold([false; 8], |mut used, triangle| {
                for (slot, set) in triangle.tex_coord_sets.iter().enumerate() {
                    used[slot] |= set.is_some();
                }
                used
            });
        for (slot, stored) in model.stored_texcoord_slots().iter().enumerate() {
            used[slot] |= *stored;
        }
        let slot = used
            .iter()
            .position(|used| !used)
            .ok_or_else(|| "This model stores all eight coordinate sets.".to_string())?
            as u8;
        let matrices = model_file
            .preview_draw_matrices(load_flags, posed, &[])
            .map_err(|error| format!("Could not pose the model: {error}"))?;
        model
            .store_front_projection_texcoord(slot, &matrices)
            .map_err(|error| format!("Could not store the goop coordinate: {error}"))?;

        // Every material has to read the same konst, because what drives the
        // wash writes one register: a material that claimed a different one
        // keeps its coating for good, which reads as the wash stopping halfway
        // when it is really one material never starting. Left to themselves the
        // materials each take the first register they have spare, and those
        // differ -- so a register all of them can take is chosen up front, by
        // trying each against a copy that is thrown away.
        // A class that paints its own colour on this model needs its wash level
        // in K0 and nowhere else: the only packet form that carries a TEV
        // colour alongside a konst writes K0 and takes no register argument. It
        // is not free on Cataquack, so the bake moves the material's own use of
        // it aside rather than refusing.
        let tinted = self
            .document
            .as_ref()
            .zip(self.registry.as_ref())
            .and_then(|(document, registry)| {
                let selected = self.mask_selected_actor.as_deref()?;
                let object = document
                    .objects
                    .iter()
                    .find(|object| object.id == selected)?;
                registry
                    .enemy_material_colors
                    .iter()
                    .find(|colour| colour.factory_name == object.factory_name)
            })
            .is_some();
        let mut shared_konst = tinted.then_some(0usize);
        for candidate in (0..4usize).filter(|_| !tinted) {
            let mut trial = model.clone();
            let mut taken = 0usize;
            let mut refused = false;
            for material in &preview.geometry.materials {
                if material.tev_stages.is_empty() {
                    continue;
                }
                let request = sms_formats::GoopLayerRequest {
                    material_name: &material.name,
                    texture_name: GOOP_LAYER_TEXTURE,
                    texture: &texture,
                    coordinate_slot: slot,
                    level,
                    preferred_konst: Some(candidate),
                    step: self.mask_wash_step.clamp(1, 255) as u8,
                    resistance: self.mask_wash_resistance.clamp(1, 255) as u8,
                };
                match trial.add_goop_layer(&request) {
                    Ok(_) => taken += 1,
                    Err(_) => {
                        refused = true;
                        break;
                    }
                }
            }
            if !refused && taken > 0 {
                shared_konst = Some(candidate);
                break;
            }
        }

        let mut authored = 0usize;
        let mut claimed_konst: Option<u8> = None;
        let mut failures = Vec::new();
        for material in &preview.geometry.materials {
            if material.tev_stages.is_empty() {
                continue;
            }
            let request = sms_formats::GoopLayerRequest {
                material_name: &material.name,
                texture_name: GOOP_LAYER_TEXTURE,
                texture: &texture,
                coordinate_slot: slot,
                level,
                preferred_konst: shared_konst,
                // Recorded against this actor rather than the project, so a
                // stubborn class can be tuned without dragging every other
                // actor's wash along with it.
                step: self.mask_wash_step.clamp(1, 255) as u8,
                resistance: self.mask_wash_resistance.clamp(1, 255) as u8,
            };
            match model.add_goop_layer(&request) {
                Ok(report) => {
                    // With a shared register every material reports the same
                    // one; without, this keeps the last, and the log below says
                    // so rather than leaving it to be discovered in the game.
                    if claimed_konst.is_some_and(|had| had != report.konst_register as u8) {
                        failures.push(format!(
                            "{} reads K{} where another material reads K{}, so one of them \
                             will not wash",
                            material.name,
                            report.konst_register,
                            claimed_konst.unwrap_or(0)
                        ));
                    }
                    claimed_konst = Some(report.konst_register as u8);
                    authored += 1;
                }
                Err(error) => failures.push(format!("{}: {error}", material.name)),
            }
        }
        if authored == 0 {
            return Err(match failures.first() {
                Some(first) => format!("No material took a goop layer ({first})."),
                None => "This model has no material to give a goop layer to.".to_string(),
            });
        }
        model
            .to_bytes()
            .map_err(|error| format!("The authored model would not rebuild: {error}"))?;
        Ok((authored, claimed_konst.unwrap_or(0)))
    }

    /// Replaces the actor's wash mask with a painted image, as a stage
    /// archive edit that previews immediately and ships with the build.
    fn reimport_mask_goop_map(&mut self) {
        let Some(choice) = self.selected_mask_choice() else {
            self.log.push("Pick an actor first.".to_string());
            return;
        };
        let Some((_, Some(mask_name))) = self.authored_mask() else {
            self.log
                .push("This actor has no authored goop mask slot to replace.".to_string());
            return;
        };
        let wraps = self
            .mask_preview
            .as_ref()
            .and_then(|preview| {
                preview
                    .geometry
                    .textures
                    .iter()
                    .find(|texture| texture.name == mask_name)
            })
            .map(|texture| (texture.wrap_s, texture.wrap_t))
            .unwrap_or((1, 1));

        let Some(path) = rfd::FileDialog::new()
            .set_title("Reimport Goop Map")
            .add_filter("PNG image", &["png"])
            .pick_file()
        else {
            return;
        };
        let painted = match image::open(&path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                self.log.push(format!("Could not read the image: {error}"));
                return;
            }
        };
        // The export put the map into image space, where PNG keeps V, so a map
        // painted against that layout comes back the other way up. The model
        // stores its own the way GX reads it -- first row written is v zero,
        // which the stored coordinate puts at the model's foot -- so reading a
        // painted map straight in lands the mask upside down and moves the
        // coating without anything having been repainted.
        let painted = if mask_name == GOOP_LAYER_TEXTURE {
            image::imageops::flip_vertical(&painted)
        } else {
            painted
        };
        let (width, height) = painted.dimensions();
        if width == 0
            || height == 0
            || width % 8 != 0
            || height % 8 != 0
            || width > 1024
            || height > 1024
        {
            self.log.push(format!(
                "Goop masks must have sides that are multiples of 8, up to 1024                  ({width}x{height} given)."
            ));
            return;
        }
        // A retail mask is one channel, so spreading it lands the encoder on
        // I8 the way those ship. A layer this tool bakes is not: it carries
        // its coating in the colour channels and only its mask in alpha, so
        // spreading there would repaint the coat in grey and throw the colour
        // away. Keep the coat and take only the alpha from what was painted.
        let pixels: Vec<u8> = if mask_name == GOOP_LAYER_TEXTURE {
            let coat = self.mask_preview.as_ref().and_then(|preview| {
                preview
                    .geometry
                    .textures
                    .iter()
                    .find(|texture| texture.name == mask_name)
            });
            painted
                .enumerate_pixels()
                .flat_map(|(x, y, pixel)| {
                    let u = (x as f32 + 0.5) / width as f32;
                    let v = (y as f32 + 0.5) / height as f32;
                    let texel = coat
                        .and_then(|coat| sample_texture(coat, u, v))
                        .unwrap_or([255, 255, 255, 255]);
                    // Zero would tie with the wash's floor and never leave.
                    [texel[0], texel[1], texel[2], pixel.0[0].max(1)]
                })
                .collect()
        } else {
            painted
                .pixels()
                .flat_map(|pixel| {
                    let value = pixel.0[0];
                    [value, value, value, value]
                })
                .collect()
        };
        let rgba = match sms_formats::RgbaImage::new(width as u16, height as u16, pixels) {
            Ok(rgba) => rgba,
            Err(error) => {
                self.log.push(format!("Could not stage the image: {error}"));
                return;
            }
        };
        let mut options = sms_formats::GxTextureEncodeOptions::default();
        options.sampler.wrap_s = wraps.0;
        options.sampler.wrap_t = wraps.1;
        let bti =
            match sms_formats::GxEncodedTexture::encode_rgba(mask_name.clone(), &rgba, options)
                .and_then(|encoded| encoded.to_bti())
            {
                Ok(bti) => bti,
                Err(error) => {
                    self.log.push(format!("Could not encode the mask: {error}"));
                    return;
                }
            };

        let (raw_path, bytes) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            let Some(raw_path) = document.archive_resource_path_for_asset(&choice.model_path)
            else {
                self.log.push(
                    "This actor's model lives outside the stage archive, so its mask cannot                      be edited here yet."
                        .to_string(),
                );
                return;
            };
            let bytes = match document.read_asset_bytes(&choice.model_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.log.push(format!("Could not read the model: {error}"));
                    return;
                }
            };
            (raw_path, bytes)
        };
        let mut model = match sms_formats::J3dRebuildDocument::parse(&bytes) {
            Ok(model) => model,
            Err(error) => {
                self.log.push(format!("Could not parse the model: {error}"));
                return;
            }
        };
        let replaced = match model.replace_named_texture_from_bti(&mask_name, &bti) {
            Ok(count) => count,
            Err(error) => {
                self.log
                    .push(format!("Could not replace '{mask_name}': {error}"));
                return;
            }
        };
        if replaced == 0 {
            self.log
                .push(format!("The model carries no texture named '{mask_name}'."));
            return;
        }

        let Some(document) = self.document.as_mut() else {
            return;
        };
        let before = document.archive_edits.clone();
        document.archive_edits.replace_model(raw_path, model);
        self.finish_mask_author_edit(
            before,
            format!("Reimported '{mask_name}' at {width}x{height}; Ctrl+Z reverses it."),
        );
        self.build_mask_preview(&choice);
    }

    /// Drops the reimported mask, returning the actor to what its model
    /// originally authored.
    /// The actor's model as the retail game ships it.
    ///
    /// Reset has to restore the model, not merely drop the edit layered over
    /// it. A custom stage keeps its own copy of every actor in the project's
    /// resource list, so once a coating is baked into that copy there is
    /// nothing underneath to fall back to -- which is why the goop survived
    /// every reset. The untouched model is still in the retail archives the
    /// stage was built from, found by the same archive-internal path.
    ///
    /// Returns `None` rather than a guess: reset refuses instead of clearing
    /// when no pristine copy can be proven, because the resource list is also
    /// where the stage keeps its only copy of the model.
    fn pristine_model_bytes(&self, internal_path: &str) -> Option<Vec<u8>> {
        let document = self.document.as_ref()?;
        let archives = sms_formats::discover_scene_archives(&document.base_root).ok()?;
        for archive in archives {
            let candidate = format!("{}!/{}", archive.path.display(), internal_path);
            let Ok(bytes) = sms_formats::read_stage_asset_bytes(&candidate) else {
                continue;
            };
            // Prove it before it goes anywhere near the project.
            if sms_formats::J3dFile::parse(&bytes).is_ok() {
                return Some(bytes);
            }
        }
        None
    }

    /// Puts every actor in the stage back on its retail model.
    ///
    /// Runs the single-actor reset over each in turn rather than reaching into
    /// the archive directly: on a custom stage the resource list is the stage's
    /// own content, so the only safe way to shed a coating is the one that
    /// restores the model from retail behind it.
    fn restore_every_mask_goop_default(&mut self) {
        let restored_from = self.mask_selected_actor.clone();
        let actors: Vec<String> = self
            .mask_actor_choices()
            .into_iter()
            .map(|choice| choice.object_id)
            .collect();
        let total = actors.len();
        let mut reset = 0usize;
        for object_id in actors {
            self.mask_selected_actor = Some(object_id);
            let said = self.log.len();
            self.restore_mask_goop_default();
            // The single-actor reset says so when there was nothing to shed,
            // which is the difference between an actor restored and one that
            // was already wearing its own model.
            if !self.log[said..]
                .iter()
                .any(|line| line.contains("already wears"))
            {
                reset += 1;
            }
        }
        self.mask_selected_actor = restored_from;
        // Every layer is gone, so the classes recorded for the wash go with
        // them -- otherwise a build would still be telling the patch to watch
        // for actors that no longer carry anything.
        self.mask_wash_vtables.clear();
        if let Some(project) = self.current_project.as_mut() {
            if !project.descriptor.mask_wash_vtables.is_empty() {
                project.descriptor.mask_wash_vtables.clear();
                self.persist_project_settings(false);
            }
        }
        self.log.push(format!(
            "Put {reset} of {total} actor(s) back on their retail models."
        ));
        if let Some(choice) = self.selected_mask_choice() {
            self.build_mask_preview(&choice);
        }
    }

    fn restore_mask_goop_default(&mut self) {
        let Some(choice) = self.selected_mask_choice() else {
            return;
        };
        // Both of these used to return in silence, which is indistinguishable
        // from a button that does nothing at all: the path is only worked out
        // for a document that knows where its stage archive came from.
        let Some(raw_path) = self
            .document
            .as_ref()
            .and_then(|document| document.archive_resource_path_for_asset(&choice.model_path))
        else {
            self.log.push(
                "This actor's model lives outside the stage archive, so there is no edit to \
                 reset."
                    .to_string(),
            );
            return;
        };
        let Some(document) = self.document.as_mut() else {
            self.log
                .push("No stage is open, so there is nothing to reset.".to_string());
            return;
        };
        let before = document.archive_edits.clone();
        // Only the model edit, never the resource edit. On a custom stage the
        // resource list is not a pile of overrides to shed -- it is where the
        // stage keeps its own content, the actor's base model included. Clearing
        // it here deleted the model this reset was meant to restore, and every
        // later read fell through to a stage archive that a custom stage never
        // wrote to disk.
        let count = document.archive_edits.models.len();
        document
            .archive_edits
            .models
            .retain(|edit| edit.raw_resource_path != raw_path);

        // Dropping the edit is only half of it. On a custom stage the resource
        // list holds the stage's own copy of the model, and a baked coating
        // lives in that copy -- so restore it from retail rather than leaving
        // whatever the last bake wrote.
        let internal = String::from_utf8_lossy(&raw_path).into_owned();
        let mut restored = false;
        if let Some(pristine) = self.pristine_model_bytes(&internal) {
            if let Some(document) = self.document.as_mut() {
                if let Some(edit) = document
                    .archive_edits
                    .resources
                    .iter_mut()
                    .find(|edit| edit.raw_resource_path == raw_path)
                {
                    match sms_scene::StageResourceDocument::parse_for_path(&raw_path, &pristine) {
                        Ok(document) => {
                            edit.document = document;
                            restored = true;
                        }
                        Err(error) => {
                            // Say so rather than restore nothing quietly: the
                            // actor keeps whatever it was wearing, and a reset
                            // that reports success would be a lie.
                            self.log.push(format!(
                                "The retail model for this actor could not be read back:                                  {error}"
                            ));
                        }
                    }
                }
            }
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if document.archive_edits.models.len() == count && !restored {
            self.log
                .push("This actor already wears its original mask.".to_string());
            return;
        }
        self.finish_mask_author_edit(
            before,
            "Reset the actor's model to its original -- baked coatings and reimported \
             masks dropped."
                .to_string(),
        );
        // Reset clears the actor, not merely its stored edit. The tool paints a
        // coating of its own -- a generated mask, or StayPakkun's borrowed
        // default over a front projection -- and that overlay outlives the edit.
        // Dropping the edit alone left the goop sitting on screen, which is why
        // the button read as inert even on the runs where it worked.
        self.mask_generated = false;
        self.mask_mask_source = MaskTextureSource::Generated;
        self.build_mask_preview(&choice);
    }

    /// Records an archive edit for undo and marks the document dirty.
    fn finish_mask_author_edit(&mut self, before: StageArchiveEdits, message: String) {
        // The stage viewport keeps its own copy of every actor, built when the
        // document loaded. An authoring edit changes the resource underneath it
        // and nothing told it to look again, so a reset restored the retail
        // model while the screen kept drawing the coated one -- and the tool,
        // which clones that same cache to draw its preview, kept drawing it too.
        self.rebuild_model_preview_from_document();
        let (record, dirty) = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            (
                ObjectUndoRecord::between(
                    &document.objects,
                    &document.objects,
                    &before,
                    &document.archive_edits,
                ),
                stage_document_differs_from_saved(
                    document,
                    &self.saved_objects,
                    &self.saved_lighting,
                    &self.saved_death_barrier,
                    &self.saved_archive_edits,
                    &self.saved_dialogue_authoring,
                    &self.saved_dialogue_library,
                ),
            )
        };
        if !record.is_empty() {
            self.push_undo_record(record);
        }
        self.document_dirty = dirty;
        self.flush_document_change();
        self.log.push(message);
    }

    /// Takes the wash settings from the project the first time this panel
    /// draws for it, so a coverage and an invert survive a restart.
    fn restore_mask_wash_settings(&mut self) {
        let Some(project) = self.current_project.as_ref() else {
            return;
        };
        let id = project.descriptor.project_id.to_string();
        if self.mask_wash_settings_project.as_deref() == Some(id.as_str()) {
            return;
        }
        self.mask_wash_phase = project.descriptor.mask_wash_coverage.clamp(0.0, 1.0);
        self.mask_wash_invert = project.descriptor.mask_wash_invert;
        self.mask_bake_washable = project.descriptor.mask_bake_washable;
        self.mask_wash_resistance = project.descriptor.mask_wash_resistance.max(1);
        self.mask_wash_step = project.descriptor.mask_wash_step.clamp(1, 255);
        self.mask_wash_uniform_rate = project.descriptor.mask_wash_uniform_rate;
        self.mask_wash_reach = project.descriptor.mask_wash_reach.clone();
        self.mask_wash_settings_project = Some(id);
    }

    /// Records which konst register a bake claimed.
    ///
    /// The wash has to write the register the layer reads. The bake takes
    /// whichever the material had spare, so the answer moves between actors and
    /// between bakes of the same actor, and guessing it wrong looks exactly
    /// like a wash that never runs.
    fn remember_wash_konst(&mut self, register: u8) {
        let Some(project) = self.current_project.as_mut() else {
            return;
        };
        if project.descriptor.mask_wash_konst == register {
            return;
        }
        project.descriptor.mask_wash_konst = register;
        self.persist_project_settings(false);
    }

    /// The colours the classes wearing this model paint on it, if any.
    ///
    /// The material index is resolved against the model the tool has loaded,
    /// because the runtime needs an index rather than a name.
    fn wash_tints_for(
        &self,
        choice: &MaskActorChoice,
        wearers: &[u32],
    ) -> Vec<crate::project::MaskWashTint> {
        let Some(registry) = self.registry.as_ref() else {
            return Vec::new();
        };
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let Some(preview) = self.mask_preview.as_ref() else {
            return Vec::new();
        };
        let mut tints = Vec::new();
        for other in self.mask_actor_choices() {
            if other.model_path != choice.model_path {
                continue;
            }
            let Some(object) = document
                .objects
                .iter()
                .find(|object| object.id == other.object_id)
            else {
                continue;
            };
            let Some(class) = registry
                .find_object(&object.factory_name)
                .map(|definition| definition.class_name.clone())
            else {
                continue;
            };
            let Some(vtable) = crate::class_vtables::class_vtable(&class) else {
                continue;
            };
            if !wearers.contains(&vtable) {
                continue;
            }
            let Some(colour) = registry
                .enemy_material_colors
                .iter()
                .find(|colour| colour.factory_name == object.factory_name)
            else {
                continue;
            };
            let Some(material) = preview
                .geometry
                .materials
                .iter()
                .position(|material| material.name == colour.material_name)
            else {
                continue;
            };
            tints.push(crate::project::MaskWashTint {
                vtable,
                material: material as u16,
                // The registry counts registers the way the decomp names them,
                // where zero is TEV register zero. `GXTevRegID` counts from
                // `GX_TEVPREV`, so the same register is one higher -- and the
                // manager that paints Cataquack passes exactly that.
                register: colour.tev_register + 1,
                // A channel the class does not assign keeps whatever the model
                // authored, which is zero as far as this record is concerned.
                color: std::array::from_fn(|index| colour.color[index].unwrap_or(0)),
            });
        }
        tints
    }

    /// Remembers the class this actor is, so a wash that reaches everything can
    /// tell the actors this project authored from the ones it is handed.
    ///
    /// The vtable is what an actor carries at offset zero, which is the only
    /// identity a stub can check in a load and a compare.
    fn remember_wash_vtable(&mut self, choice: &MaskActorChoice) {
        // The coating is in the model, not in the class, so every actor wearing
        // that model wears it -- and every one of them has to be able to wash
        // it off. Cataquack comes in two classes that share one model: baking
        // on the blue one coated the red one too, and recording only the class
        // that was baked left red gooped for good.
        let wearers: Vec<u32> = {
            let Some(document) = self.document.as_ref() else {
                return;
            };
            let Some(registry) = self.registry.as_ref() else {
                return;
            };
            self.mask_actor_choices()
                .iter()
                .filter(|other| other.model_path == choice.model_path)
                .filter_map(|other| {
                    let object = document
                        .objects
                        .iter()
                        .find(|object| object.id == other.object_id)?;
                    let class = &registry.find_object(&object.factory_name)?.class_name;
                    crate::class_vtables::class_vtable(class)
                })
                .collect()
        };
        if wearers.is_empty() {
            return;
        }
        for vtable in &wearers {
            if !self.mask_wash_vtables.contains(vtable) {
                self.mask_wash_vtables.push(*vtable);
            }
        }
        // Each wearer may paint its own colour on the shared model -- blue and
        // red Cataquack differ by nothing else -- so the colour is recorded
        // against the class that paints it, to be rebound rather than lost.
        let tints = self.wash_tints_for(choice, &wearers);
        let changed = match self.current_project.as_mut() {
            Some(project) => {
                let mut changed = false;
                for vtable in &wearers {
                    if !project.descriptor.mask_wash_vtables.contains(vtable) {
                        project.descriptor.mask_wash_vtables.push(*vtable);
                        changed = true;
                    }
                }
                for tint in tints {
                    if !project.descriptor.mask_wash_tints.contains(&tint) {
                        project
                            .descriptor
                            .mask_wash_tints
                            .retain(|had| had.vtable != tint.vtable);
                        project.descriptor.mask_wash_tints.push(tint);
                        changed = true;
                    }
                }
                changed
            }
            None => false,
        };
        if changed {
            self.persist_project_settings(false);
        }
    }

    /// Keeps the project's copy in step once a control moves.
    fn remember_mask_wash_settings(&mut self) {
        let (coverage, invert, washable, resistance, step, uniform, reach) = (
            self.mask_wash_phase,
            self.mask_wash_invert,
            self.mask_bake_washable,
            self.mask_wash_resistance,
            self.mask_wash_step,
            self.mask_wash_uniform_rate,
            self.mask_wash_reach.clone(),
        );
        match self.current_project.as_mut() {
            Some(project) => {
                if project.descriptor.mask_wash_coverage == coverage
                    && project.descriptor.mask_wash_invert == invert
                    && project.descriptor.mask_bake_washable == washable
                    && project.descriptor.mask_wash_resistance == resistance
                    && project.descriptor.mask_wash_step == step
                    && project.descriptor.mask_wash_uniform_rate == uniform
                    && project.descriptor.mask_wash_reach == reach
                {
                    return;
                }
                project.descriptor.mask_wash_coverage = coverage;
                project.descriptor.mask_wash_invert = invert;
                project.descriptor.mask_bake_washable = washable;
                project.descriptor.mask_wash_resistance = resistance;
                project.descriptor.mask_wash_step = step;
                project.descriptor.mask_wash_uniform_rate = uniform;
                project.descriptor.mask_wash_reach = reach;
            }
            None => return,
        }
        self.persist_project_settings(false);
    }

    /// The inspector panel for the Mask Tool.
    pub(super) fn mask_tool_panel(&mut self, ui: &mut egui::Ui) {
        // The goop catalog is indexed lazily by whoever needs it first; the
        // colour list offers those styles, so make sure they are loaded.
        self.ensure_goop_templates_indexed();
        self.restore_mask_wash_settings();
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
                // Seed the colour from the style this actor's layer was given,
                // so a chocolate pool opens on chocolate rather than a rainbow
                // it can never wear. Only on selection: doing it whenever the
                // preview rebuilds would throw away a colour just chosen.
                if let Some(style) = self.actor_layer_style(choice) {
                    self.mask_colour_source = MaskTextureSource::GoopStyle(style);
                    self.load_goop_style(style);
                }
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
        // On an actor that already carries a layer the coating on screen is the
        // model's own texture, drawn by the stage renderer -- these pick what a
        // bake would write next, not what is being shown. HamuKuri's layer
        // samples an I4 image, so it draws grey however this is set, and
        // BossGesso's is coloured so the difference never shows there. Saying so
        // is the difference between a control that is scoped and one that looks
        // broken.
        if self.authored_mask().is_some() {
            ui.label(
                egui::RichText::new(
                    "this actor wears its own layer -- these choose what the next bake                      writes, not what is drawn",
                )
                .small()
                .weak(),
            );
        }
        let mut picked_style = None;
        egui::ComboBox::from_label("Colour")
            .selected_text(colour_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.mask_colour_source,
                    MaskTextureSource::Generated,
                    "Rainbow (generated)",
                );
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
                match self.authored_mask() {
                    Some((_, Some(name))) => format!("Authored: {name}"),
                    Some((_, None)) => "Authored (model's own mask)".to_string(),
                    None => "StayPakkun default (borrowed)".to_string(),
                }
            }
            MaskTextureSource::Model(index) => textures
                .iter()
                .find(|(slot, _)| *slot == index)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Authored / StayPakkun default".to_string()),
        };
        egui::ComboBox::from_label("Mask")
            .selected_text(mask_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.mask_mask_source,
                    MaskTextureSource::Generated,
                    "Authored / StayPakkun default",
                );
            });
        // Which pool folder an instance reads is not something the tool should
        // leave implicit: the goop tool emits one per layer and they differ, so
        // the resolved model and the manager that chose it are shown.
        if self.authored_mask().is_some() {
            let manager = self
                .document
                .as_ref()
                .zip(self.mask_selected_actor.as_ref())
                .and_then(|(document, id)| {
                    document
                        .objects
                        .iter()
                        .find(|object| &object.id == id)
                        .and_then(|object| object.raw_param("manager_name"))
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "<no manager_name>".to_string());
            let model = self
                .selected_mask_choice()
                .map(|choice| {
                    let path = choice.model_path.replace(char::from(92), "/");
                    path.rsplit('/')
                        .take(2)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            self.mask_layer_pool_label =
                format!("Layer pool: {model}  \u{2014}  manager {manager}");
        } else {
            self.mask_layer_pool_label = String::new();
        }
        let renderer = if self.mask_gpu_scene.is_some() {
            "stage renderer"
        } else {
            "CPU fallback (no stage model found for this object)"
        };
        let mask_status = match self.authored_mask() {
            Some((_, name)) => {
                let authored_coord = self.authored_goop_coord();
                let (carried, total) = self
                    .mask_preview
                    .as_ref()
                    .map(|preview| {
                        let carried = preview
                            .geometry
                            .triangles
                            .iter()
                            .filter(|triangle| {
                                triangle.mask_tex_coords.is_some()
                                    || authored_coord.is_some_and(|coord| {
                                        triangle
                                            .tex_coord_sets
                                            .get(coord)
                                            .copied()
                                            .flatten()
                                            .is_some()
                                    })
                            })
                            .count();
                        (carried, preview.geometry.triangles.len())
                    })
                    .unwrap_or((0, 0));
                format!(
                    "authored mask '{}' via its own UV on {carried} of {total} triangles",
                    name.unwrap_or_else(|| "model's own".to_string())
                )
            }
            None => "no authored mask; StayPakkun default over a front projection".to_string(),
        };
        ui.small(format!("{renderer} \u{2014} {mask_status}"));
        ui.small(&self.mask_edit_state);

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
        // A model is drawn by the stage viewport's renderer; a UV layout is a
        // flat diagram this module draws itself.
        if self.mask_view == MaskView::Model && self.mask_gpu_scene.is_some() {
            self.mask_tool_model_viewport(ui);
            return;
        }
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

    /// Draws the actor through the stage viewport's renderer, then lays the
    /// authored coating over it.
    fn mask_tool_model_viewport(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let side = available.x.min(available.y).max(64.0);
        // Take the whole space and centre the square in it, so the viewport is
        // not left sitting against one edge.
        let (outer, response) = ui.allocate_exact_size(available, egui::Sense::drag());
        let rect = egui::Rect::from_center_size(outer.center(), egui::vec2(side, side));
        if response.dragged() {
            let delta = response.drag_delta();
            self.mask_yaw += delta.x * 0.01;
            self.mask_pitch = (self.mask_pitch + delta.y * 0.01).clamp(-1.5, 1.5);
        }

        self.push_mask_wash();
        if let (Some(scene), Some(frame)) =
            (self.mask_gpu_scene.as_ref(), self.mask_gpu_frame(rect))
        {
            scene.set_frame(frame);
            ui.painter().add(scene.paint_callback(rect));
        }

        // The coating is not a property of which UV tab is open. An actor the
        // renderer washes shows it on either, so an actor the tool coats should
        // too -- otherwise the slider works on one tab and not the other purely
        // because of which actor is selected.
        //
        // An authored actor washes inside the renderer itself; the overlay
        // exists only for actors coated with the borrowed mask.
        if !self.mask_wash_materials.is_empty() {
            return;
        }
        let Some(image) = self.mask_goop_overlay_image() else {
            return;
        };
        let texture = self.mask_texture.get_or_insert_with(|| {
            ui.ctx()
                .load_texture("mask-tool-goop", image.clone(), Default::default())
        });
        texture.set(image, Default::default());
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
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

        if ui
            .checkbox(&mut self.mask_wash_invert, "Invert wash pattern")
            .on_hover_text(
                "Clear the mask's dark values first rather than its bright ones, turning the recede inside out",
            )
            .changed()
        {
            self.remember_mask_wash_settings();
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

        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut self.mask_wash_resistance)
                        .range(1..=4096)
                        .speed(1.0),
                )
                .on_hover_text(
                    "Water hits per step of the wash. Independent of the level above,                      so how stubborn a coating is does not depend on how much of it                      there is",
                )
                .changed()
            {
                self.remember_mask_wash_settings();
            }
            ui.label("Resistance");
        });
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut self.mask_wash_step)
                        .range(1..=255)
                        .speed(1.0),
                )
                .on_hover_text(
                    "Wash level removed per step. Raise it for a class the game feeds                      few water hits, so it cleans in step with the rest",
                )
                .changed()
            {
                self.remember_mask_wash_settings();
            }
            ui.label("Step");
        });
        // Reach is a level rather than a set of switches: everything already
        // covers what props covers, so having both would spend a second patch
        // on a hook that never fires.
        let mut reach = self.mask_wash_reach.clone();
        egui::ComboBox::from_label("Wash reaches")
            .selected_text(match reach.as_str() {
                "enemies" => "Enemies",
                "everything" => "Everything",
                _ => "Enemies and props",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut reach, "enemies".to_string(), "Enemies")
                    .on_hover_text("Only the enemy classes, the way retail delivers a spray");
                ui.selectable_value(&mut reach, "props".to_string(), "Enemies and props")
                    .on_hover_text(
                        "Adds the road most scenery inherits -- boats, crates, the objects \
                         that never override how they hear a spray",
                    );
                ui.selectable_value(&mut reach, "everything".to_string(), "Everything")
                    .on_hover_text(
                        "Hooks the spray where it is sent, ahead of every class -- so it \
                         reaches actors that handle the message their own way, like the \
                         Delfino bell. It also counts a spray one step earlier, so an actor \
                         tuned at a narrower reach may wash faster",
                    );
            });
        if reach != self.mask_wash_reach {
            self.mask_wash_reach = reach;
            self.remember_mask_wash_settings();
        }
        // Everything hooks ahead of the per-class cooldown already, so there is
        // no gate left for this to open.
        let rate_applies = self.mask_wash_reach != "everything";
        if ui
            .add_enabled(
                rate_applies,
                egui::Checkbox::new(
                    &mut self.mask_wash_uniform_rate,
                    "Same wash rate for every actor",
                ),
            )
            .on_disabled_hover_text(
                "Everything already counts the spray before an actor's class can gate it",
            )
            .on_hover_text(
                "Count every spray. Off, an actor only washes when its own class lets                  water through -- Gesso clears its spray cooldown each hit and PoiHana                  does not, so the same coating takes far longer on one than the other",
            )
            .changed()
        {
            self.remember_mask_wash_settings();
        }

        if ui
            .add(
                egui::Slider::new(&mut self.mask_wash_phase, 0.0..=1.0)
                    .text("Coverage")
                    .clamping(egui::SliderClamping::Always),
            )
            .drag_stopped()
        {
            self.remember_mask_wash_settings();
        }
        ui.label(
            egui::RichText::new(format!(
                "wash level K{}_A \u{2248} {}  (mask \u{2264} this stays coated)",
                self.authored_wash_konst()
                    .map_or_else(|| "?".to_string(), |r| r.to_string()),
                (self.mask_wash_phase * 255.0).round() as u16
            ))
            .small()
            .color(egui::Color32::GRAY),
        );
        if !self.mask_layer_pool_label.is_empty() {
            ui.label(
                egui::RichText::new(self.mask_layer_pool_label.clone())
                    .small()
                    .weak(),
            );
        }

        ui.separator();
        ui.heading("Pose");
        // A goop coordinate is a projection over posed vertices, so the pose is
        // part of what gets baked. Holding a frame is what makes the result
        // repeatable: left on the wall clock, the same button gives a different
        // unwrap every time it is pressed.
        if ui
            .checkbox(&mut self.mask_bake_tpose, "T-pose (ignore the idle)")
            .changed()
        {
            // The pose is baked into the preview geometry, so changing it has
            // to reload the actor rather than just redraw it.
            if let Some(choice) = self.selected_mask_choice() {
                self.build_mask_preview(&choice);
            }
        }
        // Rebuilding re-reads and re-parses the model, so it waits for the drag
        // to end. On `changed()` it ran every frame the slider moved and the
        // viewport stopped answering, which reads as a frozen tool.
        if ui
            .add_enabled(
                !self.mask_bake_tpose,
                egui::Slider::new(&mut self.mask_idle_frame, 0.0..=8.0)
                    .text("Idle frame")
                    .clamping(egui::SliderClamping::Always),
            )
            .drag_stopped()
        {
            // The pose is baked into the preview geometry, so changing it has
            // to reload the actor rather than just redraw it.
            if let Some(choice) = self.selected_mask_choice() {
                self.build_mask_preview(&choice);
            }
        }
        ui.label(
            egui::RichText::new(if self.mask_bake_tpose {
                "baking against the model's rest pose".to_string()
            } else {
                // Saying which frame is held is only true if an idle was found
                // at all; without one the controls sit over the rest pose and
                // moving them changes nothing, which reads as a dead slider.
                match self
                    .selected_mask_choice()
                    .and_then(|choice| self.mask_pose_animation(&choice))
                {
                    Some((_, frame)) => {
                        format!("baking against the idle held at frame {frame:.1}")
                    }
                    None => "no idle animation resolved for this actor -- rest pose".to_string(),
                }
            })
            .small()
            .weak(),
        );

        ui.heading("Author");
        if ui
            .checkbox(&mut self.mask_bake_washable, "Washable")
            .on_hover_text(
                "Bake the coating fully applied so FLUDD can wash it off in play,                  rather than freezing it at the coverage shown above",
            )
            .changed()
        {
            self.remember_mask_wash_settings();
        }
        ui.label(
            egui::RichText::new({
                let level = (self.mask_wash_phase.clamp(0.0, 1.0) * 255.0).round() as u8;
                if self.mask_bake_washable {
                    format!(
                        "bakes at {:.0}% (K{}_A {level}), -{} every {} water hits",
                        self.mask_wash_phase * 100.0,
                        self.authored_wash_konst()
                            .map_or_else(|| "?".to_string(), |r| r.to_string()),
                        self.mask_wash_step,
                        self.mask_wash_resistance
                    )
                } else {
                    format!(
                        "bakes at {:.0}% (K{}_A {level}) and stays put",
                        self.mask_wash_phase * 100.0,
                        self.authored_wash_konst()
                            .map_or_else(|| "?".to_string(), |r| r.to_string())
                    )
                }
            })
            .small()
            .weak(),
        );
        // Both of these refuse actors they cannot act on, and refusing after
        // the click looked like the button doing nothing at all. Say so on the
        // button instead, so the reason is on screen before it is pressed.
        let can_reimport_mask = matches!(self.authored_mask(), Some((_, Some(_))));
        let can_reimport_uv =
            self.selected_mask_choice().is_some() && self.authored_goop_coord().is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_reimport_mask,
                    egui::Button::new("Reimport goop mask\u{2026}"),
                )
                .on_hover_text(
                    "Replace this actor's wash mask with a painted PNG; it previews here \
                     and ships with the stage",
                )
                .on_disabled_hover_text(
                    "This actor carries no named wash mask to replace. Bake a goop layer \
                     first.",
                )
                .clicked()
            {
                self.reimport_mask_goop_map();
            }
            if ui
                .add_enabled(can_reimport_uv, egui::Button::new("Reimport UV\u{2026}"))
                .on_hover_text("Read a re-unwrapped goop UV back out of an edited glTF")
                .on_disabled_hover_text(
                    "This actor has no goop coordinate to re-unwrap. Bake a goop layer \
                     first, then export it.",
                )
                .clicked()
            {
                self.reimport_mask_uv();
            }
            if ui
                .button("Export glTF\u{2026}")
                .on_hover_text(
                    "Write the actor with both UV sets: the body texture on UV0, the goop \
                     mask on the goop set",
                )
                .clicked()
            {
                self.export_mask_gltf();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Bake goop layer")
                .on_hover_text(
                    "Composite the coating shown at this coverage into the model's own \
                     textures, permanently -- it ships with the stage",
                )
                .clicked()
            {
                let said = self.log.len();
                self.bake_mask_goop_layer();
                self.mask_author_status = self.log[said..].join(" ");
            }
            if ui
                .button("Reset to default")
                .on_hover_text(
                    "Clear every patch this tool applied to the actor's model -- baked \
                     coatings and reimported masks alike",
                )
                .clicked()
            {
                let said = self.log.len();
                self.restore_mask_goop_default();
                self.mask_author_status = self.log[said..].join(" ");
            }
            if ui
                .button("Reset every actor")
                .on_hover_text(
                    "Put every actor in this stage back on its retail model. Baked \
                     coatings and reimported masks are all discarded, across the whole \
                     stage, and the wash forgets which classes carried a layer",
                )
                .clicked()
            {
                let said = self.log.len();
                self.restore_every_mask_goop_default();
                self.mask_author_status = self.log[said..].join(" ");
            }
        });
        // Authoring reported itself only to the Console, which is a different
        // panel and often shut: a refusal there is indistinguishable from a
        // button that does nothing. Say it where the button was pressed.
        if !self.mask_author_status.is_empty() {
            ui.label(
                egui::RichText::new(self.mask_author_status.clone())
                    .small()
                    .weak(),
            );
        }
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
    fn goop_recedes_in_mask_order_as_the_wash_level_falls() {
        // The wash coats where the mask does not exceed the level, so a full
        // level ships the actor coated and an empty one ships it clean --
        // which is the direction retail drives from hit points.
        assert!(goop_is_visible(40, 255));
        assert!(goop_is_visible(200, 255));
        assert!(!goop_is_visible(40, 0));
        assert!(!goop_is_visible(200, 0));
        // Between the two, the brightest mask values clear first: a texel at
        // two hundred is gone by the time the level reaches a hundred and
        // twenty eight, while one at forty is still coated.
        assert!(goop_is_visible(40, 128));
        assert!(!goop_is_visible(200, 128));
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

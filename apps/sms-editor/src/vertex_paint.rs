use super::*;

use std::collections::BTreeMap;

use sms_authoring::{AssetId, ModelAssetDocument, ModelInstanceExportMode, NodePurpose};

/// Brush behaviour, mirroring the modes Affinity exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum VertexPaintMode {
    #[default]
    Brush,
    /// Blends toward white rather than erasing to nothing: white is the
    /// identity colour for a vertex-lit surface.
    Eraser,
    Smooth,
}

impl VertexPaintMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Eraser => "Eraser",
            Self::Smooth => "Smooth",
        }
    }
}

/// Complete state of one asset before or after a terrain edit.
///
/// Vertex-paint tools also expose topology-changing operations such as
/// subdivision and Boolean Cut. Keeping only colours and normals made those
/// operations impossible to undo and could leave mismatched vertex streams.
#[derive(Clone)]
pub(super) struct VertexPaintSnapshot {
    id: AssetId,
    document: ModelAssetDocument,
}

/// One undoable vertex paint operation, however many assets it touched.
pub(super) struct VertexPaintUndoRecord {
    label: String,
    before: Vec<VertexPaintSnapshot>,
    after: Vec<VertexPaintSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotApplyResult {
    Applied,
    Stale,
    Failed,
}

fn snapshot_paint_state(id: AssetId, document: &ModelAssetDocument) -> VertexPaintSnapshot {
    VertexPaintSnapshot {
        id,
        document: document.clone(),
    }
}

fn restore_paint_state(document: &mut ModelAssetDocument, snapshot: &VertexPaintSnapshot) {
    document.clone_from(&snapshot.document);
}

/// Hue rotation, split into its constant, cosine and sine parts so a rotation
/// is three dot products rather than a conversion out to HSV and back.
const HUE_LUMA: [f32; 3] = [0.213, 0.715, 0.072];
const HUE_ROTATION: [[f32; 3]; 3] = [HUE_LUMA, HUE_LUMA, HUE_LUMA];
const HUE_COSINE: [[f32; 3]; 3] = [
    [1.0 - HUE_LUMA[0], -HUE_LUMA[1], -HUE_LUMA[2]],
    [-HUE_LUMA[0], 1.0 - HUE_LUMA[1], -HUE_LUMA[2]],
    [-HUE_LUMA[0], -HUE_LUMA[1], 1.0 - HUE_LUMA[2]],
];
const HUE_SINE: [[f32; 3]; 3] = [
    [-HUE_LUMA[0], -HUE_LUMA[1], 1.0 - HUE_LUMA[2]],
    [0.143, 0.140, -0.283],
    [-(1.0 - HUE_LUMA[0]), HUE_LUMA[1], HUE_LUMA[2]],
];

/// Rewinds triangles whose winding disagrees with their own vertex normals.
///
/// Returns how many were flipped and how many were judged.
///
/// A mesh can arrive wound against its normals. Nothing shows while it is drawn
/// as an object, but map terrain is loaded with flags that leave back-face
/// culling on, so the disagreeing faces drop out and the model reads as inside
/// out. The normals are the intent -- they say which way the artist meant the
/// surface to face -- so the winding is what gets corrected, and only on the
/// triangles that actually disagree. A consistent mesh comes back untouched.
///
/// Local space is the right place for this. Compile reverses winding again for
/// a mirrored placement and carries normals through the same transform, so
/// agreement here survives into the baked model.
pub(super) fn repair_document_winding(document: &mut ModelAssetDocument) -> (usize, usize) {
    let mut flipped = 0usize;
    let mut checked = 0usize;
    for mesh in &mut document.meshes {
        for primitive in &mut mesh.primitives {
            let positions = primitive.positions.clone();
            let normals = primitive.normals.clone();
            for face in primitive.indices.chunks_exact_mut(3) {
                let corners = [face[0] as usize, face[1] as usize, face[2] as usize];
                let Some(corner_positions) = corners
                    .iter()
                    .map(|index| positions.get(*index).copied())
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let geometric = vec3_cross(
                    vec3_sub(corner_positions[1], corner_positions[0]),
                    vec3_sub(corner_positions[2], corner_positions[0]),
                );
                // A sliver has no reliable facing, so leave it rather than
                // flip it on rounding noise.
                if vec3_dot(geometric, geometric) <= 1e-8 {
                    continue;
                }
                let intended = corners
                    .iter()
                    .filter_map(|index| normals.get(*index))
                    .fold([0.0f32; 3], |sum, normal| vec3_add(sum, *normal));
                if vec3_dot(intended, intended) <= 1e-8 {
                    continue;
                }
                checked += 1;
                if vec3_dot(geometric, intended) < 0.0 {
                    face.swap(1, 2);
                    flipped += 1;
                }
            }
        }
    }
    (flipped, checked)
}

/// A non-destructive grade over vertex colours.
///
/// Held open while the sliders are being moved so every change re-grades the
/// colours the terrain had when the session started, rather than compounding
/// on its own output. Dragging exposure up and back down therefore lands
/// exactly where it began.
pub(super) struct VertexPaintGrade {
    targets: Vec<PaintTarget>,
    /// The terrain as it was before the first slider moved.
    baseline: Vec<VertexPaintSnapshot>,
}

/// Exposure, contrast, vibrance and tint, applied in that order.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct VertexPaintGradeSettings {
    /// Stops. Doubling light per stop is how exposure reads everywhere else,
    /// so it is a multiply by two to the power rather than a linear scale.
    pub(super) exposure: f32,
    pub(super) contrast: f32,
    pub(super) vibrance: f32,
    /// Degrees around the colour wheel.
    pub(super) hue: f32,
    /// Positive deepens the dark end, negative lifts it. Weighted by how dark
    /// a vertex already is, so it reaches shadow without touching highlights
    /// the way exposure would.
    pub(super) shadow: f32,
    pub(super) tint: [f32; 3],
    pub(super) tint_amount: f32,
}

impl Default for VertexPaintGradeSettings {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0.0,
            vibrance: 0.0,
            hue: 0.0,
            shadow: 0.0,
            tint: [1.0, 1.0, 1.0],
            tint_amount: 0.0,
        }
    }
}

impl VertexPaintGradeSettings {
    /// Whether the grade would leave the colours untouched.
    fn is_neutral(self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0.0
            && self.vibrance == 0.0
            && self.hue == 0.0
            && self.shadow == 0.0
            && self.tint_amount == 0.0
    }

    /// `base` is the material's own diffuse, which is what makes "shadow"
    /// mean shadow. Weighting by darkness alone treats a surface that is
    /// simply dark by design as though it were in shade, so a black floor
    /// takes the full shadow tint and the grade bleeds across the whole model.
    /// Measured against the diffuse, a vertex sitting at its material colour
    /// is unshaded whatever that colour is.
    pub(super) fn apply(self, color: &mut [f32; 4], base: [f32; 3]) {
        let luma = |value: [f32; 3]| 0.299 * value[0] + 0.587 * value[1] + 0.114 * value[2];
        let unshaded = luma(base).max(1e-3);
        let shading = |value: &[f32; 4]| {
            let lit = luma([value[0], value[1], value[2]]);
            (1.0 - lit / unshaded).clamp(0.0, 1.0)
        };

        let exposure = self.exposure.exp2();
        for channel in color.iter_mut().take(3) {
            *channel *= exposure;
        }

        // Pivoted on mid grey, so contrast pulls light and dark apart instead
        // of just brightening everything.
        let scale = 1.0 + self.contrast;
        for channel in color.iter_mut().take(3) {
            *channel = (*channel - 0.5) * scale + 0.5;
        }

        // Vibrance leans on whatever is already dull. Scaling saturation flat
        // blows out the colours that were already strong, which on baked
        // terrain means the few painted patches go first.
        let grey = luma([color[0], color[1], color[2]]);
        let high = color[0].max(color[1]).max(color[2]);
        let low = color[0].min(color[1]).min(color[2]);
        let reach = 1.0 + self.vibrance * (1.0 - (high - low).clamp(0.0, 1.0));
        for channel in color.iter_mut().take(3) {
            *channel = grey + (*channel - grey) * reach;
        }

        // Rotation about the grey axis, which keeps luminance where it is:
        // spinning hue should recolour a bake, not relight it. The constants
        // are the usual luma weights carried through the rotation.
        if self.hue != 0.0 {
            let (sin, cos) = self.hue.to_radians().sin_cos();
            let rotated: [f32; 3] = std::array::from_fn(|channel| {
                let weights = HUE_ROTATION[channel];
                weights[0] * color[0]
                    + weights[1] * color[1]
                    + weights[2] * color[2]
                    + cos
                        * (HUE_COSINE[channel][0] * color[0]
                            + HUE_COSINE[channel][1] * color[1]
                            + HUE_COSINE[channel][2] * color[2])
                    + sin
                        * (HUE_SINE[channel][0] * color[0]
                            + HUE_SINE[channel][1] * color[1]
                            + HUE_SINE[channel][2] * color[2])
            });
            color[..3].copy_from_slice(&rotated);
        }

        // Shadow after vibrance, before tint. The weight is how dark the
        // vertex is, so an occlusion bake can be deepened or lifted without
        // dragging the lit surfaces with it.
        if self.shadow != 0.0 {
            let weight = shading(color);
            for channel in color.iter_mut().take(3) {
                *channel = match self.shadow > 0.0 {
                    true => *channel * (1.0 - self.shadow * weight),
                    false => *channel + (1.0 - *channel) * (-self.shadow) * weight,
                };
            }
        }

        // Tint rides the dark end only. Tinting everything is a colour cast
        // over the whole mesh, which is what a brush stroke is for; what a
        // grade is wanted for is warming or cooling the shadows an occlusion
        // bake laid down, leaving the lit surfaces where they are.
        if self.tint_amount != 0.0 {
            let weight = shading(color) * self.tint_amount;
            for (channel, tint) in color.iter_mut().take(3).zip(self.tint.iter()) {
                *channel += (tint - *channel) * weight;
            }
        }

        for channel in color.iter_mut().take(3) {
            *channel = channel.clamp(0.0, 1.0);
        }
    }
}

/// A stroke in progress.
///
/// The assets stay in memory for the length of the drag so the viewport can
/// show the paint going down. Only the release writes to disk.
pub(super) struct VertexPaintLiveStroke {
    targets: Vec<PaintTarget>,
    visibility: crate::triangle_bvh::TriangleBvh,
    camera_position: [f32; 3],
    /// How many stroke samples are already folded in, so each frame applies
    /// only what arrived since the last one.
    applied: usize,
    /// Raw pointer endpoint from the previous UI event. The segment to the
    /// next endpoint is resampled in screen space, so frame rate cannot change
    /// paint strength or leave gaps.
    last_pointer: Option<egui::Pos2>,
    /// Distance from `last_pointer` to the next fixed brush dab.
    distance_to_next_sample: f32,
    sample_spacing: f32,
    mode: VertexPaintMode,
}

/// A terrain asset opened for painting, with the world transform of every
/// instance that uses it.
pub(super) struct PaintTarget {
    pub(super) id: AssetId,
    pub(super) document: ModelAssetDocument,
    /// One asset can be placed several times; a stroke has to consider each
    /// placement, and every placement writes back to the same vertices.
    pub(super) transforms: Vec<[[f32; 4]; 4]>,
    /// Placement the user actually selected. Asset edits are shared, but their
    /// world-space brush, bake, and cut calculations must use this transform.
    pub(super) transform: [[f32; 4]; 4],
    /// State as loaded, so the commit can record an undo step without a second
    /// read from disk.
    baseline: VertexPaintSnapshot,
}

/// One vertex of a target, resolved into world space.
#[derive(Clone, Copy)]
pub(super) struct WorldVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
}

struct StrokeApplication<'a> {
    projection: &'a crate::camera::CameraProjection,
    radius: f32,
    strength: f32,
    color: [f32; 3],
    mode: VertexPaintMode,
    visibility: &'a crate::triangle_bvh::TriangleBvh,
    camera_position: [f32; 3],
}

fn resample_stroke_points(
    raw: &[egui::Pos2],
    previous: &mut Option<egui::Pos2>,
    distance_to_next: &mut f32,
    spacing: f32,
) -> Vec<egui::Pos2> {
    let spacing = spacing.max(0.25);
    let mut samples = Vec::new();
    for &point in raw {
        let Some(start) = *previous else {
            samples.push(point);
            *previous = Some(point);
            *distance_to_next = spacing;
            continue;
        };

        let delta = point - start;
        let length = delta.length();
        if length > f32::EPSILON {
            let mut along = (*distance_to_next).max(0.0);
            while along <= length {
                samples.push(start + delta * (along / length));
                along += spacing;
            }
            *distance_to_next = along - length;
        }
        *previous = Some(point);
    }
    samples
}

pub(super) fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row]
    })
}

pub(super) fn transform_direction(matrix: [[f32; 4]; 4], direction: [f32; 3]) -> [f32; 3] {
    let rotated: [f32; 3] = std::array::from_fn(|row| {
        matrix[0][row] * direction[0]
            + matrix[1][row] * direction[1]
            + matrix[2][row] * direction[2]
    });
    let length = vec3_dot(rotated, rotated).sqrt();
    if length > f32::EPSILON {
        vec3_scale(rotated, 1.0 / length)
    } else {
        [0.0, 1.0, 0.0]
    }
}

pub(super) fn transform_normal(matrix: [[f32; 4]; 4], normal: [f32; 3]) -> [f32; 3] {
    let Some(inverse) = invert_affine(matrix) else {
        return [0.0, 1.0, 0.0];
    };
    // Normal vectors travel through the inverse transpose. With column-major
    // storage, the first index below selects the inverse row that becomes a
    // column after transposition.
    let transformed: [f32; 3] = std::array::from_fn(|row| {
        (0..3)
            .map(|column| inverse[row][column] * normal[column])
            .sum()
    });
    vec3_normalize(transformed)
}

/// Colour set 0 for a primitive, created opaque white when the mesh arrived
/// without one. White is the identity for a vertex-lit surface, so an
/// untouched model looks the same before and after the set exists.
fn primitive_colors_mut(primitive: &mut sms_authoring::ModelPrimitive) -> &mut Vec<[f32; 4]> {
    if !primitive.colors.iter().any(|set| set.set == 0) {
        primitive.colors.push(sms_authoring::ColorSet {
            set: 0,
            values: vec![[1.0, 1.0, 1.0, 1.0]; primitive.positions.len()],
        });
    }
    let values = &mut primitive
        .colors
        .iter_mut()
        .find(|set| set.set == 0)
        .expect("colour set 0 was just ensured")
        .values;
    // An imported set can be shorter than the position list if the source only
    // coloured part of the mesh.
    values.resize(primitive.positions.len(), [1.0, 1.0, 1.0, 1.0]);
    values
}

/// GX_SRC_VTX. `GXColorSrc` in the decomp is `{ GX_SRC_REG = 0, GX_SRC_VTX = 1 }`,
/// so a channel left on the default register source ignores the vertex colour
/// array entirely.
const GX_SRC_VTX: u8 = 1;

/// `GX_CULL_NONE`. The importer writes 2, `GX_CULL_BACK`, for anything the
/// source did not mark double sided.
const GX_CULL_NONE: u32 = 0;

/// `GX_CULL_BACK`, what the importer writes for anything the source did not
/// mark double sided.
const GX_CULL_BACK: u32 = 2;

/// An untextured material whose single TEV stage passes the rasterised colour
/// straight through, matching what the importer builds for a textureless
/// primitive. `GX_CC_RASC` is input 10, and `color_channel` 4 is `GX_COLOR0A0`.
fn vertex_color_material(name: &str) -> sms_authoring::ModelMaterial {
    let mut gx = sms_formats::GxMaterial {
        name: format!("{name}_vertex_color"),
        cull_mode: 2,
        color_channel_count: 1,
        material_colors: [Some([255, 255, 255, 255]), None],
        color_channels: [
            Some(sms_formats::GxColorChannel::default()),
            Some(sms_formats::GxColorChannel::default()),
            None,
            None,
        ],
        ..sms_formats::GxMaterial::default()
    };
    gx.tev_orders[0] = Some(sms_formats::GxTevOrder {
        tex_coord: None,
        tex_map: None,
        color_channel: 4,
    });
    if let Some(stage) = &mut gx.tev_stages[0] {
        stage.color_inputs = [10, 15, 15, 15];
        stage.alpha_inputs = [5, 7, 7, 7];
    }
    sms_authoring::ModelMaterial {
        gx,
        source_base_color: [1.0, 1.0, 1.0, 1.0],
        base_color_texture: None,
        vertex_color_set: Some(0),
        source_double_sided: false,
        source_alpha_mode: sms_authoring::ImportedAlphaMode::Opaque,
        source_pbr: sms_authoring::SourcePbrMetadata {
            metallic_factor: 0.0,
            roughness_factor: 1.0,
            has_metallic_roughness_texture: false,
            has_normal_texture: false,
            has_occlusion_texture: false,
            emissive_factor: [0.0; 3],
            has_emissive_texture: false,
        },
    }
}

/// Points every material used by a coloured primitive at the vertex colour
/// array.
///
/// Without this the paint is written, compiled into VTX1 and then ignored: the
/// channel still reads its colour from the material register, so nothing
/// changes in the viewport or in game. The editor preview compiles the document
/// to BMD and parses it back, so the same switch drives both.
fn enable_vertex_colors(document: &mut ModelAssetDocument) {
    // A textureless glb can import with no materials at all, and a primitive
    // with no material compiles against a default channel that reads the
    // register. Painting such a mesh stored colour that nothing could ever
    // sample, so give it a material first and point the primitives at it.
    let needs_fallback = document
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.primitives)
        .any(|primitive| primitive.material.is_none());
    let fallback = needs_fallback.then(|| {
        document
            .materials
            .push(vertex_color_material(&document.name));
        (document.materials.len() - 1) as u32
    });
    for mesh in &mut document.meshes {
        for primitive in &mut mesh.primitives {
            if primitive.material.is_none() {
                primitive.material = fallback;
            }
        }
    }

    let mut used: Vec<u32> = Vec::new();
    for mesh in &document.meshes {
        for primitive in &mesh.primitives {
            if let Some(material) = primitive.material {
                if !used.contains(&material) {
                    used.push(material);
                }
            }
        }
    }
    let seed_colors = used
        .iter()
        .filter_map(|index| {
            let material = document.materials.get(*index as usize)?;
            let channel = material.gx.color_channels[0].unwrap_or_default();
            (channel.material_source != GX_SRC_VTX).then(|| {
                let factor = material.gx.material_colors[0]
                    .unwrap_or([255; 4])
                    .map(|value| f32::from(value) / 255.0);
                (*index, factor)
            })
        })
        .collect::<BTreeMap<_, _>>();
    // GX channels choose either the material register or the vertex array;
    // switching to GX_SRC_VTX does not multiply the two. Fold the old register
    // factor into COLOR0 once so untouched vertices retain the imported tint
    // and alpha. Existing imported vertex colours are multiplied by the same
    // factor, matching their appearance before the source switch.
    for mesh in &mut document.meshes {
        for primitive in &mut mesh.primitives {
            let Some(factor) = primitive
                .material
                .and_then(|index| seed_colors.get(&index).copied())
            else {
                continue;
            };
            for color in primitive_colors_mut(primitive) {
                for channel in 0..4 {
                    color[channel] = (color[channel] * factor[channel]).clamp(0.0, 1.0);
                }
            }
        }
    }
    for index in used {
        let Some(material) = document.materials.get_mut(index as usize) else {
            continue;
        };
        material.vertex_color_set = Some(0);
        let channel = material.gx.color_channels[0].get_or_insert_with(Default::default);
        // `enable` is GXSetChanCtrl's *lighting* enable, not a channel switch.
        // Turning it on with no lights bound resolves the channel to ambient
        // only, which greys the surface out; leaving it off passes the source
        // colour straight through, which is what vertex paint wants.
        channel.enable = 0;
        channel.material_source = GX_SRC_VTX;
        if material.gx.color_channel_count == 0 {
            material.gx.color_channel_count = 1;
        }
    }
}

/// Nearest placement-surface hit under a viewport ray, with the geometric
/// normal of the triangle that was hit.
///
/// The brush ring needs the normal so it can lie on the surface rather than
/// always lying flat, and a wall gets a ring standing on the wall.
fn nearest_surface_hit(
    triangles: &[PreviewTriangle],
    origin: [f32; 3],
    direction: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let mut best: Option<(f32, [f32; 3], [f32; 3])> = None;
    for triangle in triangles
        .iter()
        .filter(|triangle| preview_triangle_frames_object(triangle))
    {
        let [a, b, c] = triangle.vertices;
        // Moller-Trumbore.
        let edge_a = vec3_sub(b, a);
        let edge_b = vec3_sub(c, a);
        let pvec = vec3_cross(direction, edge_b);
        let determinant = vec3_dot(edge_a, pvec);
        if determinant.abs() < 1e-6 {
            continue;
        }
        let inverse = 1.0 / determinant;
        let tvec = vec3_sub(origin, a);
        let u = vec3_dot(tvec, pvec) * inverse;
        if !(-1e-4..=1.0 + 1e-4).contains(&u) {
            continue;
        }
        let qvec = vec3_cross(tvec, edge_a);
        let v = vec3_dot(direction, qvec) * inverse;
        if v < -1e-4 || u + v > 1.0 + 1e-4 {
            continue;
        }
        let distance = vec3_dot(edge_b, qvec) * inverse;
        if distance <= 0.0 {
            continue;
        }
        if best.is_none_or(|(nearest, _, _)| distance < nearest) {
            let normal = vec3_cross(edge_a, edge_b);
            let length = vec3_dot(normal, normal).sqrt();
            let normal = if length > f32::EPSILON {
                vec3_scale(normal, 1.0 / length)
            } else {
                [0.0, 1.0, 0.0]
            };
            best = Some((
                distance,
                vec3_add(origin, vec3_scale(direction, distance)),
                normal,
            ));
        }
    }
    best.map(|(_, position, normal)| (position, normal))
}

fn vertex_visible_from(
    camera_position: [f32; 3],
    vertex_position: [f32; 3],
    visibility: &crate::triangle_bvh::TriangleBvh,
) -> bool {
    let camera_to_vertex = vec3_sub(vertex_position, camera_position);
    let distance = vec3_dot(camera_to_vertex, camera_to_vertex).sqrt();
    let tolerance = (distance * 1e-4).max(0.05);
    distance <= tolerance
        || !visibility.ray_hits(
            camera_position,
            vec3_scale(camera_to_vertex, 1.0 / distance),
            distance - tolerance,
        )
}

const IDENTITY_MATRIX: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Inverse of an affine transform, or nothing if it is degenerate.
///
/// Cutting runs in the target mesh's own space so split positions can be
/// written straight back into the primitive, which means carrying the cutter
/// the other way through this.
pub(super) fn invert_affine(matrix: [[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    // Column major: matrix[column][row].
    let linear = |row: usize, column: usize| matrix[column][row];
    let determinant = linear(0, 0) * (linear(1, 1) * linear(2, 2) - linear(1, 2) * linear(2, 1))
        - linear(0, 1) * (linear(1, 0) * linear(2, 2) - linear(1, 2) * linear(2, 0))
        + linear(0, 2) * (linear(1, 0) * linear(2, 1) - linear(1, 1) * linear(2, 0));
    if determinant.abs() <= 1e-12 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let cofactor = |row: usize, column: usize| {
        let rows = [(row + 1) % 3, (row + 2) % 3];
        let columns = [(column + 1) % 3, (column + 2) % 3];
        (linear(rows[0], columns[0]) * linear(rows[1], columns[1])
            - linear(rows[0], columns[1]) * linear(rows[1], columns[0]))
            * inverse
    };
    // Transposed cofactors give the inverse of the linear part.
    let mut result: [[f32; 4]; 4] = std::array::from_fn(|column| {
        std::array::from_fn(|row| match (row < 3, column < 3) {
            (true, true) => cofactor(column, row),
            _ => 0.0,
        })
    });
    let translation: [f32; 3] = std::array::from_fn(|axis| matrix[3][axis]);
    result[3] = std::array::from_fn(|row| match row < 3 {
        true => {
            -(result[0][row] * translation[0]
                + result[1][row] * translation[1]
                + result[2][row] * translation[2])
        }
        false => 1.0,
    });
    Some(result)
}

/// Column-major 4x4 multiply, `a` applied after `b`.
pub(super) fn multiply_matrix(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            (0..4)
                .map(|index| a[index][row] * b[column][index])
                .sum::<f32>()
        })
    })
}

/// World transform of every mesh in a document, by mesh index.
///
/// Compilation applies each node's global transform to its primitives, so raw
/// primitive positions are not where the mesh is drawn. Screen-space painting
/// projects vertices, and skipping this put every projected vertex somewhere
/// the mesh is not.
pub(super) fn mesh_node_transforms(
    document: &ModelAssetDocument,
) -> BTreeMap<u32, Vec<[[f32; 4]; 4]>> {
    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut globals: Vec<[[f32; 4]; 4]> = Vec::with_capacity(document.nodes.len());
    for index in 0..document.nodes.len() {
        // Walk to the root, then apply the chain downward. Depth is tiny and
        // the parent link is already resolved, so this stays simple.
        let mut chain = Vec::new();
        let mut cursor = Some(index);
        let mut guard = 0usize;
        while let Some(current) = cursor {
            chain.push(current);
            cursor = document.nodes[current].parent.map(|parent| parent as usize);
            guard += 1;
            if guard > document.nodes.len() {
                break;
            }
        }
        let mut transform = IDENTITY;
        for node in chain.into_iter().rev() {
            transform = multiply_matrix(transform, document.nodes[node].local_transform);
        }
        globals.push(transform);
    }
    let mut by_mesh = BTreeMap::new();
    let mut active = vec![document.scene_roots.is_empty(); document.nodes.len()];
    let mut pending = document.scene_roots.clone();
    while let Some(index) = pending.pop() {
        let Some(node) = document.nodes.get(index as usize) else {
            continue;
        };
        if std::mem::replace(&mut active[index as usize], true) {
            continue;
        }
        pending.extend(node.children.iter().copied());
    }
    for (index, node) in document.nodes.iter().enumerate() {
        if !active[index] || node.purpose != NodePurpose::Render {
            continue;
        }
        if let Some(mesh) = node.mesh {
            by_mesh
                .entry(mesh)
                .or_insert_with(Vec::new)
                .push(globals[index]);
        }
    }
    by_mesh
}

/// Cosine-weighted hemisphere directions around `normal`.
///
/// Hammersley rather than random sampling: the same vertex has to bake the
/// same value every time, and a low discrepancy set covers the hemisphere more
/// evenly than random for the same ray count.
fn hemisphere_directions(normal: [f32; 3], count: u32) -> Vec<[f32; 3]> {
    let reference = match normal[1].abs() > 0.9 {
        true => [1.0, 0.0, 0.0],
        false => [0.0, 1.0, 0.0],
    };
    let tangent = vec3_normalize(vec3_cross(reference, normal));
    let bitangent = vec3_cross(normal, tangent);
    (0..count.max(1))
        .map(|index| {
            let u = (index as f32 + 0.5) / count.max(1) as f32;
            // Radical inverse in base 2.
            let mut bits = index;
            bits = bits.rotate_right(16);
            bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xaaaa_aaaa) >> 1);
            bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xcccc_cccc) >> 2);
            bits = ((bits & 0x0f0f_0f0f) << 4) | ((bits & 0xf0f0_f0f0) >> 4);
            bits = ((bits & 0x00ff_00ff) << 8) | ((bits & 0xff00_ff00) >> 8);
            let v = bits as f32 * 2.328_306_4e-10;

            let radius = u.sqrt();
            let phi = v * std::f32::consts::TAU;
            let (sin, cos) = phi.sin_cos();
            vec3_normalize(vec3_add(
                vec3_scale(normal, (1.0 - u).max(0.0).sqrt()),
                vec3_add(
                    vec3_scale(tangent, radius * cos),
                    vec3_scale(bitangent, radius * sin),
                ),
            ))
        })
        .collect()
}

impl SmsEditorApp {
    /// Assets behind every instance exported as map terrain.
    ///
    /// The stage terrain is composed from all of them, so a paint operation
    /// covers the lot rather than whichever one happens to be selected.
    /// Terrain assets to operate on.
    ///
    /// `selection_only` restricts to the selected instance's asset. Every
    /// paint and bake uses it: stage terrain overlaps in world space, so an
    /// unscoped operation aimed at a ramp also rewrites the floor beneath it.
    /// Only preparing a stage on tool open leaves it false, since that has to
    /// reach every asset before anything is selected.
    pub(super) fn terrain_paint_targets_scoped(&self, selection_only: bool) -> Vec<PaintTarget> {
        let Some(catalog) = self.model_catalog().ok() else {
            return Vec::new();
        };
        // A stroke with no selection used to fall through to every terrain
        // asset, which is how a brush aimed at a ramp ended up repainting the
        // floor. Scoped operations now require a selection outright.
        let selected = match selection_only {
            true => match self.selected_model_instance() {
                Some(instance) => Some((instance.placement.asset_id, instance.placement.transform)),
                None => return Vec::new(),
            },
            false => None,
        };
        let mut by_asset: BTreeMap<AssetId, Vec<[[f32; 4]; 4]>> = BTreeMap::new();
        for instance in self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| {
                instance.placement.export_mode == ModelInstanceExportMode::MapTerrain
            })
            .filter(|instance| {
                selected.is_none_or(|(asset, _)| instance.placement.asset_id == asset)
            })
        {
            by_asset
                .entry(instance.placement.asset_id)
                .or_default()
                .push(instance.placement.transform);
        }
        by_asset
            .into_iter()
            .filter_map(|(id, transforms)| {
                let transform = selected
                    .filter(|(asset, _)| *asset == id)
                    .map(|(_, transform)| transform)
                    .or_else(|| transforms.first().copied())
                    .unwrap_or(IDENTITY_MATRIX);
                catalog.load_asset(id).ok().map(|document| PaintTarget {
                    id,
                    baseline: snapshot_paint_state(id, &document),
                    document,
                    transforms,
                    transform,
                })
            })
            .collect()
    }

    /// World-space occurrences of every primitive vertex in the selected
    /// asset placement. A glTF mesh may be referenced by several nodes, so one
    /// stored vertex can appear at several world positions.
    fn target_world_vertex_occurrences(target: &PaintTarget) -> Vec<Vec<Vec<WorldVertex>>> {
        Self::target_world_vertex_occurrences_with_transform(target, target.transform)
    }

    fn target_world_vertex_occurrences_with_transform(
        target: &PaintTarget,
        transform: [[f32; 4]; 4],
    ) -> Vec<Vec<Vec<WorldVertex>>> {
        let nodes = mesh_node_transforms(&target.document);
        target
            .document
            .meshes
            .iter()
            .enumerate()
            .flat_map(|(mesh_index, mesh)| {
                let node_transforms = nodes.get(&(mesh_index as u32)).cloned().unwrap_or_default();
                mesh.primitives
                    .iter()
                    .map(move |primitive| {
                        node_transforms
                            .iter()
                            .map(|node| {
                                // The node transform runs first, exactly as
                                // compilation does it, then the placement puts
                                // it in the stage.
                                let combined = multiply_matrix(transform, *node);
                                primitive
                                    .positions
                                    .iter()
                                    .enumerate()
                                    .map(|(index, position)| WorldVertex {
                                        position: transform_point(combined, *position),
                                        normal: transform_normal(
                                            combined,
                                            primitive
                                                .normals
                                                .get(index)
                                                .copied()
                                                .unwrap_or([0.0, 1.0, 0.0]),
                                        ),
                                    })
                                    .collect()
                            })
                            .collect()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Files an undo step for a paint operation, or folds it into the open
    /// group if one is running.
    fn record_vertex_paint_undo(
        &mut self,
        label: &str,
        before: Vec<VertexPaintSnapshot>,
        after: Vec<VertexPaintSnapshot>,
    ) {
        if let Some(group) = &mut self.vertex_paint_undo_group {
            // A bake that smooths normals first commits twice. Keeping the
            // earliest `before` and the latest `after` makes it one undo press,
            // which is what the button looks like from outside.
            for snapshot in before {
                if !group.before.iter().any(|entry| entry.id == snapshot.id) {
                    group.before.push(snapshot);
                }
            }
            for snapshot in after {
                match group.after.iter_mut().find(|entry| entry.id == snapshot.id) {
                    Some(entry) => *entry = snapshot,
                    None => group.after.push(snapshot),
                }
            }
            return;
        }
        self.push_vertex_paint_undo(VertexPaintUndoRecord {
            label: label.to_string(),
            before,
            after,
        });
    }

    fn push_vertex_paint_undo(&mut self, record: VertexPaintUndoRecord) {
        if record.before.is_empty() {
            return;
        }
        self.vertex_paint_undo_stack.push_back(record);
        if self.vertex_paint_undo_stack.len() > 40 {
            self.vertex_paint_undo_stack.pop_front();
        }
        self.vertex_paint_redo_stack.clear();
    }

    pub(super) fn invalidate_terrain_asset_undo_history(&mut self, reason: &str) {
        let had_history = !self.vertex_paint_undo_stack.is_empty()
            || !self.vertex_paint_redo_stack.is_empty()
            || self.vertex_paint_undo_group.is_some();
        self.vertex_paint_undo_stack.clear();
        self.vertex_paint_redo_stack.clear();
        self.vertex_paint_undo_group = None;
        if had_history {
            self.log
                .push(format!("Cleared terrain edit history because {reason}."));
        }
    }

    /// Collects every commit until [`Self::end_vertex_paint_undo_group`] into a
    /// single undo step.
    fn begin_vertex_paint_undo_group(&mut self) {
        if self.vertex_paint_undo_group.is_none() {
            self.vertex_paint_undo_group = Some(VertexPaintUndoRecord {
                label: String::new(),
                before: Vec::new(),
                after: Vec::new(),
            });
        }
    }

    fn end_vertex_paint_undo_group(&mut self, label: &str) {
        let Some(mut group) = self.vertex_paint_undo_group.take() else {
            return;
        };
        group.label = label.to_string();
        self.push_vertex_paint_undo(group);
    }

    pub(super) fn undo_vertex_paint(&mut self) -> bool {
        let Some(record) = self.vertex_paint_undo_stack.pop_back() else {
            return false;
        };
        let label = format!("Undo {}", record.label.to_lowercase());
        match self.apply_vertex_paint_snapshots(&record.before, &record.after, &label) {
            SnapshotApplyResult::Applied => {
                self.vertex_paint_redo_stack.push_back(record);
                true
            }
            SnapshotApplyResult::Stale => {
                self.invalidate_terrain_asset_undo_history(
                    "a terrain asset changed after that edit",
                );
                true
            }
            SnapshotApplyResult::Failed => {
                self.vertex_paint_undo_stack.push_back(record);
                false
            }
        }
    }

    pub(super) fn redo_vertex_paint(&mut self) -> bool {
        let Some(record) = self.vertex_paint_redo_stack.pop_back() else {
            return false;
        };
        let label = format!("Redo {}", record.label.to_lowercase());
        match self.apply_vertex_paint_snapshots(&record.after, &record.before, &label) {
            SnapshotApplyResult::Applied => {
                self.vertex_paint_undo_stack.push_back(record);
                true
            }
            SnapshotApplyResult::Stale => {
                self.invalidate_terrain_asset_undo_history(
                    "a terrain asset changed after that edit",
                );
                true
            }
            SnapshotApplyResult::Failed => {
                self.vertex_paint_redo_stack.push_back(record);
                false
            }
        }
    }

    /// Writes snapshots back to their assets and refreshes the preview.
    fn apply_vertex_paint_snapshots(
        &mut self,
        snapshots: &[VertexPaintSnapshot],
        expected: &[VertexPaintSnapshot],
        label: &str,
    ) -> SnapshotApplyResult {
        if !self.content_catalog_mutation_allowed(label) {
            return SnapshotApplyResult::Failed;
        }
        let Ok(catalog) = self.model_catalog() else {
            return SnapshotApplyResult::Failed;
        };
        if snapshots.len() != expected.len() {
            self.log.push(format!(
                "{label} was not applied because its terrain revision set is incomplete."
            ));
            return SnapshotApplyResult::Stale;
        }
        // Preflight every asset before writing any of them. A material edit,
        // source replacement, or later terrain operation must invalidate this
        // full-document snapshot rather than being silently overwritten.
        for revision in expected {
            let Ok(document) = catalog.load_asset(revision.id) else {
                return SnapshotApplyResult::Failed;
            };
            if document != revision.document {
                self.log.push(format!(
                    "{label} was not applied because terrain asset {} has newer edits.",
                    revision.id
                ));
                return SnapshotApplyResult::Stale;
            }
        }
        let mut restored = 0usize;
        for snapshot in snapshots {
            let document = snapshot.document.clone();
            match catalog.save_asset(snapshot.id, &document) {
                Ok(_) => {
                    restored += 1;
                    self.model_asset_preview_cache
                        .retain(|key, _| key.asset_id != snapshot.id);
                }
                Err(error) => self
                    .log
                    .push(format!("Could not restore painted asset: {error}")),
            }
        }
        if restored != snapshots.len() {
            self.log.push(format!(
                "{label} restored {restored} of {} terrain asset(s); the undo record was retained for another attempt.",
                snapshots.len()
            ));
            return SnapshotApplyResult::Failed;
        }
        self.force_refresh_model_catalog();
        self.rebuild_model_preview_cache();
        self.log
            .push(format!("{label} across {restored} terrain asset(s)."));
        SnapshotApplyResult::Applied
    }

    /// Writes every modified target back to the catalog.
    ///
    /// Topology-only callers must leave `enable_colors` false so a cut or
    /// subdivision does not silently rewrite the asset's GX materials.
    pub(super) fn commit_terrain_targets(
        &mut self,
        targets: Vec<PaintTarget>,
        label: &str,
        enable_colors: bool,
    ) {
        if targets.is_empty() {
            return;
        }
        if !self.content_catalog_mutation_allowed(label) {
            return;
        }
        let Ok(catalog) = self.model_catalog() else {
            return;
        };
        // A live stroke spans multiple frames, so another in-app operation can
        // replace the same asset after the stroke loaded its document. Never
        // let the stale in-memory target overwrite that newer revision.
        for target in &targets {
            match catalog.load_asset(target.id) {
                Ok(current) if current == target.baseline.document => {}
                Ok(_) => {
                    self.log.push(format!(
                        "{label} was not applied because terrain asset {} changed while the edit \
                         was in progress.",
                        target.id
                    ));
                    self.invalidate_terrain_asset_undo_history(
                        "a terrain asset changed while an edit was in progress",
                    );
                    for target in &targets {
                        self.model_asset_preview_cache
                            .retain(|key, _| key.asset_id != target.id);
                    }
                    self.rebuild_model_preview_cache();
                    return;
                }
                Err(error) => {
                    self.log.push(format!(
                        "{label} was not applied because terrain asset {} could not be reloaded: \
                         {error}",
                        target.id
                    ));
                    for target in &targets {
                        self.model_asset_preview_cache
                            .retain(|key, _| key.asset_id != target.id);
                    }
                    self.rebuild_model_preview_cache();
                    return;
                }
            }
        }
        let mut saved = 0usize;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for mut target in targets {
            if enable_colors {
                enable_vertex_colors(&mut target.document);
            }
            match catalog.save_asset(target.id, &target.document) {
                Ok(_) => {
                    saved += 1;
                    before.push(target.baseline);
                    after.push(snapshot_paint_state(target.id, &target.document));
                    self.model_asset_preview_cache
                        .retain(|key, _| key.asset_id != target.id);
                }
                Err(error) => self
                    .log
                    .push(format!("Could not save painted asset: {error}")),
            }
        }
        if saved == 0 {
            return;
        }
        self.record_vertex_paint_undo(label, before, after);
        self.force_refresh_model_catalog();
        self.rebuild_model_preview_cache();
        self.log
            .push(format!("{label} across {saved} terrain asset(s)."));
    }

    /// Runs `edit` over every terrain vertex, giving it the world position,
    /// world normal and the colour to modify.
    fn edit_terrain_vertex_colors_scoped(
        &mut self,
        label: &str,
        selection_only: bool,
        mut edit: impl FnMut(&[WorldVertex], &mut [f32; 4]),
    ) {
        let mut targets = self.terrain_paint_targets_scoped(selection_only);
        if targets.is_empty() {
            self.log.push(
                "No terrain to paint: set a model instance to 'Bake as map terrain' first."
                    .to_string(),
            );
            return;
        }
        for target in &mut targets {
            enable_vertex_colors(&mut target.document);
            let world = Self::target_world_vertex_occurrences(target);
            let mut primitive_index = 0usize;
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let occurrences = &world[primitive_index];
                    let colors = primitive_colors_mut(primitive);
                    for (index, color) in colors.iter_mut().enumerate() {
                        let vertices = occurrences
                            .iter()
                            .filter_map(|vertices| vertices.get(index).copied())
                            .collect::<Vec<_>>();
                        if !vertices.is_empty() {
                            edit(&vertices, color);
                        }
                    }
                    primitive_index += 1;
                }
            }
        }
        self.commit_terrain_targets(targets, label, true);
    }

    /// Blends every vertex toward the average of its edge neighbours.
    pub(super) fn smooth_terrain_vertex_colors(&mut self) {
        let amount = self.vertex_paint_strength.clamp(0.0, 1.0);
        let iterations = self.vertex_paint_smooth_iterations.clamp(1, 20);
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "No terrain to paint: set a model instance to 'Bake as map terrain' first."
                    .to_string(),
            );
            return;
        }
        for target in &mut targets {
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let welds = weld_positions(&primitive.positions);
                    let adjacency = primitive_adjacency(&primitive.indices, &welds);
                    let colors = primitive_colors_mut(primitive);
                    // Repeated passes spread the blend further than one pass
                    // at higher strength, which just overshoots.
                    for _ in 0..iterations {
                        let smoothed = smoothed_colors(colors, &welds, &adjacency, amount);
                        colors.copy_from_slice(&smoothed);
                    }
                }
            }
        }
        self.commit_terrain_targets(targets, "Smoothed vertex paint", true);
    }

    /// Resets every terrain vertex to opaque white.
    /// Rewinds terrain triangles that disagree with their own vertex normals.
    pub(super) fn repair_terrain_winding(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "Select a terrain instance in the hierarchy before repairing winding.".to_string(),
            );
            return;
        }
        let mut flipped = 0usize;
        let mut checked = 0usize;
        for target in &mut targets {
            let (mesh_flipped, mesh_checked) = repair_document_winding(&mut target.document);
            flipped += mesh_flipped;
            checked += mesh_checked;
        }

        if flipped == 0 {
            self.log.push(format!(
                "Winding already agrees with the normals across {checked} triangle(s); nothing \
                 to repair."
            ));
            return;
        }
        // Geometry only, so the colour channel switch is left alone.
        self.commit_terrain_targets(targets, "Repaired terrain winding", false);
        self.log.push(format!(
            "Rewound {flipped} of {checked} triangle(s) to face the way their normals point."
        ));
    }

    /// Whether the selected terrain draws both sides, reading the asset once
    /// and remembering the answer.
    ///
    /// The panel asks every frame, and the answer lives in the asset on disk,
    /// so it is cached against the id it was read for and re-read only when
    /// the selection moves or this tool changes it.
    fn terrain_draws_both_sides(&mut self) -> Option<bool> {
        let asset = self
            .selected_model_instance()
            .map(|instance| instance.placement.asset_id)?;
        if let Some((cached, value)) = self.vertex_paint_double_sided {
            if cached == asset {
                return Some(value);
            }
        }
        let document = self.model_catalog().ok()?.load_asset(asset).ok()?;
        // An empty material list is the textureless case, which paints against
        // a material this tool synthesises later; report it as single sided so
        // the toggle still offers the fix.
        let both = !document.materials.is_empty()
            && document
                .materials
                .iter()
                .all(|material| material.gx.cull_mode == GX_CULL_NONE);
        self.vertex_paint_double_sided = Some((asset, both));
        Some(both)
    }

    /// Turns culling on or off for the selected terrain's materials.
    ///
    /// Baking as terrain is where a single-sided material starts costing you
    /// faces: the same mesh drawn as a separate object keeps them, so the two
    /// modes disagree and the terrain reads as inside out. Drawing both sides
    /// makes them agree no matter which way any given triangle is wound, which
    /// is the one repair that does not depend on trusting the normals.
    ///
    /// It is not free. Both sides of every triangle get rasterised, so reach
    /// for `Fix Facing` first and keep this for meshes it cannot resolve.
    pub(super) fn set_terrain_double_sided(&mut self, both: bool) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "Select a terrain instance in the hierarchy before changing its culling."
                    .to_string(),
            );
            return;
        }
        let wanted = match both {
            true => GX_CULL_NONE,
            false => GX_CULL_BACK,
        };
        let mut changed = 0usize;
        for target in &mut targets {
            for material in &mut target.document.materials {
                if material.gx.cull_mode != wanted {
                    material.gx.cull_mode = wanted;
                    material.source_double_sided = both;
                    changed += 1;
                }
            }
        }
        // The cache is stale either way now: the write may have changed
        // nothing, in which case the state was already what was asked for.
        self.vertex_paint_double_sided = None;
        if changed == 0 {
            self.log.push(match both {
                true => "That terrain already draws both sides.".to_string(),
                false => "That terrain already culls back faces.".to_string(),
            });
            return;
        }
        let label = match both {
            true => "Made terrain double sided",
            false => "Made terrain single sided",
        };
        self.commit_terrain_targets(targets, label, false);
        self.log.push(match both {
            true => format!(
                "{changed} material(s) now draw both sides, so no face drops out of the bake."
            ),
            false => format!("{changed} material(s) now cull back faces again."),
        });
    }

    /// Turns blended terrain materials back into opaque ones.
    ///
    /// A glTF exporter will mark a material `BLEND` for having any alpha
    /// plumbing at all, whether or not anything is actually transparent, and
    /// the importer maps that faithfully: blending on and depth writes off.
    /// Terrain then stops occluding itself and the stage looks like its faces
    /// are missing, because you are seeing through them rather than at them.
    ///
    /// The separate-object path hides this. Its loader flags derive
    /// pixel-engine state from material mode and overwrite the stored blend
    /// and depth state, so the same asset looks right there and wrong once it
    /// bakes as terrain.
    ///
    /// Materials whose base colour is genuinely translucent are left alone, so
    /// this cannot quietly turn real glass into a wall.
    pub(super) fn make_terrain_opaque(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "Select a terrain instance in the hierarchy before changing its blending."
                    .to_string(),
            );
            return;
        }
        let mut changed = 0usize;
        let mut kept = 0usize;
        for target in &mut targets {
            for material in &mut target.document.materials {
                let blended = material.gx.blend_mode != sms_formats::GxBlendMode::default()
                    || material.gx.depth_mode.update_enabled == 0;
                if !blended {
                    continue;
                }
                if material.source_base_color[3] < 0.999 {
                    kept += 1;
                    continue;
                }
                material.gx.blend_mode = sms_formats::GxBlendMode::default();
                material.gx.depth_mode.update_enabled = 1;
                material.source_alpha_mode = sms_authoring::ImportedAlphaMode::Opaque;
                changed += 1;
            }
        }

        if changed == 0 {
            self.log.push(match kept {
                0 => "That terrain is already opaque.".to_string(),
                _ => format!(
                    "Left {kept} translucent material(s) alone; nothing else on that terrain                      was blended."
                ),
            });
            return;
        }
        self.commit_terrain_targets(targets, "Made terrain opaque", false);
        let mut message =
            format!("{changed} material(s) now write depth again, so the terrain occludes itself.");
        if kept > 0 {
            message.push_str(&format!(
                " Left {kept} alone for having a genuinely translucent base colour."
            ));
        }
        self.log.push(message);
    }

    pub(super) fn clear_terrain_vertex_colors(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "No terrain to clear: set a model instance to 'Bake as map terrain' first."
                    .to_string(),
            );
            return;
        }
        // Back to the material's own diffuse, not to white. Painting points the
        // colour channel at the vertex array, so the material register stops
        // contributing; clearing to flat white therefore threw the model's
        // diffuse away along with the paint, and the mesh came back blank.
        for target in &mut targets {
            let diffuse = target
                .document
                .materials
                .iter()
                .map(|material| material.source_base_color)
                .collect::<Vec<_>>();
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let base = primitive
                        .material
                        .and_then(|index| diffuse.get(index as usize))
                        .copied()
                        .unwrap_or([1.0; 4]);
                    for color in primitive_colors_mut(primitive) {
                        *color = [base[0], base[1], base[2], color[3]];
                    }
                }
            }
        }
        self.commit_terrain_targets(targets, "Cleared vertex paint", true);
    }

    /// Directional light baked into vertex colours.
    ///
    /// Ported from Affinity: a hard lambert term and a half-lambert wrap term
    /// blended by softness, then applied as a shade multiplier scaled by the
    /// shadow amount, so shadow 0 leaves the colours untouched.
    pub(super) fn bake_terrain_sun(&mut self) {
        let (yaw, pitch) = (
            self.vertex_paint_sun_yaw.to_radians(),
            self.vertex_paint_sun_pitch.to_radians(),
        );
        let light = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let softness = self.vertex_paint_sun_softness.clamp(0.0, 1.0);
        let shadow = self.vertex_paint_sun_shadow.clamp(0.0, 1.0);
        let smooth_normals = self.vertex_paint_sun_smooth_normals;
        self.begin_vertex_paint_undo_group();
        if smooth_normals {
            // Faceted meshes carry a face normal on every vertex, so N.L jumps
            // at each edge and the bake comes out hard-edged. Averaging the
            // normals across welded positions first gives the gradient a
            // smooth-shaded mesh would have had.
            self.smooth_terrain_normals_for_bake();
        }
        self.edit_terrain_vertex_colors_scoped(
            "Baked sun into vertex paint",
            true,
            |vertices, color| {
                let shade = vertices
                    .iter()
                    .map(|vertex| {
                        let lambert = vec3_dot(vertex.normal, light);
                        let hard = lambert.max(0.0);
                        let wrap = lambert * 0.5 + 0.5;
                        let blended = (hard + (wrap - hard) * softness).clamp(0.0, 1.0);
                        (1.0 - shadow) + shadow * blended
                    })
                    .sum::<f32>()
                    / vertices.len() as f32;
                for channel in color.iter_mut().take(3) {
                    *channel = (*channel * shade).clamp(0.0, 1.0);
                }
            },
        );
        self.end_vertex_paint_undo_group("Baked sun into vertex paint");
    }

    /// Darkens creases, following Blender's "Dirty Vertex Colors".
    /// Raycast ambient occlusion baked into vertex colours.
    ///
    /// The cavity bake infers edges from how a vertex's neighbours sit against
    /// its normal, so it answers "how sharp is this fold" rather than "what is
    /// actually blocking this vertex". That makes it sensitive to triangulation
    /// and to how sharp one fold is against another, which is why two arms of
    /// the same ramp bake differently. This asks the question directly: fire
    /// rays over the hemisphere and count what they hit.
    ///
    /// Occluders come from all terrain, not just the selection, so a floor
    /// correctly darkens the ramp standing on it. Only the selected asset is
    /// written.
    pub(super) fn bake_terrain_occlusion(&mut self) {
        let amount = self.vertex_paint_ao_strength.clamp(0.0, 1.0);
        let reach = self.vertex_paint_ao_distance.max(1.0);
        let rays = self.vertex_paint_ao_rays.clamp(8, 256);

        // Every terrain triangle in the stage, in world space.
        let mut occluders: Vec<[[f32; 3]; 3]> = Vec::new();
        for target in &self.terrain_paint_targets_scoped(false) {
            for transform in &target.transforms {
                let world =
                    Self::target_world_vertex_occurrences_with_transform(target, *transform);
                let mut primitive_index = 0usize;
                for mesh in &target.document.meshes {
                    for primitive in &mesh.primitives {
                        let Some(occurrences) = world.get(primitive_index) else {
                            continue;
                        };
                        primitive_index += 1;
                        for vertices in occurrences {
                            for triangle in primitive.indices.chunks_exact(3) {
                                let corners = [
                                    vertices.get(triangle[0] as usize),
                                    vertices.get(triangle[1] as usize),
                                    vertices.get(triangle[2] as usize),
                                ];
                                if let [Some(a), Some(b), Some(c)] = corners {
                                    occluders.push([a.position, b.position, c.position]);
                                }
                            }
                        }
                    }
                }
            }
        }
        if occluders.is_empty() {
            self.log
                .push("No terrain to occlude: nothing is flagged as map terrain.".to_string());
            return;
        }
        // Indexed, not walked. Every ray would otherwise test every triangle,
        // which is rays times vertices times triangles for a single bake.
        let occluders = crate::triangle_bvh::TriangleBvh::build(occluders);
        self.log.push(format!(
            "Baking occlusion against {} triangles at {rays} rays per vertex...",
            occluders.len()
        ));

        self.edit_terrain_vertex_colors_scoped(
            "Baked occlusion into vertex paint",
            true,
            |vertices, color| {
                let occlusion = vertices
                    .iter()
                    .map(|vertex| {
                        let normal = vec3_normalize(vertex.normal);
                        // Lifted off the surface, or every ray starts by
                        // hitting the triangle it left.
                        let origin =
                            vec3_add(vertex.position, vec3_scale(normal, reach * 0.001 + 0.05));
                        let directions = hemisphere_directions(normal, rays);
                        let blocked = directions
                            .iter()
                            .filter(|direction| occluders.ray_hits(origin, **direction, reach))
                            .count();
                        blocked as f32 / directions.len().max(1) as f32
                    })
                    .sum::<f32>()
                    / vertices.len() as f32;
                let shade = 1.0 - occlusion * amount;
                for channel in color.iter_mut().take(3) {
                    *channel = (*channel * shade).clamp(0.0, 1.0);
                }
            },
        );
    }

    /// Subdivides the selected terrain instance, or all of it when nothing is
    /// selected.
    pub(super) fn subdivide_terrain(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log
                .push("Nothing to subdivide: select a terrain instance first.".to_string());
            return;
        }
        let mut triangles = 0usize;
        for target in &mut targets {
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    subdivide_primitive(primitive);
                    triangles += primitive.indices.len() / 3;
                }
            }
        }
        self.commit_terrain_targets(targets, "Subdivided terrain", false);
        self.log
            .push(format!("Terrain now has {triangles} triangle(s)."));
    }

    /// Averages vertex normals across welded positions, in place.
    ///
    /// Only used ahead of a lighting bake: it changes shading, not geometry,
    /// and a faceted mesh otherwise bakes hard steps at every edge.
    fn smooth_terrain_normals_for_bake(&mut self) {
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            return;
        }
        for target in &mut targets {
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let welds = weld_positions(&primitive.positions);
                    let mut sums: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
                    for (index, weld) in welds.iter().enumerate() {
                        let normal = primitive
                            .normals
                            .get(index)
                            .copied()
                            .unwrap_or([0.0, 1.0, 0.0]);
                        let entry = sums.entry(*weld).or_insert([0.0; 3]);
                        for (sum, axis) in entry.iter_mut().zip(normal.iter()) {
                            *sum += *axis;
                        }
                    }
                    for (index, weld) in welds.iter().enumerate() {
                        let Some(sum) = sums.get(weld) else {
                            continue;
                        };
                        let length = vec3_dot(*sum, *sum).sqrt();
                        if length <= f32::EPSILON {
                            continue;
                        }
                        if let Some(normal) = primitive.normals.get_mut(index) {
                            *normal = vec3_scale(*sum, 1.0 / length);
                        }
                    }
                }
            }
        }
        self.commit_terrain_targets(targets, "Smoothed normals for baking", false);
    }
}

/// Splits every triangle into four by its edge midpoints.
///
/// Vertex colour cannot hold detail finer than the mesh, so a sparse surface
/// simply has nothing for a small brush to write to. One pass quadruples the
/// triangle count and halves the spacing; attributes are interpolated so the
/// surface, its UVs and any existing paint are unchanged.
fn subdivide_primitive(primitive: &mut sms_authoring::ModelPrimitive) {
    let mut midpoints: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut indices = Vec::with_capacity(primitive.indices.len() * 4);

    let original = std::mem::take(&mut primitive.indices);
    for triangle in original.chunks_exact(3) {
        let (a, b, c) = (triangle[0], triangle[1], triangle[2]);
        let mut midpoint = |primitive: &mut sms_authoring::ModelPrimitive, x: u32, y: u32| -> u32 {
            let key = if x < y { (x, y) } else { (y, x) };
            if let Some(existing) = midpoints.get(&key) {
                return *existing;
            }
            let index = primitive.positions.len() as u32;
            let blend3 = |values: &[[f32; 3]]| -> Option<[f32; 3]> {
                let (first, second) = (values.get(x as usize)?, values.get(y as usize)?);
                Some(std::array::from_fn(|axis| {
                    (first[axis] + second[axis]) * 0.5
                }))
            };
            if let Some(position) = blend3(&primitive.positions) {
                primitive.positions.push(position);
            }
            if let Some(normal) = blend3(&primitive.normals) {
                let length = vec3_dot(normal, normal).sqrt();
                primitive.normals.push(if length > f32::EPSILON {
                    vec3_scale(normal, 1.0 / length)
                } else {
                    normal
                });
            }
            if let (Some(first), Some(second)) = (
                primitive.tangents.get(x as usize),
                primitive.tangents.get(y as usize),
            ) {
                let mut tangent = std::array::from_fn(|axis| (first[axis] + second[axis]) * 0.5);
                let direction = [tangent[0], tangent[1], tangent[2]];
                let length = vec3_dot(direction, direction).sqrt();
                if length > f32::EPSILON {
                    for component in tangent.iter_mut().take(3) {
                        *component /= length;
                    }
                }
                tangent[3] = if first[3].signum() == second[3].signum() {
                    first[3].signum()
                } else {
                    first[3]
                };
                primitive.tangents.push(tangent);
            }
            for set in &mut primitive.tex_coords {
                if let (Some(first), Some(second)) =
                    (set.values.get(x as usize), set.values.get(y as usize))
                {
                    let blended = std::array::from_fn(|axis| (first[axis] + second[axis]) * 0.5);
                    set.values.push(blended);
                }
            }
            for set in &mut primitive.colors {
                if let (Some(first), Some(second)) =
                    (set.values.get(x as usize), set.values.get(y as usize))
                {
                    let blended = std::array::from_fn(|axis| (first[axis] + second[axis]) * 0.5);
                    set.values.push(blended);
                }
            }
            midpoints.insert(key, index);
            index
        };
        let ab = midpoint(primitive, a, b);
        let bc = midpoint(primitive, b, c);
        let ca = midpoint(primitive, c, a);
        indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }
    primitive.indices = indices;
}

/// Maps each vertex to a representative index shared by every vertex at the
/// same position, so split vertices are coloured as one.
fn weld_positions(positions: &[[f32; 3]]) -> Vec<usize> {
    const WELD_SCALE: f32 = 64.0;
    let mut representative: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    positions
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let key = std::array::from_fn(|axis| (position[axis] * WELD_SCALE).round() as i64);
            *representative.entry(key).or_insert(index)
        })
        .collect()
}

/// Deduplicated edge neighbours per welded position.
fn primitive_adjacency(indices: &[u32], welds: &[usize]) -> BTreeMap<usize, Vec<usize>> {
    let mut adjacency: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let weld = |index: u32| welds.get(index as usize).copied().unwrap_or(index as usize);
    for triangle in indices.chunks_exact(3) {
        for (from, to) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let (a, b) = (weld(from), weld(to));
            if a == b {
                continue;
            }
            let neighbours = adjacency.entry(a).or_default();
            if !neighbours.contains(&b) {
                neighbours.push(b);
            }
            let neighbours = adjacency.entry(b).or_default();
            if !neighbours.contains(&a) {
                neighbours.push(a);
            }
        }
    }
    adjacency
}

fn smoothed_colors(
    colors: &[[f32; 4]],
    welds: &[usize],
    adjacency: &BTreeMap<usize, Vec<usize>>,
    amount: f32,
) -> Vec<[f32; 4]> {
    // Average per welded position first, so split vertices agree before they
    // are blended outward.
    let mut sums: BTreeMap<usize, ([f32; 3], usize)> = BTreeMap::new();
    for (index, color) in colors.iter().enumerate() {
        let entry = sums
            .entry(welds.get(index).copied().unwrap_or(index))
            .or_insert(([0.0; 3], 0));
        for (sum, channel) in entry.0.iter_mut().zip(color.iter()) {
            *sum += *channel;
        }
        entry.1 += 1;
    }
    let averages: BTreeMap<usize, [f32; 3]> = sums
        .into_iter()
        .map(|(position, (sum, count))| {
            (
                position,
                std::array::from_fn(|axis| sum[axis] / count.max(1) as f32),
            )
        })
        .collect();

    let mut blended: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for (position, own) in &averages {
        let Some(neighbours) = adjacency.get(position) else {
            blended.insert(*position, *own);
            continue;
        };
        let mut sum = [0.0f32; 3];
        let mut count = 0usize;
        for neighbour in neighbours {
            if let Some(color) = averages.get(neighbour) {
                for axis in 0..3 {
                    sum[axis] += color[axis];
                }
                count += 1;
            }
        }
        if count == 0 {
            blended.insert(*position, *own);
            continue;
        }
        blended.insert(
            *position,
            std::array::from_fn(|axis| own[axis] + (sum[axis] / count as f32 - own[axis]) * amount),
        );
    }

    colors
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let position = welds.get(index).copied().unwrap_or(index);
            let rgb = blended
                .get(&position)
                .copied()
                .unwrap_or([color[0], color[1], color[2]]);
            [rgb[0], rgb[1], rgb[2], color[3]]
        })
        .collect()
}

impl SmsEditorApp {
    /// Shift erases and Ctrl smooths, whatever the panel is set to. Ctrl wins
    /// when both are held, since smoothing is the harder one to reach.
    fn vertex_paint_modifier_mode(shift: bool, ctrl: bool) -> Option<VertexPaintMode> {
        match (ctrl, shift) {
            (true, _) => Some(VertexPaintMode::Smooth),
            (_, true) => Some(VertexPaintMode::Eraser),
            _ => None,
        }
    }

    /// Opens a stroke, loading the assets it will paint.
    fn begin_vertex_paint_live_stroke(&mut self, mode: VertexPaintMode) {
        if self.vertex_paint_live.is_some() {
            return;
        }
        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log.push(
                "Select a terrain instance in the hierarchy before painting; the brush only \
                 paints the selected one."
                    .to_string(),
            );
            return;
        }
        // Up front, not at the commit: the preview reads the same material
        // switch the export does, so without it the stroke would go down
        // invisibly and only appear once the asset was saved.
        for target in &mut targets {
            enable_vertex_colors(&mut target.document);
        }
        for target in &targets {
            self.cache_model_previews_for_document(target.id, &target.document);
        }
        // Enabling colours can add a fallback material and change GPU batch
        // structure. Pay for one full rebuild at stroke start; subsequent
        // samples update only the authored surfaces.
        self.rebuild_gpu_viewport_scene();
        let visibility = crate::triangle_bvh::TriangleBvh::build(
            self.model_preview
                .as_ref()
                .map(|preview| {
                    preview
                        .triangles
                        .iter()
                        .filter(|triangle| preview_triangle_frames_object(triangle))
                        .map(|triangle| triangle.vertices)
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.vertex_paint_live = Some(VertexPaintLiveStroke {
            targets,
            visibility,
            camera_position: self.camera_frame().position,
            applied: 0,
            last_pointer: None,
            distance_to_next_sample: 0.0,
            sample_spacing: (self.vertex_paint_radius.max(1.0) * 0.25).max(1.0),
            mode,
        });
    }

    /// Folds the samples added since the last frame into the open stroke.
    fn advance_vertex_paint_live_stroke(&mut self) {
        let Some(rect) = self.vertex_paint_rect else {
            return;
        };
        let Some(mut live) = self.vertex_paint_live.take() else {
            return;
        };
        if self.vertex_paint_stroke.len() <= live.applied {
            self.vertex_paint_live = Some(live);
            return;
        }
        let raw = &self.vertex_paint_stroke[live.applied..];
        live.applied = self.vertex_paint_stroke.len();
        let samples = resample_stroke_points(
            raw,
            &mut live.last_pointer,
            &mut live.distance_to_next_sample,
            live.sample_spacing,
        );
        if samples.is_empty() {
            self.vertex_paint_live = Some(live);
            return;
        }

        Self::apply_stroke_to_targets(
            &mut live.targets,
            &samples,
            StrokeApplication {
                projection: &self.camera_projection(rect),
                radius: self.vertex_paint_radius.max(1.0),
                strength: self.vertex_paint_strength.clamp(0.0, 1.0),
                color: self.vertex_paint_color,
                mode: live.mode,
                visibility: &live.visibility,
                camera_position: live.camera_position,
            },
        );

        // Rebuilt from memory. The catalog is not touched until the stroke
        // ends, so dragging costs no disk writes.
        for target in &live.targets {
            self.cache_model_previews_for_document(target.id, &target.document);
        }
        self.refresh_authored_model_instance_preview_surfaces();
        self.vertex_paint_live = Some(live);
    }

    /// Ends a stroke and writes it to the catalog as one undoable step.
    pub(super) fn finish_vertex_paint_live_stroke(&mut self) {
        self.vertex_paint_stroke.clear();
        let Some(live) = self.vertex_paint_live.take() else {
            return;
        };
        let label = match live.mode {
            VertexPaintMode::Brush => "Painted vertex colour",
            VertexPaintMode::Eraser => "Erased vertex colour",
            VertexPaintMode::Smooth => "Softened vertex colour",
        };
        self.commit_terrain_targets(live.targets, label, true);
    }

    /// Applies stroke samples to already-loaded assets.
    ///
    /// Screen space, not a surface raycast. Casting a ray and painting around
    /// the hit point misses wherever the ray leaves the mesh, so edges and
    /// silhouettes could not be painted at all. Projecting the vertices instead
    /// also makes the radius genuinely pixels, which is what the slider claims.
    fn apply_stroke_to_targets(
        targets: &mut [PaintTarget],
        samples: &[egui::Pos2],
        application: StrokeApplication<'_>,
    ) {
        for target in targets {
            let world = Self::target_world_vertex_occurrences(target);
            // Erasing goes back to the material's own diffuse, the same place
            // Clear Paint goes. Painting points the colour channel at the
            // vertex array, so the material register stops reaching the
            // surface: white in that array is not "no paint", it is a white
            // surface, and erasing to it bleached the model.
            let diffuse = target
                .document
                .materials
                .iter()
                .map(|material| material.source_base_color)
                .collect::<Vec<_>>();
            // Apply the fixed-distance dabs in order. Grouping them by UI
            // frame would change repeated brush blends, and smoothing would
            // sample a different intermediate neighbourhood.
            for sample in samples {
                let mut primitive_index = 0usize;
                for mesh in &mut target.document.meshes {
                    for primitive in &mut mesh.primitives {
                        let Some(occurrences) = world.get(primitive_index) else {
                            continue;
                        };
                        primitive_index += 1;
                        let base = primitive
                            .material
                            .and_then(|index| diffuse.get(index as usize))
                            .copied()
                            .unwrap_or([1.0; 4]);
                        let welds = weld_positions(&primitive.positions);
                        let adjacency = primitive_adjacency(&primitive.indices, &welds);
                        let colors = primitive_colors_mut(primitive);
                        let smoothed = matches!(application.mode, VertexPaintMode::Smooth)
                            .then(|| smoothed_colors(colors, &welds, &adjacency, 1.0));

                        for (index, value) in colors.iter_mut().enumerate() {
                            let amount = occurrences
                                .iter()
                                .filter_map(|vertices| vertices.get(index))
                                .filter_map(|vertex| {
                                    let (screen, _) = application
                                        .projection
                                        .project_world_to_screen(vertex.position)?;
                                    let distance = screen.distance(*sample);
                                    if distance >= application.radius
                                        || !vertex_visible_from(
                                            application.camera_position,
                                            vertex.position,
                                            application.visibility,
                                        )
                                    {
                                        return None;
                                    }
                                    let falloff =
                                        1.0 - (distance / application.radius).clamp(0.0, 1.0);
                                    Some((falloff * falloff * application.strength).clamp(0.0, 1.0))
                                })
                                .fold(0.0f32, f32::max);
                            if amount <= f32::EPSILON {
                                continue;
                            }
                            let goal = match application.mode {
                                VertexPaintMode::Brush => application.color,
                                VertexPaintMode::Eraser => [base[0], base[1], base[2]],
                                VertexPaintMode::Smooth => smoothed
                                    .as_ref()
                                    .and_then(|set| set.get(index))
                                    .map(|blend| [blend[0], blend[1], blend[2]])
                                    .unwrap_or([value[0], value[1], value[2]]),
                            };
                            for axis in 0..3 {
                                value[axis] += (goal[axis] - value[axis]) * amount;
                                value[axis] = value[axis].clamp(0.0, 1.0);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn handle_vertex_paint_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if self.tool != EditorTool::VertexPaint {
            return false;
        }
        self.vertex_paint_rect = Some(rect);
        let pointer = ui.input(|input| input.pointer.interact_pos());
        // Surface hit is only for drawing the ring; painting itself is screen
        // space, so a miss here never stops a stroke.
        self.vertex_paint_cursor = response
            .hovered()
            .then(|| pointer.and_then(|pointer| self.vertex_paint_surface_hit(rect, pointer)))
            .flatten();
        let (down, shift, ctrl, alt) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                input.modifiers.shift,
                input.modifiers.command,
                input.modifiers.alt,
            )
        });
        self.vertex_paint_modifier_mode = Self::vertex_paint_modifier_mode(shift, ctrl);
        if alt {
            if self.vertex_paint_live.is_some() || !self.vertex_paint_stroke.is_empty() {
                self.finish_vertex_paint_live_stroke();
            }
            return false;
        }
        if down && response.hovered() {
            if self.vertex_paint_live.is_none() {
                // Latched at the start of the stroke: letting a modifier go
                // halfway through should not switch what the stroke is doing.
                let mode = self
                    .vertex_paint_modifier_mode
                    .unwrap_or(self.vertex_paint_mode);
                self.begin_vertex_paint_live_stroke(mode);
            }
            if let Some(pointer) = pointer {
                self.vertex_paint_stroke.push(pointer);
            }
            self.advance_vertex_paint_live_stroke();
            return true;
        }
        if !down && (self.vertex_paint_live.is_some() || !self.vertex_paint_stroke.is_empty()) {
            self.finish_vertex_paint_live_stroke();
            return true;
        }
        false
    }

    /// Surface point and normal under a viewport position.
    fn vertex_paint_surface_hit(
        &self,
        rect: egui::Rect,
        position: egui::Pos2,
    ) -> Option<([f32; 3], [f32; 3])> {
        let preview = self.model_preview.as_ref()?;
        let frame = self.camera_frame();
        let focal = perspective_focal_length(rect, self.viewport_zoom).max(1.0);
        let local = position - rect.center() - self.viewport_pan;
        let ray = vec3_normalize(vec3_add(
            frame.forward,
            vec3_add(
                vec3_scale(frame.right, local.x / focal),
                vec3_scale(frame.up, -local.y / focal),
            ),
        ));
        nearest_surface_hit(&preview.triangles, frame.position, ray)
    }

    /// Brush ring and sun direction, drawn only while the tool is active.
    pub(super) fn paint_vertex_paint_overlay(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.tool != EditorTool::VertexPaint {
            return;
        }
        let projection = self.camera_projection(rect);

        // Sun direction, anchored at the camera focus rather than the pointer
        // so it does not sit across the brush ring while painting.
        let (yaw, pitch) = (
            self.vertex_paint_sun_yaw.to_radians(),
            self.vertex_paint_sun_pitch.to_radians(),
        );
        let light = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let anchor = self.renderer.camera().focus;
        let reach = (self.renderer.camera().distance * 0.5).max(500.0);
        if let Some([start, end]) = projection
            .project_world_segment_to_screen(anchor, vec3_add(anchor, vec3_scale(light, reach)))
        {
            painter.line_segment(
                [start, end],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(240, 200, 90)),
            );
            painter.circle_filled(end, 3.5, egui::Color32::from_rgb(240, 200, 90));
        }

        // Brush ring, lying in the surface's tangent plane so painting a wall
        // shows a ring standing on that wall rather than a flat disc.
        let Some((center, normal)) = self.vertex_paint_cursor else {
            return;
        };
        let reference = if normal[1].abs() > 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let tangent = vec3_normalize(vec3_cross(reference, normal));
        let bitangent = vec3_cross(normal, tangent);
        // The slider is in pixels, so the ring is sized at the surface's depth
        // to match what the brush will actually reach.
        let world_radius = projection
            .project_world_to_screen(center)
            .map_or(1.0, |(_, depth)| {
                let focal = perspective_focal_length(rect, self.viewport_zoom).max(1.0);
                (self.vertex_paint_radius * depth / focal).max(0.01)
            });
        // The ring shows what a click would actually do, modifiers included.
        let color = match self
            .vertex_paint_modifier_mode
            .unwrap_or(self.vertex_paint_mode)
        {
            VertexPaintMode::Eraser => egui::Color32::from_rgb(230, 230, 230),
            VertexPaintMode::Smooth => egui::Color32::from_rgb(120, 190, 235),
            VertexPaintMode::Brush => egui::Color32::from_rgb(
                (self.vertex_paint_color[0] * 255.0) as u8,
                (self.vertex_paint_color[1] * 255.0) as u8,
                (self.vertex_paint_color[2] * 255.0) as u8,
            ),
        };
        let mut ring = Vec::with_capacity(49);
        for step in 0..=48 {
            let angle = step as f32 / 48.0 * std::f32::consts::TAU;
            let offset = vec3_add(
                vec3_scale(tangent, angle.cos() * world_radius),
                vec3_scale(bitangent, angle.sin() * world_radius),
            );
            match projection.project_world_to_screen(vec3_add(center, offset)) {
                Some((screen, _)) => ring.push(screen),
                None => return,
            }
        }
        painter.add(egui::Shape::line(ring, egui::Stroke::new(2.0, color)));
    }

    /// Re-grades from the baseline and shows it, without touching the catalog.
    fn refresh_vertex_paint_grade(&mut self) {
        let settings = self.vertex_paint_grade_settings;
        let asset = self
            .selected_model_instance()
            .map(|instance| instance.placement.asset_id);

        // A session belongs to the terrain it opened on. Grading whatever got
        // selected later against a stale baseline would write one mesh's
        // colours onto another.
        if self
            .vertex_paint_grade
            .as_ref()
            .is_some_and(|grade| grade.baseline.first().map(|entry| entry.id) != asset)
        {
            self.vertex_paint_grade = None;
        }
        if self.vertex_paint_grade.is_none() {
            if settings.is_neutral() {
                return;
            }
            let targets = self.terrain_paint_targets_scoped(true);
            if targets.is_empty() {
                self.log
                    .push("Select a terrain instance in the hierarchy before grading.".to_string());
                return;
            }
            let baseline = targets
                .iter()
                .map(|target| snapshot_paint_state(target.id, &target.document))
                .collect();
            self.vertex_paint_grade = Some(VertexPaintGrade { targets, baseline });
        }

        let Some(mut grade) = self.vertex_paint_grade.take() else {
            return;
        };
        for (target, baseline) in grade.targets.iter_mut().zip(grade.baseline.iter()) {
            restore_paint_state(&mut target.document, baseline);
            let diffuse = target
                .document
                .materials
                .iter()
                .map(|material| material.source_base_color)
                .collect::<Vec<_>>();
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let base = primitive
                        .material
                        .and_then(|index| diffuse.get(index as usize))
                        .copied()
                        .unwrap_or([1.0; 4]);
                    for color in primitive_colors_mut(primitive) {
                        settings.apply(color, [base[0], base[1], base[2]]);
                    }
                }
            }
        }
        for target in &grade.targets {
            self.cache_model_previews_for_document(target.id, &target.document);
        }
        self.vertex_paint_grade = Some(grade);
    }

    /// Writes the grade to the catalog as one undo step.
    fn apply_vertex_paint_grade(&mut self) {
        let Some(grade) = self.vertex_paint_grade.take() else {
            return;
        };
        self.commit_terrain_targets(grade.targets, "Graded vertex colour", true);
        // The grade is in the asset now, so the sliders start again from
        // neutral rather than re-applying themselves on the next nudge.
        self.vertex_paint_grade_settings = VertexPaintGradeSettings::default();
    }

    /// Puts the terrain back as it was before the sliders moved.
    fn revert_vertex_paint_grade(&mut self) {
        self.vertex_paint_grade_settings = VertexPaintGradeSettings::default();
        let Some(mut grade) = self.vertex_paint_grade.take() else {
            return;
        };
        for (target, baseline) in grade.targets.iter_mut().zip(grade.baseline.iter()) {
            restore_paint_state(&mut target.document, baseline);
        }
        for target in &grade.targets {
            self.cache_model_previews_for_document(target.id, &target.document);
        }
    }

    pub(super) fn vertex_paint_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Vertex Paint");
        let terrain = self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| {
                instance.placement.export_mode == ModelInstanceExportMode::MapTerrain
            })
            .count();
        if terrain == 0 {
            ui.colored_label(
                egui::Color32::from_rgb(245, 190, 90),
                "No terrain to paint. Set a model instance to 'Bake as map terrain' first.",
            );
            return;
        }
        // Says which asset a stroke will hit, since that differs from what the
        // whole-stage bakes below cover.
        match self.selected_model_instance() {
            Some(instance) => {
                ui.small(format!(
                    "Painting and baking '{}' only.",
                    instance.placement.name
                ));
            }
            None => {
                ui.colored_label(
                    egui::Color32::from_rgb(245, 190, 90),
                    "Select a terrain instance in the hierarchy to paint or bake it.",
                );
            }
        }
        ui.separator();

        ui.horizontal(|ui| {
            for mode in [
                VertexPaintMode::Brush,
                VertexPaintMode::Eraser,
                VertexPaintMode::Smooth,
            ] {
                ui.selectable_value(&mut self.vertex_paint_mode, mode, mode.label());
            }
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_rgb(&mut self.vertex_paint_color);
            ui.label("Colour");
        });
        ui.add(egui::Slider::new(&mut self.vertex_paint_radius, 4.0..=200.0).text("Radius"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_strength, 0.0..=1.0).text("Strength"));
        ui.label(
            "Shift while painting erases, Ctrl smooths. Ctrl+Z and Ctrl+Y step through strokes.",
        );

        ui.separator();
        if let Some(both) = self.terrain_draws_both_sides() {
            let mut wanted = both;
            if ui
                .checkbox(&mut wanted, "Draw both sides")
                .on_hover_text(
                    "Stops terrain culling back faces, so none can drop out of the bake. Costs                      fill rate; try Fix Facing first",
                )
                .changed()
            {
                self.set_terrain_double_sided(wanted);
            }
        }
        if ui
            .button("Subdivide")
            .on_hover_text(
                "Split every triangle into four. Vertex colour cannot hold detail finer than                  the mesh, so a sparse surface has nothing for a small brush to paint.",
            )
            .clicked()
        {
            self.subdivide_terrain();
        }

        ui.horizontal(|ui| {
            if ui
                .button("Smooth Out")
                .on_hover_text("Blend every vertex toward its neighbours across the whole stage")
                .clicked()
            {
                self.smooth_terrain_vertex_colors();
            }
            if ui
                .button("Make Opaque")
                .on_hover_text(
                    "Clear blending and re-enable depth writes, so terrain stops showing                      through itself",
                )
                .clicked()
            {
                self.make_terrain_opaque();
            }
            if ui
                .button("Fix Facing")
                .on_hover_text(
                    "Rewind triangles that face away from their own normals, which is what                      makes a mesh look inside out once it bakes as terrain",
                )
                .clicked()
            {
                self.repair_terrain_winding();
            }
            if ui
                .button("Clear Paint")
                .on_hover_text(
                    "Reset the terrain to its material diffuse, dropping paint and bakes",
                )
                .clicked()
            {
                self.clear_terrain_vertex_colors();
            }
        });

        ui.add(
            egui::Slider::new(&mut self.vertex_paint_smooth_iterations, 1..=20)
                .text("Smooth passes"),
        );

        ui.separator();
        ui.strong("Ambient occlusion (raycast)");
        ui.label(
            "Fires rays from every vertex and counts what they hit. Slower than Dirty, and \
             indifferent to how the mesh was triangulated.",
        );
        ui.add(egui::Slider::new(&mut self.vertex_paint_ao_strength, 0.0..=1.0).text("Strength"));
        ui.add(
            egui::Slider::new(&mut self.vertex_paint_ao_distance, 10.0..=4000.0)
                .logarithmic(true)
                .text("Distance"),
        )
        .on_hover_text("How far a ray looks for a blocker before calling the vertex open");
        ui.add(egui::Slider::new(&mut self.vertex_paint_ao_rays, 8..=256).text("Rays"))
            .on_hover_text("More rays is smoother and slower");
        if ui
            .button("Bake AO")
            .on_hover_text("Occlude the selected terrain against every terrain mesh in the stage")
            .clicked()
        {
            self.bake_terrain_occlusion();
        }

        ui.separator();
        ui.strong("Grade");
        ui.label("Adjusts the colours already on the terrain. Nothing is written until Apply.");
        let mut changed = false;
        changed |= ui
            .add(
                egui::Slider::new(&mut self.vertex_paint_grade_settings.exposure, -3.0..=3.0)
                    .text("Exposure"),
            )
            .on_hover_text("Stops, so each one doubles or halves the light")
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut self.vertex_paint_grade_settings.contrast, -1.0..=1.0)
                    .text("Contrast"),
            )
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut self.vertex_paint_grade_settings.vibrance, -1.0..=1.0)
                    .text("Vibrance"),
            )
            .on_hover_text("Leans on the duller colours, so painted patches do not blow out first")
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut self.vertex_paint_grade_settings.hue, -180.0..=180.0)
                    .text("Hue"),
            )
            .on_hover_text("Rotates the colours without changing how light they are")
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut self.vertex_paint_grade_settings.shadow, -1.0..=1.0)
                    .text("Shadow"),
            )
            .on_hover_text("Deepens or lifts the dark end without moving the lit surfaces")
            .changed();
        ui.horizontal(|ui| {
            changed |= ui
                .color_edit_button_rgb(&mut self.vertex_paint_grade_settings.tint)
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.vertex_paint_grade_settings.tint_amount, 0.0..=1.0)
                        .text("Shadow tint"),
                )
                .changed();
        });
        if changed {
            self.refresh_vertex_paint_grade();
        }
        ui.horizontal(|ui| {
            let open = self.vertex_paint_grade.is_some();
            if ui
                .add_enabled(open, egui::Button::new("Apply"))
                .on_hover_text("Write the graded colours to the asset")
                .clicked()
            {
                self.apply_vertex_paint_grade();
            }
            if ui
                .add_enabled(open, egui::Button::new("Revert"))
                .on_hover_text("Put the colours back and reset the sliders")
                .clicked()
            {
                self.revert_vertex_paint_grade();
            }
        });

        ui.separator();
        ui.strong("Sun (directional light bake)");
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_yaw, 0.0..=360.0).text("Yaw"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_pitch, -89.0..=89.0).text("Pitch"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_softness, 0.0..=1.0).text("Softness"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_shadow, 0.0..=1.0).text("Shadow"));
        ui.checkbox(
            &mut self.vertex_paint_sun_smooth_normals,
            "Smooth normals first",
        )
        .on_hover_text(
            "Average normals across shared positions before lighting. A faceted mesh otherwise \
             bakes a hard step at every edge.",
        );
        if ui
            .button("Bake Sun")
            .on_hover_text("Multiply vertex colours by a directional light term")
            .clicked()
        {
            self.bake_terrain_sun();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sms_authoring::{ModelMesh, ModelNode, ModelPrimitive};

    fn test_document() -> ModelAssetDocument {
        let mut document = ModelAssetDocument::new("terrain");
        document.scene_roots.push(0);
        document.nodes.push(ModelNode {
            name: "terrain".to_string(),
            parent: None,
            children: Vec::new(),
            mesh: Some(0),
            purpose: NodePurpose::Render,
            local_transform: IDENTITY_MATRIX,
        });
        document.meshes.push(ModelMesh {
            name: "terrain".to_string(),
            primitives: vec![ModelPrimitive {
                positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, 2.0]],
                normals: vec![[0.0, 1.0, 0.0]; 3],
                tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                tex_coords: Vec::new(),
                colors: Vec::new(),
                indices: vec![0, 1, 2],
                material: None,
            }],
        });
        document
    }

    #[test]
    fn topology_snapshot_restores_the_complete_asset() {
        let id = AssetId::new();
        let original = test_document();
        let snapshot = snapshot_paint_state(id, &original);
        let mut changed = original.clone();
        subdivide_primitive(&mut changed.meshes[0].primitives[0]);
        enable_vertex_colors(&mut changed);
        assert_ne!(changed, original);

        restore_paint_state(&mut changed, &snapshot);
        assert_eq!(changed, original);
    }

    #[test]
    fn subdivision_keeps_every_vertex_stream_aligned() {
        let mut primitive = test_document().meshes.remove(0).primitives.remove(0);
        subdivide_primitive(&mut primitive);

        assert_eq!(primitive.positions.len(), 6);
        assert_eq!(primitive.normals.len(), primitive.positions.len());
        assert_eq!(primitive.tangents.len(), primitive.positions.len());
        assert_eq!(primitive.indices.len(), 12);
        assert!(primitive
            .tangents
            .iter()
            .all(|tangent| (tangent[0] - 1.0).abs() < 1e-5 && tangent[3] == 1.0));
    }

    #[test]
    fn materialless_primitives_get_a_dedicated_fallback() {
        let mut document = test_document();
        document.materials.push(vertex_color_material("unrelated"));
        let previous_materials = document.materials.len();
        enable_vertex_colors(&mut document);

        assert_eq!(document.materials.len(), previous_materials + 1);
        assert_eq!(
            document.meshes[0].primitives[0].material,
            Some(previous_materials as u32)
        );
    }

    #[test]
    fn enabling_vertex_colors_preserves_the_imported_material_tint_and_alpha() {
        let mut document = test_document();
        let mut material = vertex_color_material("tinted");
        material.gx.material_colors[0] = Some([64, 128, 192, 32]);
        material.gx.color_channels[0]
            .as_mut()
            .expect("vertex material channel")
            .material_source = 0;
        document.materials.push(material);
        document.meshes[0].primitives[0].material = Some(0);

        enable_vertex_colors(&mut document);

        let expected = [64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 32.0 / 255.0];
        assert!(document.meshes[0].primitives[0].colors[0]
            .values
            .iter()
            .all(|color| *color == expected));
        let once = document.clone();
        enable_vertex_colors(&mut document);
        assert_eq!(
            document, once,
            "material tint must be folded into COLOR0 once"
        );
    }

    #[test]
    fn selected_transform_drives_world_space_edits() {
        let id = AssetId::new();
        let document = test_document();
        let mut selected_transform = IDENTITY_MATRIX;
        selected_transform[3][0] = 100.0;
        let target = PaintTarget {
            id,
            baseline: snapshot_paint_state(id, &document),
            document,
            transforms: vec![IDENTITY_MATRIX, selected_transform],
            transform: selected_transform,
        };

        let world = SmsEditorApp::target_world_vertex_occurrences(&target);
        assert_eq!(world[0][0][0].position, [100.0, 0.0, 0.0]);
    }

    #[test]
    fn every_node_reference_contributes_a_world_space_occurrence() {
        let id = AssetId::new();
        let mut document = test_document();
        document.scene_roots.clear();
        document.nodes.clear();
        for (name, x) in [("left", -25.0), ("right", 40.0)] {
            let mut transform = IDENTITY_MATRIX;
            transform[3][0] = x;
            document.nodes.push(ModelNode {
                name: name.to_string(),
                parent: None,
                children: Vec::new(),
                mesh: Some(0),
                purpose: NodePurpose::Render,
                local_transform: transform,
            });
        }
        let target = PaintTarget {
            id,
            baseline: snapshot_paint_state(id, &document),
            document,
            transforms: vec![IDENTITY_MATRIX],
            transform: IDENTITY_MATRIX,
        };

        let world = SmsEditorApp::target_world_vertex_occurrences(&target);

        assert_eq!(world[0].len(), 2);
        assert_eq!(world[0][0][0].position, [-25.0, 0.0, 0.0]);
        assert_eq!(world[0][1][0].position, [40.0, 0.0, 0.0]);
    }

    #[test]
    fn inactive_meshes_have_no_world_space_occurrence() {
        let id = AssetId::new();
        let mut document = test_document();
        document.nodes = vec![
            ModelNode {
                name: "active root".to_string(),
                parent: None,
                children: Vec::new(),
                mesh: None,
                purpose: NodePurpose::Render,
                local_transform: IDENTITY_MATRIX,
            },
            ModelNode {
                name: "inactive mesh".to_string(),
                parent: None,
                children: Vec::new(),
                mesh: Some(0),
                purpose: NodePurpose::Render,
                local_transform: IDENTITY_MATRIX,
            },
        ];
        document.scene_roots = vec![0];
        let target = PaintTarget {
            id,
            baseline: snapshot_paint_state(id, &document),
            document,
            transforms: vec![IDENTITY_MATRIX],
            transform: IDENTITY_MATRIX,
        };

        let world = SmsEditorApp::target_world_vertex_occurrences(&target);

        assert_eq!(world.len(), 1);
        assert!(world[0].is_empty());
    }

    #[test]
    fn mesh_transforms_include_only_active_render_nodes() {
        let mut document = test_document();
        let mut root_transform = IDENTITY_MATRIX;
        root_transform[3][0] = 10.0;
        let mut inactive_transform = IDENTITY_MATRIX;
        inactive_transform[3][0] = 30.0;
        document.nodes = vec![
            ModelNode {
                name: "active render".to_string(),
                parent: None,
                children: vec![1],
                mesh: Some(0),
                purpose: NodePurpose::Render,
                local_transform: root_transform,
            },
            ModelNode {
                name: "active collision".to_string(),
                parent: Some(0),
                children: Vec::new(),
                mesh: Some(0),
                purpose: NodePurpose::CollisionOnly,
                local_transform: IDENTITY_MATRIX,
            },
            ModelNode {
                name: "inactive render".to_string(),
                parent: None,
                children: Vec::new(),
                mesh: Some(0),
                purpose: NodePurpose::Render,
                local_transform: inactive_transform,
            },
        ];
        document.scene_roots = vec![0];

        let transforms = mesh_node_transforms(&document);

        assert_eq!(transforms.get(&0), Some(&vec![root_transform]));
    }

    #[test]
    fn normals_use_inverse_transpose_under_non_uniform_scale() {
        let mut transform = IDENTITY_MATRIX;
        transform[0][0] = 2.0;
        let inverse_sqrt_two = 1.0 / 2.0f32.sqrt();

        let normal = transform_normal(transform, [inverse_sqrt_two, inverse_sqrt_two, 0.0]);

        assert!((normal[0] - 0.447_213_6).abs() < 1e-5);
        assert!((normal[1] - 0.894_427_2).abs() < 1e-5);
        assert!(normal[2].abs() < 1e-5);
    }

    #[test]
    fn foreground_geometry_blocks_vertices_behind_it() {
        let visibility = crate::triangle_bvh::TriangleBvh::build(vec![[
            [-10.0, -10.0, 5.0],
            [10.0, -10.0, 5.0],
            [0.0, 10.0, 5.0],
        ]]);
        assert!(vertex_visible_from(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 5.0],
            &visibility
        ));
        assert!(!vertex_visible_from(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 10.0],
            &visibility
        ));
    }

    #[test]
    fn failed_undo_keeps_the_record_available() {
        let id = AssetId::new();
        let document = test_document();
        let snapshot = snapshot_paint_state(id, &document);
        let mut app = SmsEditorApp::default();
        app.vertex_paint_undo_stack
            .push_back(VertexPaintUndoRecord {
                label: "test".to_string(),
                before: vec![snapshot.clone()],
                after: vec![snapshot],
            });

        assert!(!app.undo_vertex_paint());
        assert_eq!(app.vertex_paint_undo_stack.len(), 1);
        assert!(app.vertex_paint_redo_stack.is_empty());
    }

    #[test]
    fn terrain_undo_refuses_to_overwrite_a_newer_asset_revision() {
        let temporary = tempfile::tempdir().unwrap();
        let catalog =
            sms_authoring::ModelAssetCatalog::open_content_root(temporary.path().join("Content"))
                .unwrap();
        let before = test_document();
        let mut after = before.clone();
        enable_vertex_colors(&mut after);
        let entry = catalog
            .create_asset("terrain.smsmodel", &after)
            .expect("create terrain asset");
        let mut newer = after.clone();
        newer.diagnostics.push(sms_authoring::Diagnostic {
            severity: sms_authoring::Severity::Info,
            code: sms_authoring::DiagnosticCode::EmptyPrimitive,
            message: "newer direct asset edit".to_string(),
            context: None,
            acknowledgement_required: false,
        });
        catalog
            .save_asset(entry.id, &newer)
            .expect("save newer revision");
        let mut app = SmsEditorApp {
            project_root: temporary.path().to_string_lossy().into_owned(),
            ..SmsEditorApp::default()
        };
        app.vertex_paint_undo_stack
            .push_back(VertexPaintUndoRecord {
                label: "painted terrain".to_string(),
                before: vec![snapshot_paint_state(entry.id, &before)],
                after: vec![snapshot_paint_state(entry.id, &after)],
            });

        assert!(app.undo_vertex_paint());

        assert_eq!(catalog.load_asset(entry.id).unwrap(), newer);
        assert!(app.vertex_paint_undo_stack.is_empty());
        assert!(app.vertex_paint_redo_stack.is_empty());
        assert!(app
            .log
            .iter()
            .any(|message| message.contains("newer edits")));
    }

    #[test]
    fn terrain_commit_refuses_to_overwrite_a_replaced_asset() {
        let temporary = tempfile::tempdir().unwrap();
        let catalog =
            sms_authoring::ModelAssetCatalog::open_content_root(temporary.path().join("Content"))
                .unwrap();
        let before = test_document();
        let entry = catalog
            .create_asset("terrain.smsmodel", &before)
            .expect("create terrain asset");
        let mut painted = before.clone();
        enable_vertex_colors(&mut painted);
        painted.meshes[0].primitives[0].colors[0].values[0] = [0.2, 0.3, 0.4, 1.0];
        let mut replacement = before.clone();
        replacement.name = "replacement".to_string();
        catalog
            .save_asset(entry.id, &replacement)
            .expect("save replacement revision");
        let mut app = SmsEditorApp {
            project_root: temporary.path().to_string_lossy().into_owned(),
            ..SmsEditorApp::default()
        };
        let target = PaintTarget {
            id: entry.id,
            baseline: snapshot_paint_state(entry.id, &before),
            document: painted,
            transforms: vec![IDENTITY_MATRIX],
            transform: IDENTITY_MATRIX,
        };

        app.commit_terrain_targets(vec![target], "Painted vertex colour", true);

        assert_eq!(catalog.load_asset(entry.id).unwrap(), replacement);
        assert!(app.vertex_paint_undo_stack.is_empty());
        assert!(app
            .log
            .iter()
            .any(|message| message.contains("changed while the edit was in progress")));
    }

    #[test]
    fn stroke_resampling_is_independent_of_pointer_event_frequency() {
        fn resample(raw: &[egui::Pos2]) -> Vec<egui::Pos2> {
            let mut previous = None;
            let mut distance_to_next = 0.0;
            resample_stroke_points(raw, &mut previous, &mut distance_to_next, 4.0)
        }

        let sparse = resample(&[egui::pos2(0.0, 0.0), egui::pos2(25.0, 0.0)]);
        let frequent = resample(&[
            egui::pos2(0.0, 0.0),
            egui::pos2(5.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::pos2(15.0, 0.0),
            egui::pos2(20.0, 0.0),
            egui::pos2(25.0, 0.0),
        ]);

        assert_eq!(sparse.len(), frequent.len());
        for (left, right) in sparse.iter().zip(&frequent) {
            assert!((*left - *right).length() < 1e-5, "{left:?} != {right:?}");
        }
    }

    #[test]
    fn tool_shortcut_finishes_an_active_paint_stroke_before_switching() {
        let mut app = SmsEditorApp {
            tool: EditorTool::VertexPaint,
            vertex_paint_stroke: vec![egui::pos2(10.0, 10.0)],
            vertex_paint_live: Some(VertexPaintLiveStroke {
                targets: Vec::new(),
                visibility: crate::triangle_bvh::TriangleBvh::build(Vec::new()),
                camera_position: [0.0; 3],
                applied: 0,
                last_pointer: None,
                distance_to_next_sample: 0.0,
                sample_spacing: 4.0,
                mode: VertexPaintMode::Brush,
            }),
            ..SmsEditorApp::default()
        };

        app.apply_tool_keyboard_shortcut(egui::Key::W);

        assert_eq!(app.tool, EditorTool::Move);
        assert!(app.vertex_paint_live.is_none());
        assert!(app.vertex_paint_stroke.is_empty());
    }
}

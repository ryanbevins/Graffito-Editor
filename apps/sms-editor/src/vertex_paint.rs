use super::*;

use std::collections::BTreeMap;

use sms_authoring::{AssetId, ModelAssetDocument, ModelInstanceExportMode};

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

/// A terrain asset opened for painting, with the world transform of every
/// instance that uses it.
struct PaintTarget {
    id: AssetId,
    document: ModelAssetDocument,
    /// One asset can be placed several times; a stroke has to consider each
    /// placement, and every placement writes back to the same vertices.
    transforms: Vec<[[f32; 4]; 4]>,
}

/// One vertex of a target, resolved into world space.
struct WorldVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

fn transform_point(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row]
    })
}

fn transform_direction(matrix: [[f32; 4]; 4], direction: [f32; 3]) -> [f32; 3] {
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
    if document.materials.is_empty() {
        document
            .materials
            .push(vertex_color_material(&document.name));
    }
    let fallback = (document.materials.len() - 1) as u32;
    for mesh in &mut document.meshes {
        for primitive in &mut mesh.primitives {
            if primitive.material.is_none() {
                primitive.material = Some(fallback);
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

impl SmsEditorApp {
    /// Assets behind every instance exported as map terrain.
    ///
    /// The stage terrain is composed from all of them, so a paint operation
    /// covers the lot rather than whichever one happens to be selected.
    fn terrain_paint_targets(&self) -> Vec<PaintTarget> {
        let Some(catalog) = self.model_catalog().ok() else {
            return Vec::new();
        };
        let mut by_asset: BTreeMap<AssetId, Vec<[[f32; 4]; 4]>> = BTreeMap::new();
        for instance in self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| {
                instance.placement.export_mode == ModelInstanceExportMode::MapTerrain
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
                catalog.load_asset(id).ok().map(|document| PaintTarget {
                    id,
                    document,
                    transforms,
                })
            })
            .collect()
    }

    /// World-space position and normal of every vertex in a target, using its
    /// first placement. Repeated placements share vertices, so the first is the
    /// one a world-space operation is resolved against.
    fn target_world_vertices(target: &PaintTarget) -> Vec<Vec<WorldVertex>> {
        let transform = target.transforms.first().copied().unwrap_or([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        target
            .document
            .meshes
            .iter()
            .flat_map(|mesh| mesh.primitives.iter())
            .map(|primitive| {
                primitive
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(index, position)| WorldVertex {
                        position: transform_point(transform, *position),
                        normal: transform_direction(
                            transform,
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
    }

    /// Writes every modified target back to the catalog.
    fn commit_paint_targets(&mut self, targets: Vec<PaintTarget>, label: &str) {
        if targets.is_empty() {
            return;
        }
        if !self.content_catalog_mutation_allowed(label) {
            return;
        }
        let Ok(catalog) = self.model_catalog() else {
            return;
        };
        let mut saved = 0usize;
        for mut target in targets {
            // Every path that writes colour lands here, so the material switch
            // is flipped in one place rather than per operation.
            enable_vertex_colors(&mut target.document);
            match catalog.save_asset(target.id, &target.document) {
                Ok(_) => {
                    saved += 1;
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
        self.force_refresh_model_catalog();
        self.rebuild_model_preview_cache();
        self.log
            .push(format!("{label} across {saved} terrain asset(s)."));
    }

    /// Runs `edit` over every terrain vertex, giving it the world position,
    /// world normal and the colour to modify.
    fn edit_terrain_vertex_colors(
        &mut self,
        label: &str,
        mut edit: impl FnMut(&WorldVertex, &mut [f32; 4]),
    ) {
        let mut targets = self.terrain_paint_targets();
        if targets.is_empty() {
            self.log.push(
                "No terrain to paint: set a model instance to 'Bake as map terrain' first."
                    .to_string(),
            );
            return;
        }
        for target in &mut targets {
            let world = Self::target_world_vertices(target);
            let mut primitive_index = 0usize;
            for mesh in &mut target.document.meshes {
                for primitive in &mut mesh.primitives {
                    let vertices = &world[primitive_index];
                    let colors = primitive_colors_mut(primitive);
                    for (index, color) in colors.iter_mut().enumerate() {
                        if let Some(vertex) = vertices.get(index) {
                            edit(vertex, color);
                        }
                    }
                    primitive_index += 1;
                }
            }
        }
        self.commit_paint_targets(targets, label);
    }

    /// Resets every terrain vertex to opaque white.
    pub(super) fn clear_terrain_vertex_colors(&mut self) {
        self.edit_terrain_vertex_colors("Cleared vertex paint", |_, color| {
            *color = [1.0, 1.0, 1.0, color[3]];
        });
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
        self.edit_terrain_vertex_colors("Baked sun into vertex paint", |vertex, color| {
            let lambert = vec3_dot(vertex.normal, light);
            let hard = lambert.max(0.0);
            let wrap = lambert * 0.5 + 0.5;
            let blended = (hard + (wrap - hard) * softness).clamp(0.0, 1.0);
            let shade = (1.0 - shadow) + shadow * blended;
            for channel in color.iter_mut().take(3) {
                *channel = (*channel * shade).clamp(0.0, 1.0);
            }
        });
    }

    /// Darkens creases, following Blender's "Dirty Vertex Colors".
    ///
    /// Concavity at a welded position is the mean, over its edge neighbours, of
    /// the dot between the direction to that neighbour and the vertex normal.
    /// Positive means neighbours sit above the tangent plane, which is a crease
    /// ambient light cannot reach; convex and flat areas stay untouched.
    pub(super) fn bake_terrain_dirt(&mut self) {
        let amount = self.vertex_paint_dirt.clamp(0.0, 1.0);
        let mut targets = self.terrain_paint_targets();
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
                    let dirt = primitive_cavity(primitive);
                    let colors = primitive_colors_mut(primitive);
                    for (index, color) in colors.iter_mut().enumerate() {
                        let shade = 1.0 - dirt.get(index).copied().unwrap_or(0.0) * amount;
                        for channel in color.iter_mut().take(3) {
                            *channel = (*channel * shade).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
        self.commit_paint_targets(targets, "Baked cavity dirt into vertex paint");
    }

    /// Blends every vertex toward the average of its edge neighbours.
    pub(super) fn smooth_terrain_vertex_colors(&mut self) {
        let amount = self.vertex_paint_strength.clamp(0.0, 1.0);
        let mut targets = self.terrain_paint_targets();
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
                    let smoothed = smoothed_colors(colors, &welds, &adjacency, amount);
                    colors.copy_from_slice(&smoothed);
                }
            }
        }
        self.commit_paint_targets(targets, "Smoothed vertex paint");
    }
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

/// Per-vertex cavity factor in 0..1, where 1 is a deep crease.
pub(super) fn primitive_cavity(primitive: &sms_authoring::ModelPrimitive) -> Vec<f32> {
    let welds = weld_positions(&primitive.positions);
    let adjacency = primitive_adjacency(&primitive.indices, &welds);
    let mut normals: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for (index, weld) in welds.iter().enumerate() {
        let normal = primitive
            .normals
            .get(index)
            .copied()
            .unwrap_or([0.0, 1.0, 0.0]);
        let entry = normals.entry(*weld).or_insert([0.0; 3]);
        for axis in 0..3 {
            entry[axis] += normal[axis];
        }
    }

    let mut cavity: BTreeMap<usize, f32> = BTreeMap::new();
    for (position, neighbours) in &adjacency {
        let normal = normals.get(position).copied().unwrap_or([0.0, 1.0, 0.0]);
        let length = vec3_dot(normal, normal).sqrt();
        if length <= f32::EPSILON {
            continue;
        }
        let normal = vec3_scale(normal, 1.0 / length);
        let origin = primitive.positions[*position];
        let mut concavity = 0.0f32;
        let mut count = 0usize;
        for neighbour in neighbours {
            let Some(target) = primitive.positions.get(*neighbour) else {
                continue;
            };
            let delta = vec3_sub(*target, origin);
            let distance = vec3_dot(delta, delta).sqrt();
            if distance <= f32::EPSILON {
                continue;
            }
            concavity += vec3_dot(vec3_scale(delta, 1.0 / distance), normal);
            count += 1;
        }
        if count > 0 {
            cavity.insert(*position, (concavity / count as f32).clamp(0.0, 1.0));
        }
    }

    // One relaxation pass, so dirt grades into surrounding faces instead of
    // hugging single vertices.
    let relaxed: BTreeMap<usize, f32> = cavity
        .iter()
        .map(|(position, value)| {
            let Some(neighbours) = adjacency.get(position) else {
                return (*position, *value);
            };
            let mut sum = *value;
            let mut count = 1usize;
            for neighbour in neighbours {
                sum += cavity.get(neighbour).copied().unwrap_or(0.0);
                count += 1;
            }
            (*position, sum / count as f32)
        })
        .collect();

    welds
        .iter()
        .map(|weld| relaxed.get(weld).copied().unwrap_or(0.0))
        .collect()
}

impl SmsEditorApp {
    /// Applies the accumulated stroke points to every terrain asset at once.
    ///
    /// Strokes commit on release rather than per frame: each commit rewrites
    /// and saves whole `.smsmodel` documents, far too much work to repeat while
    /// the pointer is moving.
    pub(super) fn commit_vertex_paint_stroke(&mut self) {
        let stroke = std::mem::take(&mut self.vertex_paint_stroke);
        if stroke.is_empty() {
            return;
        }
        let radius = self.vertex_paint_world_radius();
        let strength = self.vertex_paint_strength.clamp(0.0, 1.0);
        let color = self.vertex_paint_color;
        let mode = self.vertex_paint_mode;
        let label = match mode {
            VertexPaintMode::Brush => "Painted vertex colour",
            VertexPaintMode::Eraser => "Erased vertex colour",
            VertexPaintMode::Smooth => "Softened vertex colour",
        };
        self.edit_terrain_vertex_colors(label, |vertex, value| {
            // Nearest stroke sample drives the falloff, so a fast drag with
            // sparse samples still paints a continuous band.
            let mut nearest = f32::INFINITY;
            for point in &stroke {
                let delta = vec3_sub(vertex.position, *point);
                nearest = nearest.min(vec3_dot(delta, delta).sqrt());
            }
            if nearest >= radius {
                return;
            }
            // Smooth falloff to the rim so overlapping dabs do not band.
            let falloff = 1.0 - (nearest / radius).clamp(0.0, 1.0);
            let amount = (falloff * falloff * strength).clamp(0.0, 1.0);
            // White is the identity for a vertex-lit surface, so erasing blends
            // back toward it rather than toward black.
            let target = match mode {
                VertexPaintMode::Brush => color,
                VertexPaintMode::Eraser | VertexPaintMode::Smooth => [1.0, 1.0, 1.0],
            };
            for axis in 0..3 {
                value[axis] += (target[axis] - value[axis]) * amount;
                value[axis] = value[axis].clamp(0.0, 1.0);
            }
        });
    }

    /// Brush radius in world units.
    ///
    /// The slider is in pixels, matching Affinity, so it is scaled by the
    /// camera distance to stay the size it looks on screen.
    fn vertex_paint_world_radius(&self) -> f32 {
        (self.vertex_paint_radius * self.renderer.camera().distance * 0.0015).max(1.0)
    }

    /// Collects stroke samples while the pointer is down over the viewport.
    pub(super) fn handle_vertex_paint_input(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if self.tool != EditorTool::VertexPaint {
            return false;
        }
        let pointer = ui.input(|input| input.pointer.interact_pos());
        // Tracked even when not painting so the brush ring follows the surface
        // the way the goop cursor does.
        self.vertex_paint_cursor = response
            .hovered()
            .then(|| pointer.and_then(|pointer| self.viewport_placement_position(rect, pointer)))
            .flatten();
        let down = ui.input(|input| input.pointer.primary_down());
        if down && response.hovered() {
            if let Some(world) = self.vertex_paint_cursor {
                self.vertex_paint_stroke.push(world);
            }
            return true;
        }
        if !down && !self.vertex_paint_stroke.is_empty() {
            self.commit_vertex_paint_stroke();
            return true;
        }
        false
    }

    /// Brush ring and sun direction, drawn only while the tool is active.
    pub(super) fn paint_vertex_paint_overlay(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.tool != EditorTool::VertexPaint {
            return;
        }
        let projection = self.camera_projection(rect);

        // Sun direction, so the bake angle is visible before committing to it.
        let (yaw, pitch) = (
            self.vertex_paint_sun_yaw.to_radians(),
            self.vertex_paint_sun_pitch.to_radians(),
        );
        let light = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let anchor = self
            .vertex_paint_cursor
            .unwrap_or(self.renderer.camera().focus);
        let reach = (self.renderer.camera().distance * 0.6).max(500.0);
        if let Some([start, end]) = projection
            .project_world_segment_to_screen(anchor, vec3_add(anchor, vec3_scale(light, reach)))
        {
            painter.line_segment(
                [start, end],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(240, 200, 90)),
            );
            painter.circle_filled(end, 3.5, egui::Color32::from_rgb(240, 200, 90));
        }

        // Brush ring on the surface under the pointer.
        let Some(center) = self.vertex_paint_cursor else {
            return;
        };
        let radius = self.vertex_paint_world_radius();
        let mut ring = Vec::new();
        for step in 0..=48 {
            let angle = step as f32 / 48.0 * std::f32::consts::TAU;
            let point = [
                center[0] + angle.cos() * radius,
                center[1],
                center[2] + angle.sin() * radius,
            ];
            match projection.project_world_to_screen(point) {
                Some((screen, _)) => ring.push(screen),
                None => return,
            }
        }
        let color = egui::Color32::from_rgb(
            (self.vertex_paint_color[0] * 255.0) as u8,
            (self.vertex_paint_color[1] * 255.0) as u8,
            (self.vertex_paint_color[2] * 255.0) as u8,
        );
        painter.add(egui::Shape::line(ring, egui::Stroke::new(2.0, color)));
    }

    /// Gives every terrain asset a white colour set and a material that reads
    /// it, the first time the tool is opened for a stage.
    ///
    /// Doing this on open rather than on first stroke means paint shows
    /// immediately. White is the identity for a vertex-lit surface, so the
    /// stage looks unchanged until something is actually painted.
    fn prepare_terrain_for_vertex_paint(&mut self) {
        if self.vertex_paint_prepared.as_deref() == Some(self.stage_id.as_str()) {
            return;
        }
        self.vertex_paint_prepared = Some(self.stage_id.clone());
        self.edit_terrain_vertex_colors("Prepared terrain for vertex paint", |_, _| {});
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
        self.prepare_terrain_for_vertex_paint();
        ui.small(format!(
            "Painting {terrain} terrain instance(s); every one is affected."
        ));
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

        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button("Smooth Out")
                .on_hover_text("Blend every vertex toward its neighbours across the whole stage")
                .clicked()
            {
                self.smooth_terrain_vertex_colors();
            }
            if ui
                .button("Clear Paint")
                .on_hover_text("Reset every terrain vertex to white")
                .clicked()
            {
                self.clear_terrain_vertex_colors();
            }
        });

        ui.separator();
        ui.strong("Dirty (cavity AO)");
        ui.add(egui::Slider::new(&mut self.vertex_paint_dirt, 0.0..=1.0).text("Dirty"));
        if ui
            .button("Bake Dirt")
            .on_hover_text("Darken creases and inner edges")
            .clicked()
        {
            self.bake_terrain_dirt();
        }

        ui.separator();
        ui.strong("Sun (directional light bake)");
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_yaw, 0.0..=360.0).text("Yaw"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_pitch, -89.0..=89.0).text("Pitch"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_softness, 0.0..=1.0).text("Softness"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_sun_shadow, 0.0..=1.0).text("Shadow"));
        if ui
            .button("Bake Sun")
            .on_hover_text("Multiply vertex colours by a directional light term")
            .clicked()
        {
            self.bake_terrain_sun();
        }
    }
}

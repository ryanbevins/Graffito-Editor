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

/// Colour and normal state of one asset, captured for vertex paint undo.
///
/// Normals ride along because the sun bake can smooth them; restoring only the
/// colours would leave the shading permanently changed.
#[derive(Clone)]
pub(super) struct VertexPaintSnapshot {
    id: AssetId,
    /// Flattened over meshes then primitives, matching the iteration order
    /// every paint operation uses.
    colors: Vec<Vec<sms_authoring::ColorSet>>,
    normals: Vec<Vec<[f32; 3]>>,
}

/// One undoable vertex paint operation, however many assets it touched.
pub(super) struct VertexPaintUndoRecord {
    label: String,
    before: Vec<VertexPaintSnapshot>,
    after: Vec<VertexPaintSnapshot>,
}

fn snapshot_paint_state(id: AssetId, document: &ModelAssetDocument) -> VertexPaintSnapshot {
    let mut colors = Vec::new();
    let mut normals = Vec::new();
    for mesh in &document.meshes {
        for primitive in &mesh.primitives {
            colors.push(primitive.colors.clone());
            normals.push(primitive.normals.clone());
        }
    }
    VertexPaintSnapshot {
        id,
        colors,
        normals,
    }
}

fn restore_paint_state(document: &mut ModelAssetDocument, snapshot: &VertexPaintSnapshot) {
    let mut index = 0usize;
    for mesh in &mut document.meshes {
        for primitive in &mut mesh.primitives {
            if let Some(colors) = snapshot.colors.get(index) {
                primitive.colors.clone_from(colors);
            }
            if let Some(normals) = snapshot.normals.get(index) {
                primitive.normals.clone_from(normals);
            }
            index += 1;
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
    /// State as loaded, so the commit can record an undo step without a second
    /// read from disk.
    baseline: VertexPaintSnapshot,
}

/// One vertex of a target, resolved into world space.
pub(super) struct WorldVertex {
    pub(super) position: [f32; 3],
    pub(super) normal: [f32; 3],
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

const IDENTITY_MATRIX: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Column-major 4x4 multiply, `a` applied after `b`.
fn multiply_matrix(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
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
fn mesh_node_transforms(document: &ModelAssetDocument) -> BTreeMap<u32, [[f32; 4]; 4]> {
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
    for (index, node) in document.nodes.iter().enumerate() {
        if let Some(mesh) = node.mesh {
            by_mesh.entry(mesh).or_insert(globals[index]);
        }
    }
    by_mesh
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
    fn terrain_paint_targets_scoped(&self, selection_only: bool) -> Vec<PaintTarget> {
        let Some(catalog) = self.model_catalog().ok() else {
            return Vec::new();
        };
        // A stroke with no selection used to fall through to every terrain
        // asset, which is how a brush aimed at a ramp ended up repainting the
        // floor. Scoped operations now require a selection outright.
        let selected = match selection_only {
            true => match self.selected_model_instance() {
                Some(instance) => Some(instance.placement.asset_id),
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
            .filter(|instance| selected.is_none_or(|asset| instance.placement.asset_id == asset))
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
                    baseline: snapshot_paint_state(id, &document),
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
        let nodes = mesh_node_transforms(&target.document);
        target
            .document
            .meshes
            .iter()
            .enumerate()
            .flat_map(|(mesh_index, mesh)| {
                // The node transform runs first, exactly as compilation does
                // it, then the instance placement puts it in the stage.
                let node = nodes
                    .get(&(mesh_index as u32))
                    .copied()
                    .unwrap_or(IDENTITY_MATRIX);
                let combined = multiply_matrix(transform, node);
                mesh.primitives
                    .iter()
                    .map(move |primitive| {
                        primitive
                            .positions
                            .iter()
                            .enumerate()
                            .map(|(index, position)| WorldVertex {
                                position: transform_point(combined, *position),
                                normal: transform_direction(
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
        if self.apply_vertex_paint_snapshots(&record.before, &label) {
            self.vertex_paint_redo_stack.push_back(record);
        }
        true
    }

    pub(super) fn redo_vertex_paint(&mut self) -> bool {
        let Some(record) = self.vertex_paint_redo_stack.pop_back() else {
            return false;
        };
        let label = format!("Redo {}", record.label.to_lowercase());
        if self.apply_vertex_paint_snapshots(&record.after, &label) {
            self.vertex_paint_undo_stack.push_back(record);
        }
        true
    }

    /// Writes snapshots back to their assets and refreshes the preview.
    fn apply_vertex_paint_snapshots(
        &mut self,
        snapshots: &[VertexPaintSnapshot],
        label: &str,
    ) -> bool {
        if !self.content_catalog_mutation_allowed(label) {
            return false;
        }
        let Ok(catalog) = self.model_catalog() else {
            return false;
        };
        let mut restored = 0usize;
        for snapshot in snapshots {
            let Ok(mut document) = catalog.load_asset(snapshot.id) else {
                continue;
            };
            restore_paint_state(&mut document, snapshot);
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
        if restored == 0 {
            return false;
        }
        self.force_refresh_model_catalog();
        self.rebuild_model_preview_cache();
        self.log
            .push(format!("{label} across {restored} terrain asset(s)."));
        true
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
        let mut before = Vec::new();
        let mut after = Vec::new();
        for mut target in targets {
            // Every path that writes colour lands here, so the material switch
            // is flipped in one place rather than per operation.
            enable_vertex_colors(&mut target.document);
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
        mut edit: impl FnMut(&WorldVertex, &mut [f32; 4]),
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
        self.edit_terrain_vertex_colors_scoped("Cleared vertex paint", true, |_, color| {
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
            |vertex, color| {
                let lambert = vec3_dot(vertex.normal, light);
                let hard = lambert.max(0.0);
                let wrap = lambert * 0.5 + 0.5;
                let blended = (hard + (wrap - hard) * softness).clamp(0.0, 1.0);
                let shade = (1.0 - shadow) + shadow * blended;
                for channel in color.iter_mut().take(3) {
                    *channel = (*channel * shade).clamp(0.0, 1.0);
                }
            },
        );
        self.end_vertex_paint_undo_group("Baked sun into vertex paint");
    }

    /// Darkens creases, following Blender's "Dirty Vertex Colors".
    ///
    /// Concavity at a welded position is the mean, over its edge neighbours, of
    /// the dot between the direction to that neighbour and the vertex normal.
    /// Positive means neighbours sit above the tangent plane, which is a crease
    /// ambient light cannot reach; convex and flat areas stay untouched.
    pub(super) fn bake_terrain_dirt(&mut self) {
        let amount = self.vertex_paint_dirt.clamp(0.0, 1.0);
        let ramp = self.vertex_paint_dirt_ramp.clamp(0.05, 8.0);
        let (yaw, pitch) = (
            self.vertex_paint_dirt_yaw.to_radians(),
            self.vertex_paint_dirt_pitch.to_radians(),
        );
        let direction = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let bias = self.vertex_paint_dirt_bias.clamp(0.0, 1.0);
        let mut targets = self.terrain_paint_targets_scoped(true);
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
                    let dirt =
                        primitive_cavity(primitive, &world[primitive_index], direction, bias);
                    primitive_index += 1;
                    let colors = primitive_colors_mut(primitive);
                    for (index, color) in colors.iter_mut().enumerate() {
                        // The ramp is a contrast curve on the cavity factor:
                        // above 1 the dirt tightens into the deepest creases,
                        // below 1 it spreads across the shallower ones.
                        let cavity = dirt
                            .get(index)
                            .copied()
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0)
                            .powf(ramp);
                        let shade = 1.0 - cavity * amount;
                        for channel in color.iter_mut().take(3) {
                            *channel = (*channel * shade).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
        self.commit_paint_targets(targets, "Baked cavity dirt into vertex paint");
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
        self.commit_paint_targets(targets, "Subdivided terrain");
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
        self.commit_paint_targets(targets, "Smoothed normals for baking");
    }

    /// Lays a gradient along a world axis over the terrain's bounds.
    pub(super) fn bake_terrain_ramp(&mut self) {
        let axis = self.vertex_paint_ramp_axis.min(2);
        let start = self.vertex_paint_ramp_start;
        let end = self.vertex_paint_ramp_end;
        let curve = self.vertex_paint_ramp_curve.clamp(0.05, 8.0);
        let invert = self.vertex_paint_ramp_invert;
        let color = self.vertex_paint_color;
        let strength = self.vertex_paint_strength.clamp(0.0, 1.0);

        // Bounds first, so start and end read as fractions of the terrain
        // rather than raw world coordinates the user would have to look up.
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        for target in &self.terrain_paint_targets_scoped(true) {
            for primitive in Self::target_world_vertices(target) {
                for vertex in primitive {
                    minimum = minimum.min(vertex.position[axis]);
                    maximum = maximum.max(vertex.position[axis]);
                }
            }
        }
        if !minimum.is_finite() || maximum <= minimum {
            self.log
                .push("Ramp needs terrain with a measurable extent.".to_string());
            return;
        }
        let span = maximum - minimum;
        self.edit_terrain_vertex_colors_scoped(
            "Baked ramp into vertex paint",
            true,
            |vertex, value| {
                let normalised = ((vertex.position[axis] - minimum) / span).clamp(0.0, 1.0);
                let range = (end - start).abs().max(1e-4);
                let mut t = ((normalised - start.min(end)) / range).clamp(0.0, 1.0);
                if invert {
                    t = 1.0 - t;
                }
                let amount = t.powf(curve) * strength;
                for axis in 0..3 {
                    value[axis] += (color[axis] - value[axis]) * amount;
                    value[axis] = value[axis].clamp(0.0, 1.0);
                }
            },
        );
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
        self.commit_paint_targets(targets, "Smoothed vertex paint");
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

/// Per-vertex cavity factor in 0..1, where 1 is a deep crease.
pub(super) fn primitive_cavity(
    primitive: &sms_authoring::ModelPrimitive,
    world: &[WorldVertex],
    direction: [f32; 3],
    bias: f32,
) -> Vec<f32> {
    // World space, because a non-uniform scale on the node or the placement
    // would otherwise skew the result along an axis.
    let positions = world
        .iter()
        .map(|vertex| vertex.position)
        .collect::<Vec<_>>();
    let welds = weld_positions(&positions);
    let adjacency = primitive_adjacency(&primitive.indices, &welds);

    // Angle weighted, not one equal share per edge. A grid triangulated along a
    // single diagonal gives every vertex neighbours on that diagonal and none
    // on the other, so weighting edges equally leans the whole bake along it.
    // That is the tilt: it is in the triangulation, not the transform.
    // Weighting by the corner angle each edge subtends makes the sampling even
    // no matter how the surface was cut up.
    let corner_angle = |origin: [f32; 3], a: [f32; 3], b: [f32; 3]| -> f32 {
        let u = vec3_sub(a, origin);
        let v = vec3_sub(b, origin);
        let (lu, lv) = (vec3_dot(u, u).sqrt(), vec3_dot(v, v).sqrt());
        if lu <= f32::EPSILON || lv <= f32::EPSILON {
            return 0.0;
        }
        (vec3_dot(u, v) / (lu * lv)).clamp(-1.0, 1.0).acos()
    };

    let triangles = primitive.indices.chunks_exact(3).filter_map(|triangle| {
        let indices = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        indices
            .iter()
            .all(|index| *index < positions.len() && *index < welds.len())
            .then_some(indices)
    });
    let corners = triangles
        .flat_map(|indices| {
            (0..3).map(move |corner| {
                (
                    indices[corner],
                    indices[(corner + 1) % 3],
                    indices[(corner + 2) % 3],
                )
            })
        })
        .filter_map(|(origin, a, b)| {
            let angle = corner_angle(positions[origin], positions[a], positions[b]);
            (angle > f32::EPSILON).then_some((origin, a, b, angle))
        })
        .collect::<Vec<_>>();

    // Normals get the same weighting, so a long thin triangle does not shout
    // over a compact one that covers more of the surface around the vertex.
    let mut normals: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
    for (origin, _, _, angle) in &corners {
        let normal = world
            .get(*origin)
            .map(|vertex| vertex.normal)
            .unwrap_or([0.0, 1.0, 0.0]);
        let entry = normals.entry(welds[*origin]).or_insert([0.0; 3]);
        for axis in 0..3 {
            entry[axis] += normal[axis] * angle;
        }
    }
    let normals: BTreeMap<usize, [f32; 3]> = normals
        .into_iter()
        .filter_map(|(weld, normal)| {
            let length = vec3_dot(normal, normal).sqrt();
            (length > f32::EPSILON).then(|| (weld, vec3_scale(normal, 1.0 / length)))
        })
        .collect();

    let mut sums: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
    for (origin, a, b, angle) in &corners {
        let weld = welds[*origin];
        let Some(normal) = normals.get(&weld) else {
            continue;
        };
        // The corner's weight is split between the two edges leaving it.
        let weight = angle * 0.5;
        for neighbour in [a, b] {
            let delta = vec3_sub(positions[*neighbour], positions[*origin]);
            let distance = vec3_dot(delta, delta).sqrt();
            if distance <= f32::EPSILON {
                continue;
            }
            let entry = sums.entry(weld).or_insert((0.0, 0.0));
            entry.0 += vec3_dot(vec3_scale(delta, 1.0 / distance), *normal) * weight;
            entry.1 += weight;
        }
    }

    let cavity: BTreeMap<usize, f32> = sums
        .into_iter()
        .filter(|(_, (_, weight))| *weight > f32::EPSILON)
        .map(|(weld, (sum, weight))| (weld, (sum / weight).clamp(0.0, 1.0)))
        .collect();

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

    // Rescaled to fill 0..1. The raw term is a mean of dot products over every
    // neighbour, including the coplanar ones, so even a right-angle fold only
    // reaches about 0.1 and the Dirty slider looks like it does nothing. Since
    // it never spans its own range, grade it against the sharpest crease the
    // mesh actually has.
    let peak = relaxed.values().copied().fold(0.0f32, f32::max);
    // Below this there is no crease to speak of, only the noise a flat surface
    // makes at its border, and stretching that to full black would be a lie.
    let graded: BTreeMap<usize, f32> = match peak > 0.004 {
        true => relaxed
            .into_iter()
            .map(|(weld, value)| (weld, (value / peak).clamp(0.0, 1.0)))
            .collect(),
        false => BTreeMap::new(),
    };

    let bias = bias.clamp(0.0, 1.0);
    welds
        .iter()
        .map(|weld| {
            let value = graded.get(weld).copied().unwrap_or(0.0);
            // Direction: at 0 every crease dirties alike, at 1 only the ones
            // facing the chosen angle do. Half lambert, so it fades in instead
            // of cutting off along a hard terminator.
            let facing = normals
                .get(weld)
                .map(|normal| vec3_dot(*normal, direction) * 0.5 + 0.5)
                .unwrap_or(1.0);
            value * (1.0 - bias + bias * facing)
        })
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
        let Some(rect) = self.vertex_paint_rect else {
            return;
        };
        if stroke.is_empty() {
            return;
        }
        if self.selected_model_instance().is_none() {
            self.log.push(
                "Select a terrain instance in the hierarchy before painting; the brush only                  paints the selected one."
                    .to_string(),
            );
            return;
        }
        let projection = self.camera_projection(rect);
        let radius = self.vertex_paint_radius.max(1.0);
        let strength = self.vertex_paint_strength.clamp(0.0, 1.0);
        let color = self.vertex_paint_color;
        // Shift is a quick erase, so a correction does not mean going to the
        // panel and back for every stroke.
        let mode = match self.vertex_paint_stroke_erases {
            true => VertexPaintMode::Eraser,
            false => self.vertex_paint_mode,
        };
        let label = match mode {
            VertexPaintMode::Brush => "Painted vertex colour",
            VertexPaintMode::Eraser => "Erased vertex colour",
            VertexPaintMode::Smooth => "Softened vertex colour",
        };
        // Scoped to the selected instance so a stroke cannot bleed onto the
        // terrain underneath it.
        // Counters so a stroke that does nothing can say why: whether any
        // vertex was even considered, and how close the nearest one came.
        let mut considered = 0usize;
        let mut painted = 0usize;
        let mut closest = f32::INFINITY;
        self.edit_terrain_vertex_colors_scoped(label, true, |vertex, value| {
            considered += 1;
            // Screen space, not a surface raycast. Casting a ray and painting
            // around the hit point misses wherever the ray leaves the mesh, so
            // edges and silhouettes could not be painted at all. Projecting the
            // vertices instead also makes the radius genuinely pixels, which is
            // what the slider claims.
            let Some((screen, _)) = projection.project_world_to_screen(vertex.position) else {
                return;
            };
            let mut nearest = f32::INFINITY;
            for sample in &stroke {
                nearest = nearest.min(screen.distance(*sample));
            }
            closest = closest.min(nearest);
            if nearest >= radius {
                return;
            }
            painted += 1;
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
        self.log.push(format!(
            "Vertex paint: {painted} of {considered} vertices within {radius:.0}px; nearest was {closest:.0}px."
        ));
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
        self.vertex_paint_rect = Some(rect);
        let pointer = ui.input(|input| input.pointer.interact_pos());
        // Surface hit is only for drawing the ring; painting itself is screen
        // space, so a miss here never stops a stroke.
        self.vertex_paint_cursor = response
            .hovered()
            .then(|| pointer.and_then(|pointer| self.vertex_paint_surface_hit(rect, pointer)))
            .flatten();
        let (down, shift) = ui.input(|input| (input.pointer.primary_down(), input.modifiers.shift));
        self.vertex_paint_shift_erase = shift;
        if down && response.hovered() {
            if self.vertex_paint_stroke.is_empty() {
                // Latched at the start of the stroke: letting go of shift
                // halfway through should not switch what the stroke is doing.
                self.vertex_paint_stroke_erases = shift;
            }
            if let Some(pointer) = pointer {
                self.vertex_paint_stroke.push(pointer);
            }
            return true;
        }
        if !down && !self.vertex_paint_stroke.is_empty() {
            self.commit_vertex_paint_stroke();
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
        let erasing =
            self.vertex_paint_shift_erase || self.vertex_paint_mode != VertexPaintMode::Brush;
        let color = match erasing {
            true => egui::Color32::from_rgb(230, 230, 230),
            false => egui::Color32::from_rgb(
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
        // Unscoped on purpose: every terrain asset needs its colour set and
        // material before anything is selected.
        self.edit_terrain_vertex_colors_scoped(
            "Prepared terrain for vertex paint",
            false,
            |_, _| {},
        );
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
        ui.label("Hold Shift while painting to erase. Ctrl+Z and Ctrl+Y step through strokes.");

        ui.separator();
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
                .button("Clear Paint")
                .on_hover_text("Reset every terrain vertex to white")
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
        ui.strong("Dirty (cavity AO)");
        ui.add(egui::Slider::new(&mut self.vertex_paint_dirt, 0.0..=1.0).text("Dirty"));
        ui.add(
            egui::Slider::new(&mut self.vertex_paint_dirt_ramp, 0.05..=8.0)
                .logarithmic(true)
                .text("Ramp"),
        )
        .on_hover_text("Above 1 tightens dirt into deep creases; below 1 spreads it wider");
        ui.add(egui::Slider::new(&mut self.vertex_paint_dirt_bias, 0.0..=1.0).text("Direction"))
            .on_hover_text("0 dirties every crease equally; 1 only those facing the angle below");
        ui.add_enabled_ui(self.vertex_paint_dirt_bias > 0.0, |ui| {
            ui.add(egui::Slider::new(&mut self.vertex_paint_dirt_yaw, -180.0..=180.0).text("Yaw"));
            ui.add(
                egui::Slider::new(&mut self.vertex_paint_dirt_pitch, -90.0..=90.0).text("Pitch"),
            );
        });
        if ui
            .button("Bake Dirt")
            .on_hover_text("Darken creases and inner edges")
            .clicked()
        {
            self.bake_terrain_dirt();
        }

        ui.separator();
        ui.strong("Ramp");
        ui.horizontal(|ui| {
            for (index, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                ui.selectable_value(&mut self.vertex_paint_ramp_axis, index, label);
            }
            ui.checkbox(&mut self.vertex_paint_ramp_invert, "Invert");
        });
        ui.add(egui::Slider::new(&mut self.vertex_paint_ramp_start, 0.0..=1.0).text("Start"));
        ui.add(egui::Slider::new(&mut self.vertex_paint_ramp_end, 0.0..=1.0).text("End"));
        ui.add(
            egui::Slider::new(&mut self.vertex_paint_ramp_curve, 0.05..=8.0)
                .logarithmic(true)
                .text("Curve"),
        );
        if ui
            .button("Bake Ramp")
            .on_hover_text("Blend the brush colour along an axis across the terrain bounds")
            .clicked()
        {
            self.bake_terrain_ramp();
        }

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

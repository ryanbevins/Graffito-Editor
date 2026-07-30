//! Cutting a terrain mesh along its intersection with another one.
//!
//! This is a seam cut, not solid CSG. Stage terrain here is open surfaces: a
//! ramp with no underside sitting on a floor with no thickness. "Inside" is
//! undefined for those, so union and difference have nothing to compute
//! against. What is well defined is where the two surfaces cross, and that is
//! the line worth having as real geometry.
//!
//! Splitting a triangle along a plane adds vertices without moving the
//! surface, so the cut never changes the shape of the stage. That is the whole
//! point: it buys resolution to paint and to bake against, exactly where two
//! meshes meet and where contact shading wants detail.

use super::*;

use std::collections::BTreeMap;

use sms_authoring::{AssetId, ModelInstanceExportMode};

use crate::vertex_paint::{invert_affine, mesh_node_transforms, multiply_matrix, transform_point};

/// Triangles past this and the cut is refused rather than run.
///
/// Each cutter plane can split a triangle into three, so a pathological pair
/// of meshes grows fast. The editor freezing is worse than the cut not
/// happening.
const TRIANGLE_CEILING: usize = 400_000;

/// How far off a plane a vertex has to sit before it counts as crossing it.
///
/// Sunshine works in whole world units over stages thousands of units wide, so
/// this is tight in context while still swallowing float noise.
const PLANE_EPSILON: f32 = 1e-3;

/// One vertex with everything a split has to carry through.
#[derive(Clone)]
struct CutVertex {
    position: [f32; 3],
    normal: [f32; 3],
    tex: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
}

fn lerp_vertex(a: &CutVertex, b: &CutVertex, t: f32) -> CutVertex {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    CutVertex {
        position: std::array::from_fn(|axis| mix(a.position[axis], b.position[axis])),
        normal: std::array::from_fn(|axis| mix(a.normal[axis], b.normal[axis])),
        tex: a
            .tex
            .iter()
            .zip(b.tex.iter())
            .map(|(x, y)| std::array::from_fn(|axis| mix(x[axis], y[axis])))
            .collect(),
        colors: a
            .colors
            .iter()
            .zip(b.colors.iter())
            .map(|(x, y)| std::array::from_fn(|axis| mix(x[axis], y[axis])))
            .collect(),
    }
}

/// Unit normal and offset of the plane a triangle lies in.
fn triangle_plane(triangle: &[[f32; 3]; 3]) -> Option<([f32; 3], f32)> {
    let normal = vec3_cross(
        vec3_sub(triangle[1], triangle[0]),
        vec3_sub(triangle[2], triangle[0]),
    );
    let length = vec3_dot(normal, normal).sqrt();
    if length <= f32::EPSILON {
        return None;
    }
    let normal = vec3_scale(normal, 1.0 / length);
    Some((normal, vec3_dot(normal, triangle[0])))
}

/// Where a triangle crosses a plane, as a segment.
///
/// Returns nothing when the triangle sits wholly on one side, and nothing when
/// it lies in the plane: a coplanar overlap has no single crossing line, and
/// splitting along one would be arbitrary.
fn triangle_plane_segment(
    triangle: &[[f32; 3]; 3],
    normal: [f32; 3],
    offset: f32,
) -> Option<([f32; 3], [f32; 3])> {
    let distances: [f32; 3] =
        std::array::from_fn(|corner| vec3_dot(normal, triangle[corner]) - offset);
    if distances.iter().all(|d| *d > PLANE_EPSILON) || distances.iter().all(|d| *d < -PLANE_EPSILON)
    {
        return None;
    }
    let mut points = Vec::with_capacity(2);
    for corner in 0..3 {
        let (near, far) = (distances[corner], distances[(corner + 1) % 3]);
        if near.abs() <= PLANE_EPSILON {
            points.push(triangle[corner]);
        }
        if (near > PLANE_EPSILON && far < -PLANE_EPSILON)
            || (near < -PLANE_EPSILON && far > PLANE_EPSILON)
        {
            let t = near / (near - far);
            points.push(vec3_add(
                triangle[corner],
                vec3_scale(vec3_sub(triangle[(corner + 1) % 3], triangle[corner]), t),
            ));
        }
    }
    match points.len() {
        2 => Some((points[0], points[1])),
        _ => None,
    }
}

/// Whether any part of a segment lying in a triangle's plane falls inside it.
///
/// Parametric clipping against the triangle's three edges, each treated as an
/// inward half-plane.
fn segment_touches_triangle(
    start: [f32; 3],
    end: [f32; 3],
    triangle: &[[f32; 3]; 3],
    normal: [f32; 3],
) -> bool {
    let direction = vec3_sub(end, start);
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for corner in 0..3 {
        let edge = vec3_sub(triangle[(corner + 1) % 3], triangle[corner]);
        let mut inward = vec3_cross(normal, edge);
        // Point it at the corner the edge leaves out, so winding does not
        // decide whether the test passes.
        if vec3_dot(
            inward,
            vec3_sub(triangle[(corner + 2) % 3], triangle[corner]),
        ) < 0.0
        {
            inward = vec3_scale(inward, -1.0);
        }
        let at_start = vec3_dot(inward, vec3_sub(start, triangle[corner]));
        let along = vec3_dot(inward, direction);
        if along.abs() <= f32::EPSILON {
            if at_start < -PLANE_EPSILON {
                return false;
            }
            continue;
        }
        let crossing = -at_start / along;
        match along > 0.0 {
            true => low = low.max(crossing),
            false => high = high.min(crossing),
        }
        if low > high {
            return false;
        }
    }
    true
}

/// Splits a triangle along a plane, keeping both halves.
///
/// The surface is unchanged: every new vertex sits on an edge of the triangle
/// it came from. Only the topology gets denser.
fn split_triangle(
    triangle: &[CutVertex; 3],
    normal: [f32; 3],
    offset: f32,
) -> Option<Vec<[CutVertex; 3]>> {
    let distances: [f32; 3] =
        std::array::from_fn(|corner| vec3_dot(normal, triangle[corner].position) - offset);
    let above = distances.iter().filter(|d| **d > PLANE_EPSILON).count();
    let below = distances.iter().filter(|d| **d < -PLANE_EPSILON).count();
    if above == 0 || below == 0 {
        return None;
    }

    let crossing = |from: usize, to: usize| {
        let t = distances[from] / (distances[from] - distances[to]);
        lerp_vertex(&triangle[from], &triangle[to], t.clamp(0.0, 1.0))
    };

    // One corner already sits on the plane: the split runs from it to the
    // opposite edge, giving two triangles rather than three.
    if let Some(corner) = (0..3).find(|corner| distances[*corner].abs() <= PLANE_EPSILON) {
        let (next, last) = ((corner + 1) % 3, (corner + 2) % 3);
        let middle = crossing(next, last);
        return Some(vec![
            [
                triangle[corner].clone(),
                triangle[next].clone(),
                middle.clone(),
            ],
            [triangle[corner].clone(), middle, triangle[last].clone()],
        ]);
    }

    // Otherwise one corner stands alone on its side of the plane.
    let alone = (0..3).find(|corner| match above == 1 {
        true => distances[*corner] > PLANE_EPSILON,
        false => distances[*corner] < -PLANE_EPSILON,
    })?;
    let (next, last) = ((alone + 1) % 3, (alone + 2) % 3);
    let near = crossing(alone, next);
    let far = crossing(alone, last);
    Some(vec![
        [triangle[alone].clone(), near.clone(), far.clone()],
        [near.clone(), triangle[next].clone(), triangle[last].clone()],
        [near, triangle[last].clone(), far],
    ])
}

impl SmsEditorApp {
    /// Every world-space triangle of a terrain asset, across all its placements.
    fn asset_world_triangles(&self, id: AssetId) -> Vec<[[f32; 3]; 3]> {
        let Ok(catalog) = self.model_catalog() else {
            return Vec::new();
        };
        let Ok(document) = catalog.load_asset(id) else {
            return Vec::new();
        };
        let nodes = mesh_node_transforms(&document);
        let placements = self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| instance.placement.asset_id == id)
            .map(|instance| instance.placement.transform)
            .collect::<Vec<_>>();

        let mut triangles = Vec::new();
        for placement in &placements {
            for (index, mesh) in document.meshes.iter().enumerate() {
                let node = nodes.get(&(index as u32)).copied().unwrap_or(IDENTITY);
                let matrix = multiply_matrix(*placement, node);
                for primitive in &mesh.primitives {
                    for face in primitive.indices.chunks_exact(3) {
                        let corners: Option<Vec<[f32; 3]>> = face
                            .iter()
                            .map(|index| {
                                primitive
                                    .positions
                                    .get(*index as usize)
                                    .map(|position| transform_point(matrix, *position))
                            })
                            .collect();
                        if let Some(corners) = corners {
                            triangles.push([corners[0], corners[1], corners[2]]);
                        }
                    }
                }
            }
        }
        triangles
    }

    /// Terrain instances in this stage that could act as a cutter.
    pub(super) fn boolean_cut_candidates(&self) -> Vec<(AssetId, String)> {
        let mut seen: BTreeMap<AssetId, String> = BTreeMap::new();
        for instance in self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| {
                instance.placement.export_mode == ModelInstanceExportMode::MapTerrain
            })
        {
            let name = self
                .model_catalog_entries
                .iter()
                .find(|entry| entry.id == instance.placement.asset_id)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| instance.placement.asset_id.to_string());
            seen.insert(instance.placement.asset_id, name);
        }
        seen.into_iter().collect()
    }

    /// Cuts the selected terrain along everywhere the chosen cutter crosses it.
    pub(super) fn cut_terrain_with_selected_cutter(&mut self) {
        let Some(cutter_id) = self.boolean_cutter else {
            self.log.push("Pick a cutter mesh first.".to_string());
            return;
        };
        let Some(selected) = self.selected_model_instance() else {
            self.log
                .push("Select the terrain instance to cut first.".to_string());
            return;
        };
        if selected.placement.asset_id == cutter_id {
            self.log
                .push("A mesh cannot cut itself; pick a different cutter.".to_string());
            return;
        }
        let cutter = self.asset_world_triangles(cutter_id);
        if cutter.is_empty() {
            self.log
                .push("That cutter has no geometry placed in this stage.".to_string());
            return;
        }

        let mut targets = self.terrain_paint_targets_scoped(true);
        if targets.is_empty() {
            self.log
                .push("Select a terrain instance to cut first.".to_string());
            return;
        }

        let mut added = 0usize;
        let mut total = 0usize;
        for target in &mut targets {
            let placement = target.transforms.first().copied().unwrap_or(IDENTITY);
            let nodes = mesh_node_transforms(&target.document);
            for (mesh_index, mesh) in target.document.meshes.iter_mut().enumerate() {
                let node = nodes.get(&(mesh_index as u32)).copied().unwrap_or(IDENTITY);
                // The cut runs in the target's own space, so the split
                // positions can be written straight back into the primitive.
                let Some(to_local) = invert_affine(multiply_matrix(placement, node)) else {
                    continue;
                };
                let local_cutter = cutter
                    .iter()
                    .map(|triangle| {
                        [
                            transform_point(to_local, triangle[0]),
                            transform_point(to_local, triangle[1]),
                            transform_point(to_local, triangle[2]),
                        ]
                    })
                    .collect::<Vec<_>>();

                for primitive in &mut mesh.primitives {
                    let before = primitive.indices.len() / 3;
                    match cut_primitive(primitive, &local_cutter) {
                        Ok(()) => {
                            let after = primitive.indices.len() / 3;
                            added += after.saturating_sub(before);
                            total += after;
                        }
                        Err(error) => {
                            self.log.push(error);
                            return;
                        }
                    }
                }
            }
        }

        if added == 0 {
            self.log.push(
                "Nothing was cut: the two meshes do not cross anywhere. Move them so they \
                 intersect, or pick a different cutter."
                    .to_string(),
            );
            return;
        }
        self.commit_paint_targets(targets, "Cut terrain along intersection");
        self.log.push(format!(
            "Added {added} triangle(s) along the seam; the mesh now has {total}."
        ));
    }
}

const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Splits every triangle of a primitive that a cutter triangle crosses.
fn cut_primitive(
    primitive: &mut sms_authoring::ModelPrimitive,
    cutter: &[[[f32; 3]; 3]],
) -> Result<(), String> {
    let tex_sets = primitive
        .tex_coords
        .iter()
        .map(|set| set.set)
        .collect::<Vec<_>>();
    let color_sets = primitive
        .colors
        .iter()
        .map(|set| set.set)
        .collect::<Vec<_>>();

    let vertex = |index: usize| CutVertex {
        position: primitive.positions.get(index).copied().unwrap_or_default(),
        normal: primitive
            .normals
            .get(index)
            .copied()
            .unwrap_or([0.0, 1.0, 0.0]),
        tex: primitive
            .tex_coords
            .iter()
            .map(|set| set.values.get(index).copied().unwrap_or_default())
            .collect(),
        colors: primitive
            .colors
            .iter()
            .map(|set| set.values.get(index).copied().unwrap_or([1.0; 4]))
            .collect(),
    };

    let mut working = primitive
        .indices
        .chunks_exact(3)
        .map(|face| {
            [
                vertex(face[0] as usize),
                vertex(face[1] as usize),
                vertex(face[2] as usize),
            ]
        })
        .collect::<Vec<_>>();

    for blade in cutter {
        let Some((normal, offset)) = triangle_plane(blade) else {
            continue;
        };
        let mut next = Vec::with_capacity(working.len());
        for triangle in working {
            let corners = [
                triangle[0].position,
                triangle[1].position,
                triangle[2].position,
            ];
            // The plane is unbounded but the cutter triangle is not, so the
            // crossing has to land inside it. Without this a blade would slice
            // the whole mesh along its plane instead of only where the two
            // surfaces actually meet.
            let crossed = triangle_plane_segment(&corners, normal, offset)
                .is_some_and(|(start, end)| segment_touches_triangle(start, end, blade, normal));
            match crossed
                .then(|| split_triangle(&triangle, normal, offset))
                .flatten()
            {
                Some(pieces) => next.extend(pieces),
                None => next.push(triangle),
            }
        }
        working = next;
        if working.len() > TRIANGLE_CEILING {
            return Err(format!(
                "Cut abandoned: it passed {TRIANGLE_CEILING} triangles. Simplify the cutter or \
                 cut against a smaller piece of terrain."
            ));
        }
    }

    let mut positions = Vec::with_capacity(working.len() * 3);
    let mut normals = Vec::with_capacity(working.len() * 3);
    let mut tex = vec![Vec::with_capacity(working.len() * 3); tex_sets.len()];
    let mut colors = vec![Vec::with_capacity(working.len() * 3); color_sets.len()];
    let mut indices = Vec::with_capacity(working.len() * 3);
    for triangle in &working {
        for corner in triangle {
            indices.push(positions.len() as u32);
            positions.push(corner.position);
            normals.push(corner.normal);
            for (slot, value) in tex.iter_mut().zip(corner.tex.iter()) {
                slot.push(*value);
            }
            for (slot, value) in colors.iter_mut().zip(corner.colors.iter()) {
                slot.push(*value);
            }
        }
    }

    // Vertices are no longer shared between triangles. Nothing downstream
    // needs them to be: painting and every bake weld by position, and the
    // compiler builds its own index buffer.
    primitive.positions = positions;
    primitive.normals = normals;
    primitive.tangents.clear();
    for (slot, set) in primitive.tex_coords.iter_mut().zip(tex) {
        slot.values = set;
    }
    for (slot, set) in primitive.colors.iter_mut().zip(colors) {
        slot.values = set;
    }
    primitive.indices = indices;
    Ok(())
}

impl SmsEditorApp {
    pub(super) fn boolean_cut_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Boolean Cut");
        ui.label(
            "Cuts the selected terrain wherever another mesh passes through it, adding vertices \
             along the seam. The shape does not change: every new vertex sits on an edge of the \
             triangle it came from.",
        );
        ui.separator();

        let selected = self
            .selected_model_instance()
            .map(|instance| instance.placement.asset_id);
        let name_of = |editor: &Self, id: sms_authoring::AssetId| {
            editor
                .model_catalog_entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        match selected {
            Some(id) => ui.label(format!("Cutting: {}", name_of(self, id))),
            None => ui.colored_label(
                egui::Color32::from_rgb(220, 170, 90),
                "Select the terrain instance to cut in the hierarchy.",
            ),
        };

        let candidates = self
            .boolean_cut_candidates()
            .into_iter()
            // A mesh cutting itself would split every triangle against its own
            // plane and gain nothing.
            .filter(|(id, _)| Some(*id) != selected)
            .collect::<Vec<_>>();
        let current = self
            .boolean_cutter
            .map(|id| name_of(self, id))
            .unwrap_or_else(|| "None".to_string());
        egui::ComboBox::from_label("Cutter")
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.boolean_cutter, None, "None");
                for (id, name) in &candidates {
                    ui.selectable_value(&mut self.boolean_cutter, Some(*id), name);
                }
            });
        if candidates.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(220, 170, 90),
                "No other terrain in this stage. A cutter has to be flagged 'Bake as map \
                 terrain' too.",
            );
        }

        ui.separator();
        if ui
            .add_enabled(
                selected.is_some() && self.boolean_cutter.is_some(),
                egui::Button::new("Cut Along Intersection"),
            )
            .on_hover_text("Split the selected terrain everywhere the cutter crosses it")
            .clicked()
        {
            self.cut_terrain_with_selected_cutter();
        }
        ui.label(
            "Seam cut, not solid CSG. Stage terrain is open surfaces -- a ramp with no underside \
             on a floor with no thickness -- so there is no inside for union or difference to \
             work against.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_vertex(position: [f32; 3]) -> CutVertex {
        CutVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            tex: Vec::new(),
            colors: Vec::new(),
        }
    }

    #[test]
    fn a_plane_through_the_middle_splits_a_triangle_in_three() {
        // Third corner off the plane, so one corner stands alone and the cut
        // runs to both of the edges leaving it.
        let triangle = [
            flat_vertex([-100.0, 0.0, 0.0]),
            flat_vertex([100.0, 0.0, 0.0]),
            flat_vertex([20.0, 0.0, 100.0]),
        ];
        let pieces = split_triangle(&triangle, [1.0, 0.0, 0.0], 0.0).expect("a crossing");
        assert_eq!(pieces.len(), 3);
        // Splitting is topology only: every new corner has to sit on the
        // original surface, or the cut would have moved the stage.
        for piece in &pieces {
            for corner in piece {
                assert!(
                    corner.position[1].abs() < 1e-4,
                    "a split moved a vertex off the surface: {:?}",
                    corner.position
                );
            }
        }
    }

    #[test]
    fn a_corner_already_on_the_plane_splits_in_two() {
        // Nothing to interpolate on the edges meeting that corner, so the cut
        // runs from it to the opposite edge and yields two triangles.
        let triangle = [
            flat_vertex([-100.0, 0.0, 0.0]),
            flat_vertex([100.0, 0.0, 0.0]),
            flat_vertex([0.0, 0.0, 100.0]),
        ];
        let pieces = split_triangle(&triangle, [1.0, 0.0, 0.0], 0.0).expect("a crossing");
        assert_eq!(pieces.len(), 2);
    }

    #[test]
    fn a_plane_that_misses_leaves_the_triangle_alone() {
        let triangle = [
            flat_vertex([10.0, 0.0, 0.0]),
            flat_vertex([100.0, 0.0, 0.0]),
            flat_vertex([50.0, 0.0, 100.0]),
        ];
        assert!(split_triangle(&triangle, [1.0, 0.0, 0.0], 0.0).is_none());
    }

    #[test]
    fn a_blade_only_cuts_where_it_actually_reaches() {
        // The blade's plane crosses the whole floor, but the blade itself is a
        // small triangle off to one side. Only the part it reaches may split,
        // or a cutter would slice the entire stage along its plane.
        let blade = [[0.0, -10.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
        let (normal, offset) = triangle_plane(&blade).expect("a plane");
        let near = [[-50.0, 0.0, 5.0], [50.0, 0.0, 5.0], [0.0, 0.0, 8.0]];
        let far = [[-50.0, 0.0, 900.0], [50.0, 0.0, 900.0], [0.0, 0.0, 950.0]];
        for (triangle, expected) in [(near, true), (far, false)] {
            let reached = triangle_plane_segment(&triangle, normal, offset)
                .is_some_and(|(start, end)| segment_touches_triangle(start, end, &blade, normal));
            assert_eq!(reached, expected, "triangle {triangle:?}");
        }
    }
}

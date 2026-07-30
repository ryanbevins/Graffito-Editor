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

use crate::triangle_bvh::TriangleBvh;
use crate::vertex_paint::{
    invert_affine, mesh_node_transforms, multiply_matrix, transform_direction, transform_point,
};

/// Triangles past this and the cut is refused rather than run.
///
/// Each cutter plane can split a triangle into three, so a pathological pair
/// of meshes grows fast. The editor freezing is worse than the cut not
/// happening.
const TRIANGLE_CEILING: usize = 400_000;

/// How far a coverage ray looks for the cutter.
///
/// Stages are thousands of units across and a cutter can sit well above what
/// it covers, so this is deliberately far past any of that.
const COVER_REACH: f32 = 1.0e7;

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
    tangent: Option<[f32; 4]>,
    tex: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
}

fn normalize_tangent(mut tangent: [f32; 4], fallback_handedness: f32) -> [f32; 4] {
    let length =
        (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt();
    if length > f32::EPSILON {
        for component in tangent.iter_mut().take(3) {
            *component /= length;
        }
    }
    tangent[3] = if tangent[3].abs() > f32::EPSILON {
        tangent[3].signum()
    } else {
        fallback_handedness
    };
    tangent
}

#[cfg(test)]
fn lerp_vertex(a: &CutVertex, b: &CutVertex, t: f32) -> CutVertex {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    CutVertex {
        position: std::array::from_fn(|axis| mix(a.position[axis], b.position[axis])),
        normal: std::array::from_fn(|axis| mix(a.normal[axis], b.normal[axis])),
        tangent: a.tangent.zip(b.tangent).map(|(a, b)| {
            normalize_tangent(
                std::array::from_fn(|axis| mix(a[axis], b[axis])),
                a[3].signum(),
            )
        }),
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

/// Clips a segment lying in a triangle's plane to the finite triangle.
///
/// Parametric clipping against the triangle's three edges, each treated as an
/// inward half-plane.
fn clip_segment_to_triangle(
    start: [f32; 3],
    end: [f32; 3],
    triangle: &[[f32; 3]; 3],
    normal: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
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
                return None;
            }
            continue;
        }
        let crossing = -at_start / along;
        match along > 0.0 {
            true => low = low.max(crossing),
            false => high = high.min(crossing),
        }
        if low > high {
            return None;
        }
    }
    if high - low <= PLANE_EPSILON {
        return None;
    }
    Some((
        vec3_add(start, vec3_scale(direction, low.clamp(0.0, 1.0))),
        vec3_add(start, vec3_scale(direction, high.clamp(0.0, 1.0))),
    ))
}

#[cfg(test)]
fn segment_touches_triangle(
    start: [f32; 3],
    end: [f32; 3],
    triangle: &[[f32; 3]; 3],
    normal: [f32; 3],
) -> bool {
    clip_segment_to_triangle(start, end, triangle, normal).is_some()
}

/// Splits a triangle along a plane, keeping both halves.
///
/// The surface is unchanged: every new vertex sits on an edge of the triangle
/// it came from. Only the topology gets denser.
#[cfg(test)]
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

fn barycentric_weights(point: [f32; 3], triangle: &[[f32; 3]; 3]) -> Option<[f32; 3]> {
    let edge_a = vec3_sub(triangle[1], triangle[0]);
    let edge_b = vec3_sub(triangle[2], triangle[0]);
    let offset = vec3_sub(point, triangle[0]);
    let aa = vec3_dot(edge_a, edge_a);
    let ab = vec3_dot(edge_a, edge_b);
    let bb = vec3_dot(edge_b, edge_b);
    let oa = vec3_dot(offset, edge_a);
    let ob = vec3_dot(offset, edge_b);
    let denominator = aa * bb - ab * ab;
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    let second = (bb * oa - ab * ob) / denominator;
    let third = (aa * ob - ab * oa) / denominator;
    Some([1.0 - second - third, second, third])
}

fn interpolate_cut_vertex(triangle: &[CutVertex; 3], point: [f32; 3]) -> Option<CutVertex> {
    let positions = [
        triangle[0].position,
        triangle[1].position,
        triangle[2].position,
    ];
    let weights = barycentric_weights(point, &positions)?;
    let blend = |values: [[f32; 3]; 3]| {
        std::array::from_fn(|axis| {
            values[0][axis] * weights[0]
                + values[1][axis] * weights[1]
                + values[2][axis] * weights[2]
        })
    };
    let normal = blend([triangle[0].normal, triangle[1].normal, triangle[2].normal]);
    let tangent = triangle[0]
        .tangent
        .zip(triangle[1].tangent)
        .zip(triangle[2].tangent)
        .map(|((a, b), c)| {
            normalize_tangent(
                std::array::from_fn(|axis| {
                    a[axis] * weights[0] + b[axis] * weights[1] + c[axis] * weights[2]
                }),
                a[3].signum(),
            )
        });
    let tex = (0..triangle[0].tex.len())
        .map(|set| {
            std::array::from_fn(|axis| {
                triangle[0].tex[set][axis] * weights[0]
                    + triangle[1].tex[set][axis] * weights[1]
                    + triangle[2].tex[set][axis] * weights[2]
            })
        })
        .collect();
    let colors = (0..triangle[0].colors.len())
        .map(|set| {
            std::array::from_fn(|axis| {
                triangle[0].colors[set][axis] * weights[0]
                    + triangle[1].colors[set][axis] * weights[1]
                    + triangle[2].colors[set][axis] * weights[2]
            })
        })
        .collect();
    Some(CutVertex {
        position: point,
        normal,
        tangent,
        tex,
        colors,
    })
}

fn positions_match(a: [f32; 3], b: [f32; 3]) -> bool {
    vec3_dot(vec3_sub(a, b), vec3_sub(a, b)) <= PLANE_EPSILON * PLANE_EPSILON
}

fn point_on_segment(point: [f32; 3], start: [f32; 3], end: [f32; 3]) -> bool {
    let segment = vec3_sub(end, start);
    let length_squared = vec3_dot(segment, segment);
    if length_squared <= f32::EPSILON {
        return positions_match(point, start);
    }
    let t = vec3_dot(vec3_sub(point, start), segment) / length_squared;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    positions_match(point, vec3_add(start, vec3_scale(segment, t)))
}

/// Inserts a constrained point into an existing triangulation of `original`.
///
/// Splitting every triangle that shares a hit edge keeps the result watertight.
/// Inserting the segment's first endpoint also ensures every region that can
/// contain the second endpoint has the first as a corner, so the constrained
/// segment itself becomes a real edge.
fn insert_cut_point(
    pieces: &mut Vec<[CutVertex; 3]>,
    original: &[CutVertex; 3],
    point: [f32; 3],
) -> bool {
    if pieces
        .iter()
        .flatten()
        .any(|vertex| positions_match(vertex.position, point))
    {
        return true;
    }
    let Some(vertex) = interpolate_cut_vertex(original, point) else {
        return false;
    };

    let mut split_edge = false;
    let mut edge_pieces = Vec::with_capacity(pieces.len() + 2);
    for triangle in pieces.drain(..) {
        let mut replacement = None;
        for (a, b, c) in [(0usize, 1usize, 2usize), (1, 2, 0), (2, 0, 1)] {
            if point_on_segment(point, triangle[a].position, triangle[b].position) {
                replacement = Some([
                    [triangle[a].clone(), vertex.clone(), triangle[c].clone()],
                    [vertex.clone(), triangle[b].clone(), triangle[c].clone()],
                ]);
                break;
            }
        }
        match replacement {
            Some(replacement) => {
                edge_pieces.extend(replacement);
                split_edge = true;
            }
            None => edge_pieces.push(triangle),
        }
    }
    *pieces = edge_pieces;
    if split_edge {
        return true;
    }

    let containing = pieces.iter().position(|triangle| {
        let positions = [
            triangle[0].position,
            triangle[1].position,
            triangle[2].position,
        ];
        barycentric_weights(point, &positions)
            .is_some_and(|weights| weights.iter().all(|weight| *weight >= -PLANE_EPSILON))
    });
    let Some(index) = containing else {
        return false;
    };
    let triangle = pieces.swap_remove(index);
    pieces.extend([
        [triangle[0].clone(), triangle[1].clone(), vertex.clone()],
        [triangle[1].clone(), triangle[2].clone(), vertex.clone()],
        [triangle[2].clone(), triangle[0].clone(), vertex],
    ]);
    true
}

fn split_triangle_along_segment(
    triangle: &[CutVertex; 3],
    start: [f32; 3],
    end: [f32; 3],
) -> Option<Vec<[CutVertex; 3]>> {
    if positions_match(start, end) {
        return None;
    }
    let mut pieces = vec![triangle.clone()];
    if !insert_cut_point(&mut pieces, triangle, start)
        || !insert_cut_point(&mut pieces, triangle, end)
        || pieces.len() == 1
    {
        return None;
    }
    Some(pieces)
}

/// Intersection of `triangle` with the infinite strip made by extruding one
/// cutter edge along `axis`.
///
/// Hole coverage is an axis projection, so a cutter hovering above the target
/// still needs its silhouette inserted into the target topology. The strip is
/// finite along the edge and unbounded only along the extrusion axis.
fn triangle_extruded_edge_segment(
    triangle: &[[f32; 3]; 3],
    edge_start: [f32; 3],
    edge_end: [f32; 3],
    axis: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let axis = vec3_normalize(axis);
    let edge = vec3_sub(edge_end, edge_start);
    let lateral = vec3_sub(edge, vec3_scale(axis, vec3_dot(edge, axis)));
    let lateral_length_squared = vec3_dot(lateral, lateral);
    if lateral_length_squared <= f32::EPSILON {
        // This edge projects to a point, so it contributes no footprint side.
        return None;
    }
    let plane_normal = vec3_normalize(vec3_cross(edge, axis));
    let plane_offset = vec3_dot(plane_normal, edge_start);
    let (start, end) = triangle_plane_segment(triangle, plane_normal, plane_offset)?;
    let direction = vec3_sub(end, start);
    let edge_parameter =
        |point: [f32; 3]| vec3_dot(vec3_sub(point, edge_start), lateral) / lateral_length_squared;
    let at_start = edge_parameter(start);
    let along = edge_parameter(end) - at_start;
    let (mut low, mut high) = (0.0f32, 1.0f32);
    if along.abs() <= f32::EPSILON {
        if !(-PLANE_EPSILON..=1.0 + PLANE_EPSILON).contains(&at_start) {
            return None;
        }
    } else {
        let first = (0.0 - at_start) / along;
        let second = (1.0 - at_start) / along;
        low = low.max(first.min(second));
        high = high.min(first.max(second));
    }
    if high - low <= PLANE_EPSILON {
        return None;
    }
    Some((
        vec3_add(start, vec3_scale(direction, low.clamp(0.0, 1.0))),
        vec3_add(start, vec3_scale(direction, high.clamp(0.0, 1.0))),
    ))
}

fn terrain_document_changed(
    before: &sms_authoring::ModelAssetDocument,
    after: &sms_authoring::ModelAssetDocument,
) -> bool {
    before != after
}

impl SmsEditorApp {
    /// Every world-space triangle of one exact terrain placement.
    fn instance_world_triangles(&self, instance_id: uuid::Uuid) -> Vec<[[f32; 3]; 3]> {
        let Some((asset_id, placement)) = self
            .model_instances
            .iter()
            .find(|instance| {
                instance.stage_id.eq_ignore_ascii_case(&self.stage_id)
                    && instance.placement.instance_id == instance_id
            })
            .map(|instance| (instance.placement.asset_id, instance.placement.transform))
        else {
            return Vec::new();
        };
        let Ok(catalog) = self.model_catalog() else {
            return Vec::new();
        };
        let Ok(document) = catalog.load_asset(asset_id) else {
            return Vec::new();
        };
        let nodes = mesh_node_transforms(&document);

        let mut triangles = Vec::new();
        for (index, mesh) in document.meshes.iter().enumerate() {
            let node_transforms = nodes.get(&(index as u32)).cloned().unwrap_or_default();
            for node in node_transforms {
                let matrix = multiply_matrix(placement, node);
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
    pub(super) fn boolean_cut_candidates(&self) -> Vec<(uuid::Uuid, String)> {
        let instances = self
            .model_instances
            .iter()
            .filter(|instance| instance.stage_id.eq_ignore_ascii_case(&self.stage_id))
            .filter(|instance| {
                instance.placement.export_mode == ModelInstanceExportMode::MapTerrain
            })
            .collect::<Vec<_>>();
        let mut totals = BTreeMap::<AssetId, usize>::new();
        for instance in &instances {
            *totals.entry(instance.placement.asset_id).or_default() += 1;
        }
        let mut ordinals = BTreeMap::<AssetId, usize>::new();
        instances
            .into_iter()
            .map(|instance| {
                let asset_id = instance.placement.asset_id;
                let ordinal = ordinals.entry(asset_id).or_default();
                *ordinal += 1;
                let total = totals.get(&asset_id).copied().unwrap_or(1);
                let mut name = self
                    .model_catalog_entries
                    .iter()
                    .find(|entry| entry.id == asset_id)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| asset_id.to_string());
                if total > 1 {
                    name.push_str(&format!(" ({ordinal}/{total})"));
                }
                (instance.placement.instance_id, name)
            })
            .collect()
    }

    /// Cuts the selected terrain along everywhere the chosen cutter crosses it.
    pub(super) fn cut_terrain_with_selected_cutter(&mut self) {
        let Some(cutter_instance_id) = self.boolean_cutter else {
            self.log.push("Pick a cutter mesh first.".to_string());
            return;
        };
        let Some(selected_instance_id) = self
            .selected_model_instance()
            .map(|instance| instance.placement.instance_id)
        else {
            self.log
                .push("Select the terrain instance to cut first.".to_string());
            return;
        };
        if selected_instance_id == cutter_instance_id {
            self.log
                .push("A mesh cannot cut itself; pick a different cutter.".to_string());
            return;
        }
        let cutter = self.instance_world_triangles(cutter_instance_id);
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
        let mut removed = 0usize;
        let mut total = 0usize;
        let mut changed = false;
        for target in &mut targets {
            let before_document = target.document.clone();
            let placement = target.transform;
            let nodes = mesh_node_transforms(&target.document);
            for (mesh_index, mesh) in target.document.meshes.iter_mut().enumerate() {
                let node_transforms = nodes.get(&(mesh_index as u32)).cloned().unwrap_or_default();
                let cutters = node_transforms
                    .into_iter()
                    .filter_map(|node| {
                        let to_local = invert_affine(multiply_matrix(placement, node))?;
                        let local_cutter = TriangleBvh::build(
                            cutter
                                .iter()
                                .map(|triangle| {
                                    [
                                        transform_point(to_local, triangle[0]),
                                        transform_point(to_local, triangle[1]),
                                        transform_point(to_local, triangle[2]),
                                    ]
                                })
                                .collect(),
                        );
                        // The axis is a world direction, and the cut runs in
                        // the mesh's own space, so it has to travel there too.
                        let hole = self.boolean_cut_hole.then(|| {
                            let mut axis = [0.0f32; 3];
                            axis[self.boolean_cut_axis.min(2)] = 1.0;
                            vec3_normalize(transform_direction(to_local, axis))
                        });
                        Some((local_cutter, hole))
                    })
                    .collect::<Vec<_>>();
                for primitive in &mut mesh.primitives {
                    let before = primitive.indices.len() / 3;
                    for (local_cutter, hole) in &cutters {
                        if let Err(error) = cut_primitive(primitive, local_cutter, *hole) {
                            self.log.push(error);
                            return;
                        }
                    }
                    let after = primitive.indices.len() / 3;
                    added += after.saturating_sub(before);
                    removed += before.saturating_sub(after);
                    total += after;
                }
            }
            if self.boolean_cut_hole {
                let Some(to_local) = invert_affine(placement) else {
                    self.log.push(
                        "Cut abandoned: the selected terrain transform is degenerate.".into(),
                    );
                    return;
                };
                if let Some(collision) = target.document.collision.as_mut() {
                    let local_cutter = TriangleBvh::build(
                        cutter
                            .iter()
                            .map(|triangle| {
                                [
                                    transform_point(to_local, triangle[0]),
                                    transform_point(to_local, triangle[1]),
                                    transform_point(to_local, triangle[2]),
                                ]
                            })
                            .collect(),
                    );
                    let mut axis = [0.0f32; 3];
                    axis[self.boolean_cut_axis.min(2)] = 1.0;
                    let local_axis = vec3_normalize(transform_direction(to_local, axis));
                    if let Err(error) = cut_collision_document(collision, &local_cutter, local_axis)
                    {
                        self.log.push(error);
                        return;
                    }
                }
            }
            changed |= terrain_document_changed(&before_document, &target.document);
        }

        if !changed {
            self.log.push(
                "Nothing was cut: the two meshes do not cross anywhere. Move them so they \
                 intersect, or pick a different cutter."
                    .to_string(),
            );
            return;
        }
        self.commit_terrain_targets(targets, "Cut terrain along intersection", false);
        match self.boolean_cut_hole {
            true => self.log.push(format!(
                "Cut the covered area out: {removed} triangle(s) removed, {added} added along \
                 the seam. The mesh now has {total}."
            )),
            false => self.log.push(format!(
                "Added {added} triangle(s) along the seam; the mesh now has {total}."
            )),
        }
    }
}

/// Splits every triangle of a primitive that a cutter triangle crosses.
fn cut_primitive(
    primitive: &mut sms_authoring::ModelPrimitive,
    cutter: &TriangleBvh,
    hole: Option<[f32; 3]>,
) -> Result<bool, String> {
    let vertex = |index: usize| CutVertex {
        position: primitive.positions.get(index).copied().unwrap_or_default(),
        normal: primitive
            .normals
            .get(index)
            .copied()
            .unwrap_or([0.0, 1.0, 0.0]),
        tangent: primitive.tangents.get(index).copied(),
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

    // Each triangle carries the blade to resume at. Splitting by a plane
    // leaves both halves wholly on one side of it, so neither can be cut by
    // that blade again, and a blade that missed the parent cannot reach a
    // piece of it. Only ever moving forward is what makes this terminate.
    let mut pending = primitive
        .indices
        .chunks_exact(3)
        .map(|face| {
            (
                [
                    vertex(face[0] as usize),
                    vertex(face[1] as usize),
                    vertex(face[2] as usize),
                ],
                0usize,
            )
        })
        .collect::<Vec<_>>();
    let mut done: Vec<[CutVertex; 3]> = Vec::with_capacity(pending.len());
    let mut candidates = Vec::new();
    let mut projected_candidates = Vec::new();
    let mut tokens = Vec::new();
    let mut topology_changed = false;

    while let Some((triangle, resume)) = pending.pop() {
        let corners = [
            triangle[0].position,
            triangle[1].position,
            triangle[2].position,
        ];
        let low = std::array::from_fn(|axis| {
            corners[0][axis].min(corners[1][axis]).min(corners[2][axis]) - PLANE_EPSILON
        });
        let high = std::array::from_fn(|axis| {
            corners[0][axis].max(corners[1][axis]).max(corners[2][axis]) + PLANE_EPSILON
        });
        cutter.overlapping(low, high, &mut candidates);
        tokens.clear();
        match hole {
            None => tokens.extend(candidates.iter().map(|index| *index as usize)),
            Some(axis) => {
                // Token zero for a cutter triangle is its real surface.
                // Tokens one through three are the sides made by extruding
                // each edge along the selected hole axis.
                tokens.extend(candidates.iter().map(|index| *index as usize * 4));
                cutter.overlapping_projection(low, high, axis, &mut projected_candidates);
                for triangle_index in projected_candidates.iter().map(|index| *index as usize) {
                    tokens.extend([
                        triangle_index * 4 + 1,
                        triangle_index * 4 + 2,
                        triangle_index * 4 + 3,
                    ]);
                }
                tokens.sort_unstable();
                tokens.dedup();
            }
        }

        let mut split = None;
        for token in tokens.iter().copied().filter(|token| *token >= resume) {
            let (blade_index, blade_kind) = match hole {
                Some(_) => (token / 4, token % 4),
                None => (token, 0),
            };
            let blade = cutter.triangle(blade_index);
            let segment = match (blade_kind, hole) {
                (0, _) => triangle_plane(blade).and_then(|(normal, offset)| {
                    // The plane is unbounded but the blade is not, so the
                    // crossing has to land inside it.
                    triangle_plane_segment(&corners, normal, offset).and_then(|(start, end)| {
                        clip_segment_to_triangle(start, end, blade, normal)
                    })
                }),
                (edge, Some(axis)) => {
                    let start = edge - 1;
                    triangle_extruded_edge_segment(
                        &corners,
                        blade[start],
                        blade[(start + 1) % 3],
                        axis,
                    )
                }
                _ => None,
            };
            let Some((start, end)) = segment else {
                continue;
            };
            if let Some(pieces) = split_triangle_along_segment(&triangle, start, end) {
                split = Some((pieces, token + 1));
                break;
            }
        }

        match split {
            Some((pieces, next)) => {
                topology_changed = true;
                pending.extend(pieces.into_iter().map(|piece| (piece, next)));
            }
            None => done.push(triangle),
        }
        if done.len() + pending.len() > TRIANGLE_CEILING {
            return Err(format!(
                "Cut abandoned: it passed {TRIANGLE_CEILING} triangles. Simplify the cutter or \
                 cut against a smaller piece of terrain."
            ));
        }
    }

    // With the seam in place, every remaining triangle is wholly inside the
    // cutter's footprint or wholly outside it, never straddling. That is what
    // makes a centroid enough to decide, and why the hole comes out with a
    // clean edge instead of a staircase.
    if let Some(axis) = hole {
        let before = done.len();
        done.retain(|triangle| {
            let centroid: [f32; 3] = std::array::from_fn(|component| {
                (triangle[0].position[component]
                    + triangle[1].position[component]
                    + triangle[2].position[component])
                    / 3.0
            });
            // Covered means the cutter is somewhere along the axis, either
            // side. A ramp resting on a floor is above it; a ceiling is below.
            let covered = cutter.ray_hits(centroid, axis, COVER_REACH)
                || cutter.ray_hits(centroid, vec3_scale(axis, -1.0), COVER_REACH);
            !covered
        });
        topology_changed |= done.len() != before;
    }

    if !topology_changed {
        return Ok(false);
    }

    let mut positions = Vec::with_capacity(done.len() * 3);
    let mut normals = Vec::with_capacity(done.len() * 3);
    let mut tangents = primitive
        .tangents
        .is_empty()
        .then(Vec::new)
        .unwrap_or_else(|| Vec::with_capacity(done.len() * 3));
    let mut tex = vec![Vec::with_capacity(done.len() * 3); primitive.tex_coords.len()];
    let mut colors = vec![Vec::with_capacity(done.len() * 3); primitive.colors.len()];
    let mut indices = Vec::with_capacity(done.len() * 3);
    for triangle in &done {
        for corner in triangle {
            indices.push(positions.len() as u32);
            positions.push(corner.position);
            normals.push(corner.normal);
            if let Some(tangent) = corner.tangent {
                tangents.push(tangent);
            }
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
    primitive.tangents = tangents;
    for (slot, set) in primitive.tex_coords.iter_mut().zip(tex) {
        slot.values = set;
    }
    for (slot, set) in primitive.colors.iter_mut().zip(colors) {
        slot.values = set;
    }
    primitive.indices = indices;
    Ok(true)
}

/// Applies a hole cut to every collision surface without losing its material
/// metadata. Collision and render geometry are stored independently, so a
/// visible hole must be repeated here or the exported stage remains solid.
fn cut_collision_document(
    collision: &mut sms_authoring::CollisionDocument,
    cutter: &TriangleBvh,
    axis: [f32; 3],
) -> Result<(), String> {
    collision
        .validate()
        .map_err(|error| format!("Collision cut abandoned: {error}"))?;

    let mut vertices = Vec::new();
    let mut groups = Vec::with_capacity(collision.groups.len());
    for group in &collision.groups {
        let mut primitive = sms_authoring::ModelPrimitive {
            positions: collision.vertices.clone(),
            normals: vec![[0.0, 1.0, 0.0]; collision.vertices.len()],
            tangents: Vec::new(),
            tex_coords: Vec::new(),
            colors: Vec::new(),
            indices: group.triangles.iter().flatten().copied().collect(),
            material: None,
        };
        cut_primitive(&mut primitive, cutter, Some(axis))?;

        let base = u32::try_from(vertices.len())
            .map_err(|_| "Collision cut abandoned: vertex count exceeds u32.".to_string())?;
        let triangles = primitive
            .indices
            .chunks_exact(3)
            .map(|triangle| [triangle[0] + base, triangle[1] + base, triangle[2] + base])
            .collect();
        vertices.extend(primitive.positions);
        groups.push(sms_authoring::CollisionGroup {
            name: group.name.clone(),
            surface: group.surface.clone(),
            triangles,
        });
    }

    collision.vertices = vertices;
    collision.groups = groups;
    collision
        .cleanup_exact()
        .map_err(|error| format!("Collision cut abandoned: {error}"))?;
    Ok(())
}

impl SmsEditorApp {
    pub(super) fn boolean_cut_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Boolean Cut");
        ui.label(
            "Cuts the selected terrain where another mesh passes over or through it: the seam \
             becomes real edges, and the covered area is removed.",
        );
        ui.separator();

        let selected = self
            .selected_model_instance()
            .map(|instance| (instance.placement.instance_id, instance.placement.asset_id));
        let name_of = |editor: &Self, id: sms_authoring::AssetId| {
            editor
                .model_catalog_entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| id.to_string())
        };
        match selected {
            Some((_, id)) => ui.label(format!("Cutting: {}", name_of(self, id))),
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
            .filter(|(id, _)| selected.is_none_or(|(selected_id, _)| *id != selected_id))
            .collect::<Vec<_>>();
        let current = self
            .boolean_cutter
            .and_then(|id| {
                candidates
                    .iter()
                    .find(|(candidate_id, _)| *candidate_id == id)
                    .map(|(_, name)| name.clone())
            })
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
        ui.checkbox(&mut self.boolean_cut_hole, "Cut a hole")
            .on_hover_text("Remove the part of the terrain the cutter covers, not just the seam");
        ui.add_enabled_ui(self.boolean_cut_hole, |ui| {
            ui.horizontal(|ui| {
                ui.label("Along");
                for (index, label) in ["X", "Y", "Z"].into_iter().enumerate() {
                    ui.selectable_value(&mut self.boolean_cut_axis, index, label);
                }
            });
        });
        ui.label(
            "Covered means the cutter sits somewhere along that axis, either side of the \
             surface. Y suits a ramp or a building standing on a floor.",
        );

        ui.separator();
        if ui
            .add_enabled(
                selected.is_some() && self.boolean_cutter.is_some(),
                egui::Button::new("Cut"),
            )
            .on_hover_text("Split the selected terrain where the cutter meets it")
            .clicked()
        {
            self.cut_terrain_with_selected_cutter();
        }
        ui.label(
            "Not solid CSG. Stage terrain is open surfaces -- a ramp with no underside on a \
             floor with no thickness -- so there is no volume to subtract. The hole is the \
             covered footprint, which is the same result for terrain that sits on terrain.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sms_authoring::{CollisionDocument, CollisionGroup, CollisionSurface};

    fn flat_vertex(position: [f32; 3]) -> CutVertex {
        CutVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            tangent: None,
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

    #[test]
    fn a_finite_blade_does_not_extend_its_seam_to_the_target_edges() {
        let target = [
            flat_vertex([-100.0, 0.0, 0.0]),
            flat_vertex([100.0, 0.0, 0.0]),
            flat_vertex([100.0, 0.0, 100.0]),
        ];
        let blade = [[0.0, -10.0, 10.0], [0.0, 10.0, 10.0], [0.0, 0.0, 20.0]];
        let (normal, offset) = triangle_plane(&blade).expect("a plane");
        let target_positions = [target[0].position, target[1].position, target[2].position];
        let (start, end) =
            triangle_plane_segment(&target_positions, normal, offset).expect("a crossing");
        let (start, end) =
            clip_segment_to_triangle(start, end, &blade, normal).expect("finite overlap");
        let pieces = split_triangle_along_segment(&target, start, end).expect("a constrained seam");

        let seam_points = pieces
            .iter()
            .flatten()
            .filter(|vertex| vertex.position[0].abs() <= PLANE_EPSILON)
            .map(|vertex| vertex.position[2])
            .collect::<Vec<_>>();
        assert!(!seam_points.is_empty());
        assert!(
            seam_points
                .iter()
                .all(|z| (10.0 - PLANE_EPSILON..=20.0 + PLANE_EPSILON).contains(z)),
            "finite seam escaped the blade: {seam_points:?}"
        );
    }

    #[test]
    fn a_hole_cut_removes_covered_collision_and_preserves_its_surface() {
        let surface = CollisionSurface {
            surface_type: 7,
            attribute_0: 2,
            attribute_1: 3,
            data: Some(11),
        };
        let mut collision = CollisionDocument {
            vertices: vec![[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
            groups: vec![CollisionGroup {
                name: "floor".to_string(),
                surface: surface.clone(),
                triangles: vec![[0, 1, 2]],
            }],
        };
        let cutter = TriangleBvh::build(vec![[
            [-100.0, 1.0, -100.0],
            [100.0, 1.0, -100.0],
            [0.0, 1.0, 100.0],
        ]]);

        cut_collision_document(&mut collision, &cutter, [0.0, 1.0, 0.0]).expect("collision cut");

        assert!(collision.vertices.is_empty());
        assert!(collision.groups[0].triangles.is_empty());
        assert_eq!(collision.groups[0].surface, surface);
    }

    #[test]
    fn a_missed_cut_preserves_the_indexed_primitive_exactly() {
        let mut primitive = sms_authoring::ModelPrimitive {
            positions: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 0.0, 10.0],
                [0.0, 0.0, 10.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: Vec::new(),
            tex_coords: Vec::new(),
            colors: Vec::new(),
            indices: vec![0, 1, 2, 0, 2, 3],
            material: None,
        };
        let before = primitive.clone();
        let cutter = TriangleBvh::build(vec![[
            [100.0, -10.0, 100.0],
            [100.0, 10.0, 100.0],
            [100.0, 0.0, 110.0],
        ]]);

        assert!(!cut_primitive(&mut primitive, &cutter, None).expect("cut"));
        assert_eq!(primitive, before);
    }

    #[test]
    fn a_floating_partial_cutter_inserts_its_projected_hole_boundary() {
        let mut primitive = sms_authoring::ModelPrimitive {
            positions: vec![[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            tangents: Vec::new(),
            tex_coords: Vec::new(),
            colors: Vec::new(),
            indices: vec![0, 1, 2],
            material: None,
        };
        let cutter =
            TriangleBvh::build(vec![[[-2.0, 5.0, -2.0], [2.0, 5.0, -2.0], [0.0, 5.0, 2.0]]]);

        assert!(cut_primitive(&mut primitive, &cutter, Some([0.0, 1.0, 0.0])).expect("cut"));

        let remaining_area = primitive
            .indices
            .chunks_exact(3)
            .map(|face| {
                let a = primitive.positions[face[0] as usize];
                let b = primitive.positions[face[1] as usize];
                let c = primitive.positions[face[2] as usize];
                vec3_dot(vec3_cross(vec3_sub(b, a), vec3_sub(c, a)), [0.0, 1.0, 0.0]).abs() * 0.5
            })
            .sum::<f32>();
        // Original area is 200; the projected cutter area is 8.
        assert!((remaining_area - 192.0).abs() < 1e-3, "{remaining_area}");
    }

    #[test]
    fn collision_only_change_counts_even_when_render_triangle_count_is_equal() {
        let mut before = sms_authoring::ModelAssetDocument::new("terrain");
        before.collision = Some(CollisionDocument {
            vertices: vec![[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
            groups: vec![CollisionGroup {
                name: "floor".to_string(),
                surface: CollisionSurface::default(),
                triangles: vec![[0, 1, 2]],
            }],
        });
        let mut after = before.clone();
        after.collision.as_mut().expect("collision").groups[0]
            .triangles
            .clear();

        assert!(terrain_document_changed(&before, &after));
        assert_eq!(
            before
                .meshes
                .iter()
                .flat_map(|mesh| &mesh.primitives)
                .map(|primitive| primitive.indices.len() / 3)
                .sum::<usize>(),
            after
                .meshes
                .iter()
                .flat_map(|mesh| &mesh.primitives)
                .map(|primitive| primitive.indices.len() / 3)
                .sum::<usize>()
        );
    }

    #[test]
    fn repeated_asset_placements_remain_distinct_boolean_cutters() {
        let asset_id = AssetId::new();
        let mut first = sms_authoring::ModelInstancePlacement::new(asset_id, "terrain");
        first.export_mode = ModelInstanceExportMode::MapTerrain;
        let mut second = sms_authoring::ModelInstancePlacement::new(asset_id, "terrain");
        second.export_mode = ModelInstanceExportMode::MapTerrain;
        let expected = [first.instance_id, second.instance_id];
        let app = SmsEditorApp {
            stage_id: "stage".to_string(),
            model_instances: vec![
                EditorModelInstance {
                    stage_id: "stage".to_string(),
                    placement: first,
                    local_bounds: [[0.0; 3]; 2],
                },
                EditorModelInstance {
                    stage_id: "stage".to_string(),
                    placement: second,
                    local_bounds: [[0.0; 3]; 2],
                },
            ],
            ..SmsEditorApp::default()
        };

        let candidates = app.boolean_cut_candidates();

        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates
                .iter()
                .map(|(instance_id, _)| *instance_id)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(candidates[0].1.ends_with("(1/2)"));
        assert!(candidates[1].1.ends_with("(2/2)"));
    }
}

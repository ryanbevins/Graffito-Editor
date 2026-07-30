//! A bounding volume hierarchy over triangles.
//!
//! Both the occlusion bake and the boolean cut ask the same shape of question
//! -- what could this ray, or this triangle, possibly touch -- and both
//! answered it by walking every triangle in the stage. That is fine for a ramp
//! on a floor and hopeless for a real stage: occlusion alone is rays times
//! vertices times triangles.
//!
//! The tree wraps everything in one box, splits the triangles in half along
//! their longest axis, and repeats. A query that misses a box discards
//! everything beneath it at once, so the cost falls from every triangle to the
//! depth of the tree plus whatever survives to a leaf.

use crate::{vec3_cross, vec3_dot, vec3_sub};

/// Slack on the parallel-ray containment test, so a ray lying exactly in a box
/// face counts as touching it rather than falling either way on rounding.
const SLAB_EPSILON: f32 = 1e-4;

/// Triangles per leaf. Small enough that a leaf test is cheap, large enough
/// that the tree does not become mostly pointers.
const LEAF_SIZE: usize = 8;

struct BvhNode {
    min: [f32; 3],
    max: [f32; 3],
    /// Leaf only: where its triangles start in `order`.
    start: u32,
    /// Zero marks an interior node.
    count: u32,
    /// Interior only: the right child. The left child always follows its
    /// parent, so it needs no index.
    right: u32,
}

pub(super) struct TriangleBvh {
    triangles: Vec<[[f32; 3]; 3]>,
    /// Per-triangle bounds, kept so an overlap query can answer for the
    /// triangle itself rather than for whichever leaf it landed in.
    boxes: Vec<([f32; 3], [f32; 3])>,
    /// Triangle indices, permuted so every leaf owns a contiguous run.
    order: Vec<u32>,
    nodes: Vec<BvhNode>,
}

impl TriangleBvh {
    pub(super) fn build(triangles: Vec<[[f32; 3]; 3]>) -> Self {
        let boxes = triangles
            .iter()
            .map(|triangle| {
                let min = std::array::from_fn(|axis| {
                    triangle[0][axis]
                        .min(triangle[1][axis])
                        .min(triangle[2][axis])
                });
                let max = std::array::from_fn(|axis| {
                    triangle[0][axis]
                        .max(triangle[1][axis])
                        .max(triangle[2][axis])
                });
                (min, max)
            })
            .collect::<Vec<_>>();
        let centroids = boxes
            .iter()
            .map(|(min, max)| std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5))
            .collect::<Vec<_>>();

        let mut order = (0..triangles.len() as u32).collect::<Vec<_>>();
        let mut nodes = Vec::new();
        if !order.is_empty() {
            build_range(&mut nodes, &mut order, &boxes, &centroids, 0);
        }
        Self {
            triangles,
            boxes,
            order,
            nodes,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.triangles.len()
    }

    pub(super) fn triangle(&self, index: usize) -> &[[f32; 3]; 3] {
        &self.triangles[index]
    }

    /// Whether anything blocks a ray within `reach`. Stops at the first hit.
    pub(super) fn ray_hits(&self, origin: [f32; 3], direction: [f32; 3], reach: f32) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if !slab_hit(node.min, node.max, origin, direction, reach) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right as usize);
                stack.push(index + 1);
                continue;
            }
            for slot in node.start..node.start + node.count {
                let triangle = &self.triangles[self.order[slot as usize] as usize];
                if ray_triangle_hit_distance(origin, direction, triangle, reach).is_some() {
                    return true;
                }
            }
        }
        false
    }

    /// Distance to the nearest triangle along a ray within `reach`.
    pub(super) fn nearest_ray_hit(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        reach: f32,
    ) -> Option<f32> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut nearest = reach;
        let mut found = false;
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if !slab_hit(node.min, node.max, origin, direction, nearest) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right as usize);
                stack.push(index + 1);
                continue;
            }
            for slot in node.start..node.start + node.count {
                let triangle = &self.triangles[self.order[slot as usize] as usize];
                if let Some(distance) =
                    ray_triangle_hit_distance(origin, direction, triangle, nearest)
                {
                    nearest = distance;
                    found = true;
                }
            }
        }
        found.then_some(nearest)
    }

    /// Every triangle whose box overlaps the given one.
    pub(super) fn overlapping(&self, min: [f32; 3], max: [f32; 3], found: &mut Vec<u32>) {
        found.clear();
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if (0..3).any(|axis| min[axis] > node.max[axis] || max[axis] < node.min[axis]) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right as usize);
                stack.push(index + 1);
                continue;
            }
            // A leaf's box is the union of its triangles, so reaching one
            // says only that something in it might overlap. Test each.
            found.extend(
                self.order[node.start as usize..(node.start + node.count) as usize]
                    .iter()
                    .copied()
                    .filter(|triangle| {
                        let (low, high) = self.boxes[*triangle as usize];
                        (0..3).all(|axis| low[axis] <= max[axis] && high[axis] >= min[axis])
                    }),
            );
        }
        // Callers walk blades in index order so a split never revisits one it
        // has already passed.
        found.sort_unstable();
    }

    /// Every triangle whose bounds overlap the query after both are projected
    /// along `axis`.
    ///
    /// Boolean-hole cutters may sit far above the target while their projected
    /// footprints still overlap it. Two perpendicular projected intervals
    /// provide a BVH-accelerated broad phase for that extrusion.
    pub(super) fn overlapping_projection(
        &self,
        min: [f32; 3],
        max: [f32; 3],
        axis: [f32; 3],
        found: &mut Vec<u32>,
    ) {
        found.clear();
        if self.nodes.is_empty() {
            return;
        }
        let Some((first, second)) = projection_basis(axis) else {
            return;
        };
        let query = [project_box(min, max, first), project_box(min, max, second)];
        let overlaps = |low: [f32; 3], high: [f32; 3]| {
            [first, second]
                .into_iter()
                .zip(query)
                .all(|(direction, query)| {
                    let projected = project_box(low, high, direction);
                    projected.0 <= query.1 && projected.1 >= query.0
                })
        };

        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index];
            if !overlaps(node.min, node.max) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right as usize);
                stack.push(index + 1);
                continue;
            }
            found.extend(
                self.order[node.start as usize..(node.start + node.count) as usize]
                    .iter()
                    .copied()
                    .filter(|triangle| {
                        let (low, high) = self.boxes[*triangle as usize];
                        overlaps(low, high)
                    }),
            );
        }
        found.sort_unstable();
    }
}

fn projection_basis(axis: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
    let length = vec3_dot(axis, axis).sqrt();
    if length <= f32::EPSILON {
        return None;
    }
    let axis = axis.map(|component| component / length);
    let reference = if axis[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let first = vec3_cross(reference, axis);
    let first_length = vec3_dot(first, first).sqrt();
    if first_length <= f32::EPSILON {
        return None;
    }
    let first = first.map(|component| component / first_length);
    Some((first, vec3_cross(axis, first)))
}

fn project_box(min: [f32; 3], max: [f32; 3], direction: [f32; 3]) -> (f32, f32) {
    let center = std::array::from_fn::<_, 3, _>(|axis| (min[axis] + max[axis]) * 0.5);
    let extent = std::array::from_fn::<_, 3, _>(|axis| (max[axis] - min[axis]) * 0.5);
    let middle = vec3_dot(center, direction);
    let radius = (0..3)
        .map(|axis| extent[axis] * direction[axis].abs())
        .sum::<f32>();
    (middle - radius, middle + radius)
}

fn build_range(
    nodes: &mut Vec<BvhNode>,
    order: &mut [u32],
    boxes: &[([f32; 3], [f32; 3])],
    centroids: &[[f32; 3]],
    start: usize,
) -> u32 {
    let index = nodes.len() as u32;
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for id in order.iter() {
        let (low, high) = boxes[*id as usize];
        for axis in 0..3 {
            min[axis] = min[axis].min(low[axis]);
            max[axis] = max[axis].max(high[axis]);
        }
    }
    nodes.push(BvhNode {
        min,
        max,
        start: start as u32,
        count: order.len() as u32,
        right: 0,
    });
    if order.len() <= LEAF_SIZE {
        return index;
    }

    // Split down the middle of the longest axis. A surface area heuristic
    // builds better trees, but this is already the difference between linear
    // and logarithmic, and it is far simpler to be sure of.
    let axis = (0..3)
        .max_by(|a, b| {
            (max[*a] - min[*a])
                .partial_cmp(&(max[*b] - min[*b]))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0);
    order.sort_unstable_by(|a, b| {
        centroids[*a as usize][axis]
            .partial_cmp(&centroids[*b as usize][axis])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let half = order.len() / 2;
    let (left, right) = order.split_at_mut(half);
    nodes[index as usize].count = 0;
    build_range(nodes, left, boxes, centroids, start);
    let right_index = build_range(nodes, right, boxes, centroids, start + half);
    nodes[index as usize].right = right_index;
    index
}

/// Slab test: the ray enters all three pairs of planes before leaving any.
fn slab_hit(
    min: [f32; 3],
    max: [f32; 3],
    origin: [f32; 3],
    direction: [f32; 3],
    reach: f32,
) -> bool {
    let mut near = 0.0f32;
    let mut far = reach;
    for axis in 0..3 {
        // A ray parallel to this axis never crosses its slabs, so the only
        // question is whether it already lies between them. Taking the usual
        // reciprocal here would compute 0 * infinity for an origin sitting
        // exactly on a face, and the NaN would throw the whole test away --
        // which is exactly what happens on grid-aligned terrain, where box
        // faces land on round numbers.
        if direction[axis].abs() <= 1e-12 {
            if origin[axis] < min[axis] - SLAB_EPSILON || origin[axis] > max[axis] + SLAB_EPSILON {
                return false;
            }
            continue;
        }
        let inverse = 1.0 / direction[axis];
        let low = (min[axis] - origin[axis]) * inverse;
        let high = (max[axis] - origin[axis]) * inverse;
        near = near.max(low.min(high));
        far = far.min(low.max(high));
        if near > far {
            return false;
        }
    }
    true
}

/// Any-hit ray/triangle test, Moller-Trumbore. Stops at the first blocker
/// rather than finding the nearest, which is all occlusion needs.
#[cfg(test)]
pub(super) fn ray_hits_triangle(
    origin: [f32; 3],
    direction: [f32; 3],
    triangle: &[[f32; 3]; 3],
    reach: f32,
) -> bool {
    ray_triangle_hit_distance(origin, direction, triangle, reach).is_some()
}

fn ray_triangle_hit_distance(
    origin: [f32; 3],
    direction: [f32; 3],
    triangle: &[[f32; 3]; 3],
    reach: f32,
) -> Option<f32> {
    let edge_a = vec3_sub(triangle[1], triangle[0]);
    let edge_b = vec3_sub(triangle[2], triangle[0]);
    let pvec = vec3_cross(direction, edge_b);
    let determinant = vec3_dot(edge_a, pvec);
    if determinant.abs() < 1e-8 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let tvec = vec3_sub(origin, triangle[0]);
    let u = vec3_dot(tvec, pvec) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qvec = vec3_cross(tvec, edge_a);
    let v = vec3_dot(direction, qvec) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = vec3_dot(edge_b, qvec) * inverse;
    (distance > 1e-3 && distance < reach).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of triangles, enough of them to force several levels of tree.
    fn floor(side: usize) -> Vec<[[f32; 3]; 3]> {
        let mut triangles = Vec::new();
        for z in 0..side {
            for x in 0..side {
                let (x0, z0) = (x as f32 * 10.0, z as f32 * 10.0);
                let (x1, z1) = (x0 + 10.0, z0 + 10.0);
                triangles.push([[x0, 0.0, z0], [x1, 0.0, z0], [x1, 0.0, z1]]);
                triangles.push([[x0, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1]]);
            }
        }
        triangles
    }

    #[test]
    fn the_tree_agrees_with_testing_every_triangle() {
        // The tree is only worth having if it never changes an answer, so the
        // test is against the brute force it replaces rather than against
        // hand-written expectations.
        let triangles = floor(12);
        let bvh = TriangleBvh::build(triangles.clone());
        assert_eq!(bvh.len(), triangles.len());

        for step in 0..40 {
            let x = step as f32 * 7.0 - 40.0;
            let origin = [x, 50.0, x * 0.5 + 10.0];
            for direction in [
                [0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.3, -1.0, 0.2],
                [1.0, 0.0, 0.0],
            ] {
                let reach = 500.0;
                let brute = triangles
                    .iter()
                    .any(|triangle| ray_hits_triangle(origin, direction, triangle, reach));
                assert_eq!(
                    bvh.ray_hits(origin, direction, reach),
                    brute,
                    "origin {origin:?} direction {direction:?}"
                );
            }
        }
    }

    #[test]
    fn a_short_reach_stops_the_ray_before_the_floor() {
        let bvh = TriangleBvh::build(floor(4));
        let origin = [15.0, 100.0, 15.0];
        assert!(bvh.ray_hits(origin, [0.0, -1.0, 0.0], 500.0));
        assert!(!bvh.ray_hits(origin, [0.0, -1.0, 0.0], 50.0));
        assert_eq!(
            bvh.nearest_ray_hit(origin, [0.0, -1.0, 0.0], 500.0),
            Some(100.0)
        );
    }

    #[test]
    fn nearest_ray_hit_returns_the_frontmost_stacked_surface() {
        let bvh = TriangleBvh::build(vec![
            [[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [0.0, 0.0, 10.0]],
            [[-10.0, 20.0, -10.0], [10.0, 20.0, -10.0], [0.0, 20.0, 10.0]],
        ]);

        assert_eq!(
            bvh.nearest_ray_hit([0.0, 50.0, 0.0], [0.0, -1.0, 0.0], 100.0),
            Some(30.0)
        );
    }

    #[test]
    fn an_overlap_query_finds_exactly_the_boxes_that_meet() {
        let triangles = floor(10);
        let bvh = TriangleBvh::build(triangles.clone());
        let (min, max) = ([12.0, -1.0, 12.0], [28.0, 1.0, 28.0]);
        let mut found = Vec::new();
        bvh.overlapping(min, max, &mut found);

        let expected = triangles
            .iter()
            .enumerate()
            .filter(|(_, triangle)| {
                (0..3).all(|axis| {
                    let low = triangle[0][axis]
                        .min(triangle[1][axis])
                        .min(triangle[2][axis]);
                    let high = triangle[0][axis]
                        .max(triangle[1][axis])
                        .max(triangle[2][axis]);
                    low <= max[axis] && high >= min[axis]
                })
            })
            .count();
        assert_eq!(found.len(), expected);
        // Callers rely on the order to avoid revisiting a blade they passed.
        assert!(found.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn projected_overlap_ignores_distance_along_the_extrusion_axis() {
        let bvh = TriangleBvh::build(vec![
            [[0.0, 1000.0, 0.0], [4.0, 1000.0, 0.0], [0.0, 1000.0, 4.0]],
            [
                [20.0, 1000.0, 0.0],
                [24.0, 1000.0, 0.0],
                [20.0, 1000.0, 4.0],
            ],
        ]);
        let mut found = Vec::new();

        bvh.overlapping_projection(
            [-1.0, 0.0, -1.0],
            [5.0, 0.0, 5.0],
            [0.0, 1.0, 0.0],
            &mut found,
        );

        assert_eq!(found, vec![0]);
    }

    #[test]
    fn an_empty_tree_answers_without_panicking() {
        let bvh = TriangleBvh::build(Vec::new());
        assert_eq!(bvh.len(), 0);
        assert!(!bvh.ray_hits([0.0; 3], [0.0, -1.0, 0.0], 100.0));
        let mut found = vec![7u32];
        bvh.overlapping([0.0; 3], [1.0; 3], &mut found);
        assert!(found.is_empty());
    }
}

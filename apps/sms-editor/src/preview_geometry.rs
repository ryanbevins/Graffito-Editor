use super::*;

pub(super) fn transform_preview_vertices(
    vertices: [[f32; 3]; 3],
    transform: Transform,
) -> [[f32; 3]; 3] {
    vertices.map(|vertex| transform_preview_point(vertex, transform))
}

pub(super) fn transform_j3d_matrix_point(matrix: J3dMatrix34, point: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * point[0] + matrix[0][1] * point[1] + matrix[0][2] * point[2] + matrix[0][3],
        matrix[1][0] * point[0] + matrix[1][1] * point[1] + matrix[1][2] * point[2] + matrix[1][3],
        matrix[2][0] * point[0] + matrix[2][1] * point[1] + matrix[2][2] * point[2] + matrix[2][3],
    ]
}

pub(super) fn transform_j3d_matrix_normal(matrix: J3dMatrix34, normal: [f32; 3]) -> [f32; 3] {
    let [a, b, c] = [matrix[0][0], matrix[0][1], matrix[0][2]];
    let [d, e, f] = [matrix[1][0], matrix[1][1], matrix[1][2]];
    let [g, h, i] = [matrix[2][0], matrix[2][1], matrix[2][2]];
    vec3_normalize([
        (e * i - f * h) * normal[0] + (f * g - d * i) * normal[1] + (d * h - e * g) * normal[2],
        (c * h - b * i) * normal[0] + (a * i - c * g) * normal[1] + (b * g - a * h) * normal[2],
        (b * f - c * e) * normal[0] + (c * d - a * f) * normal[1] + (a * e - b * d) * normal[2],
    ])
}

pub(super) fn transform_preview_normals(
    normals: [[f32; 3]; 3],
    transform: Transform,
) -> [[f32; 3]; 3] {
    normals.map(|normal| transform_preview_normal(normal, transform))
}

pub(super) fn transform_preview_normal(mut normal: [f32; 3], transform: Transform) -> [f32; 3] {
    for (component, scale) in normal.iter_mut().zip(transform.scale) {
        if scale.abs() > 0.00001 {
            *component /= scale;
        }
    }
    normal = rotate_x_degrees(normal, transform.rotation_degrees[0]);
    normal = rotate_y_degrees(normal, transform.rotation_degrees[1]);
    normal = rotate_z_degrees(normal, transform.rotation_degrees[2]);
    vec3_normalize(normal)
}

pub(super) fn transform_preview_point(mut point: [f32; 3], transform: Transform) -> [f32; 3] {
    point[0] *= transform.scale[0];
    point[1] *= transform.scale[1];
    point[2] *= transform.scale[2];

    point = rotate_x_degrees(point, transform.rotation_degrees[0]);
    point = rotate_y_degrees(point, transform.rotation_degrees[1]);
    point = rotate_z_degrees(point, transform.rotation_degrees[2]);

    [
        point[0] + transform.translation[0],
        point[1] + transform.translation[1],
        point[2] + transform.translation[2],
    ]
}

pub(super) fn retransform_preview_point(
    point: [f32; 3],
    old_transform: Transform,
    new_transform: Transform,
) -> [f32; 3] {
    transform_preview_point(
        inverse_transform_preview_point(point, old_transform),
        new_transform,
    )
}

pub(super) fn retransform_preview_normal(
    normal: [f32; 3],
    old_transform: Transform,
    new_transform: Transform,
) -> [f32; 3] {
    transform_preview_normal(
        inverse_transform_preview_normal(normal, old_transform),
        new_transform,
    )
}

pub(super) fn inverse_transform_preview_normal(
    mut normal: [f32; 3],
    transform: Transform,
) -> [f32; 3] {
    normal = rotate_z_degrees(normal, -transform.rotation_degrees[2]);
    normal = rotate_y_degrees(normal, -transform.rotation_degrees[1]);
    normal = rotate_x_degrees(normal, -transform.rotation_degrees[0]);
    for (component, scale) in normal.iter_mut().zip(transform.scale) {
        *component *= scale;
    }
    vec3_normalize(normal)
}

pub(super) fn inverse_transform_preview_point(
    mut point: [f32; 3],
    transform: Transform,
) -> [f32; 3] {
    point[0] -= transform.translation[0];
    point[1] -= transform.translation[1];
    point[2] -= transform.translation[2];

    point = rotate_z_degrees(point, -transform.rotation_degrees[2]);
    point = rotate_y_degrees(point, -transform.rotation_degrees[1]);
    point = rotate_x_degrees(point, -transform.rotation_degrees[0]);

    [
        point[0] / transform.scale[0],
        point[1] / transform.scale[1],
        point[2] / transform.scale[2],
    ]
}

pub(super) fn transform_has_invertible_scale(transform: Transform) -> bool {
    transform
        .scale
        .iter()
        .all(|value| value.is_finite() && value.abs() > 0.00001)
}

pub(super) fn rotate_x_degrees(point: [f32; 3], degrees: f32) -> [f32; 3] {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    [
        point[0],
        point[1] * cos - point[2] * sin,
        point[1] * sin + point[2] * cos,
    ]
}

pub(super) fn rotate_y_degrees(point: [f32; 3], degrees: f32) -> [f32; 3] {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    [
        point[0] * cos + point[2] * sin,
        point[1],
        -point[0] * sin + point[2] * cos,
    ]
}

pub(super) fn rotate_z_degrees(point: [f32; 3], degrees: f32) -> [f32; 3] {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    [
        point[0] * cos - point[1] * sin,
        point[0] * sin + point[1] * cos,
        point[2],
    ]
}

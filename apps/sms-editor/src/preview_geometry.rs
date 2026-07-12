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

pub(super) fn transform_j3d_billboard(
    billboard: J3dBillboard,
    transform: Transform,
    transformed_normals: Option<[[f32; 3]; 3]>,
) -> Option<J3dBillboard> {
    rebase_j3d_billboard(
        billboard,
        transform_preview_point(billboard.center, transform),
        billboard
            .axes
            .map(|axis| transform_preview_vector(axis, transform)),
        transformed_normals,
    )
}

pub(super) fn retransform_j3d_billboard(
    billboard: J3dBillboard,
    old_transform: Transform,
    new_transform: Transform,
    transformed_normals: Option<[[f32; 3]; 3]>,
) -> Option<J3dBillboard> {
    rebase_j3d_billboard(
        billboard,
        retransform_preview_point(billboard.center, old_transform, new_transform),
        billboard.axes.map(|axis| {
            transform_preview_vector(
                inverse_transform_preview_vector(axis, old_transform),
                new_transform,
            )
        }),
        transformed_normals,
    )
}

pub(super) fn transform_j3d_billboard_matrix(
    billboard: J3dBillboard,
    matrix: J3dMatrix34,
    transformed_normals: Option<[[f32; 3]; 3]>,
) -> Option<J3dBillboard> {
    rebase_j3d_billboard(
        billboard,
        transform_j3d_matrix_point(matrix, billboard.center),
        billboard
            .axes
            .map(|axis| transform_j3d_matrix_vector(matrix, axis)),
        transformed_normals,
    )
}

pub(super) fn j3d_billboard_world_vertices(
    billboard: J3dBillboard,
    camera: CameraFrame,
) -> [[f32; 3]; 3] {
    let axes = match billboard.mode {
        sms_formats::J3dBillboardMode::Full => [
            camera.right,
            camera.up,
            camera.forward.map(|component| -component),
        ],
        sms_formats::J3dBillboardMode::YAxis => {
            let axis_y = vec3_normalize(billboard.axes[1]);
            let axis_z =
                vec3_normalize(vec3_cross(camera.right, axis_y)).map(|component| -component);
            [camera.right, axis_y, axis_z]
        }
    };
    billboard.offsets.map(|offset| {
        let mut vertex = billboard.center;
        for axis in 0..3 {
            for component in 0..3 {
                vertex[component] += axes[axis][component] * offset[axis];
            }
        }
        vertex
    })
}

fn rebase_j3d_billboard(
    mut billboard: J3dBillboard,
    center: [f32; 3],
    axis_vectors: [[f32; 3]; 3],
    transformed_normals: Option<[[f32; 3]; 3]>,
) -> Option<J3dBillboard> {
    let axis_lengths = axis_vectors.map(vec3_length);
    if axis_lengths
        .iter()
        .any(|length| !length.is_finite() || *length <= 0.00001)
    {
        return None;
    }
    billboard.center = center;
    billboard.axes = std::array::from_fn(|index| {
        let length = axis_lengths[index];
        axis_vectors[index].map(|component| component / length)
    });
    billboard.offsets = billboard
        .offsets
        .map(|offset| std::array::from_fn(|index| offset[index] * axis_lengths[index]));
    billboard.normals = transformed_normals
        .map(|normals| normals.map(|normal| billboard.axes.map(|axis| vec3_dot(normal, axis))));
    Some(billboard)
}

pub(super) fn transform_preview_vector(mut vector: [f32; 3], transform: Transform) -> [f32; 3] {
    vector[0] *= transform.scale[0];
    vector[1] *= transform.scale[1];
    vector[2] *= transform.scale[2];
    vector = rotate_x_degrees(vector, transform.rotation_degrees[0]);
    vector = rotate_y_degrees(vector, transform.rotation_degrees[1]);
    rotate_z_degrees(vector, transform.rotation_degrees[2])
}

fn inverse_transform_preview_vector(mut vector: [f32; 3], transform: Transform) -> [f32; 3] {
    vector = rotate_z_degrees(vector, -transform.rotation_degrees[2]);
    vector = rotate_y_degrees(vector, -transform.rotation_degrees[1]);
    vector = rotate_x_degrees(vector, -transform.rotation_degrees[0]);
    [
        vector[0] / transform.scale[0],
        vector[1] / transform.scale[1],
        vector[2] / transform.scale[2],
    ]
}

pub(super) fn transform_j3d_matrix_vector(matrix: J3dMatrix34, vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
}

fn vec3_length(vector: [f32; 3]) -> f32 {
    vec3_dot(vector, vector).sqrt()
}

fn vec3_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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

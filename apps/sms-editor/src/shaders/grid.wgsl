struct Camera {
    camera_position: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    projection: vec4<f32>,
    clip: vec4<f32>,
    light_position: vec4<f32>,
    light_color: vec4<f32>,
    ambient_color: vec4<f32>,
    object_light_position: vec4<f32>,
    object_light_color: vec4<f32>,
    object_ambient_color: vec4<f32>,
    lighting_meta: vec4<f32>,
    render_target_size: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

fn grid_vertex(vertex_index: u32) -> VertexOut {
    var world_position = vec3<f32>(0.0);
    var color = vec4<f32>(178.0 / 255.0, 186.0 / 255.0, 178.0 / 255.0, 32.0 / 255.0);

    if (vertex_index < 84u) {
        let line_index = vertex_index / 2u;
        let endpoint = select(-5000.0, 5000.0, (vertex_index & 1u) != 0u);
        let grid_index = i32(line_index % 21u) - 10;
        let grid_offset = f32(grid_index) * 500.0;
        if ((grid_index % 5) == 0) {
            color = vec4<f32>(213.0 / 255.0, 200.0 / 255.0, 160.0 / 255.0, 58.0 / 255.0);
        }
        if (line_index < 21u) {
            world_position = vec3<f32>(grid_offset, 0.0, endpoint);
        } else {
            world_position = vec3<f32>(endpoint, 0.0, grid_offset);
        }
    } else {
        let axis_index = (vertex_index - 84u) / 2u;
        let endpoint = select(-5200.0, 5200.0, (vertex_index & 1u) != 0u);
        if (axis_index == 0u) {
            world_position = vec3<f32>(endpoint, 0.0, 0.0);
            color = vec4<f32>(206.0 / 255.0, 82.0 / 255.0, 82.0 / 255.0, 1.0);
        } else {
            world_position = vec3<f32>(0.0, 0.0, endpoint);
            color = vec4<f32>(82.0 / 255.0, 168.0 / 255.0, 110.0 / 255.0, 1.0);
        }
    }

    let relative = world_position - camera.camera_position.xyz;
    let view_position = vec3<f32>(
        dot(relative, camera.right.xyz),
        dot(relative, camera.up.xyz),
        dot(relative, camera.forward.xyz),
    );
    let depth = view_position.z;
    let clip_x = view_position.x * camera.projection.x + camera.projection.z * depth;
    let clip_y = view_position.y * camera.projection.y + camera.projection.w * depth;

    var out: VertexOut;
    out.position = vec4<f32>(clip_x, clip_y, depth - camera.clip.x, depth);
    out.color = color;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    return grid_vertex(vertex_index);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return input.color;
}

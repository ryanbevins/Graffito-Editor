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

@group(0) @binding(0)
var<uniform> camera: Camera;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, -1.0),
    );
    let corner = corners[vertex_index] * camera.clip.w;
    let world_position = vec3<f32>(corner.x, camera.clip.y, corner.y);
    let relative = world_position - camera.camera_position.xyz;
    let view_position = vec3<f32>(
        dot(relative, camera.right.xyz),
        dot(relative, camera.up.xyz),
        dot(relative, camera.forward.xyz),
    );
    let depth = view_position.z;
    let clip_x = view_position.x * camera.projection.x + camera.projection.z * depth;
    let clip_y = view_position.y * camera.projection.y + camera.projection.w * depth;
    return vec4<f32>(clip_x, clip_y, depth - camera.clip.x, depth);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(235.0 / 255.0, 42.0 / 255.0, 48.0 / 255.0, 0.5);
}

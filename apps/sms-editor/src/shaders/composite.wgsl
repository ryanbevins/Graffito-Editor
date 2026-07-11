@group(0) @binding(0)
var viewport_texture: texture_2d<f32>;

@group(0) @binding(1)
var viewport_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );
    var out: VertexOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}

fn srgb_to_linear_channel(value: f32) -> f32 {
    if (value <= 0.04045) {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn srgb_to_linear(value: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear_channel(value.r),
        srgb_to_linear_channel(value.g),
        srgb_to_linear_channel(value.b),
    );
}

@fragment
fn fs_srgb_target(input: VertexOut) -> @location(0) vec4<f32> {
    let gx_color = textureSample(viewport_texture, viewport_sampler, input.uv);
    // The surface will apply the inverse sRGB transfer. Decode here so the
    // numeric GX framebuffer value survives presentation unchanged.
    return vec4<f32>(srgb_to_linear(gx_color.rgb), 1.0);
}

@fragment
fn fs_unorm_target(input: VertexOut) -> @location(0) vec4<f32> {
    let gx_color = textureSample(viewport_texture, viewport_sampler, input.uv);
    return vec4<f32>(gx_color.rgb, 1.0);
}

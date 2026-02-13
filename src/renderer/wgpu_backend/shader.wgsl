// Rectangle instancing shader for wgpu backend.
// Each instance provides pixel-space rect (x, y, w, h) and RGBA color.
// Uniform provides screen_size for NDC conversion.

struct Uniforms {
    screen_size: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Instance attributes
struct InstanceInput {
    @location(0) rect: vec4<f32>,   // x, y, width, height (pixels)
    @location(1) color: vec4<f32>,  // r, g, b, a (0.0-1.0)
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: InstanceInput,
) -> VertexOutput {
    // Unit quad: 0=(0,0), 1=(1,0), 2=(0,1), 3=(1,1)
    let uv = vec2<f32>(
        f32(vertex_index & 1u),
        f32((vertex_index >> 1u) & 1u),
    );

    // Pixel position
    let px = instance.rect.xy + uv * instance.rect.zw;

    // Convert to NDC: x: [0, width] -> [-1, 1], y: [0, height] -> [1, -1]
    let ndc = vec2<f32>(
        (px.x / uniforms.screen_size.x) * 2.0 - 1.0,
        1.0 - (px.y / uniforms.screen_size.y) * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}

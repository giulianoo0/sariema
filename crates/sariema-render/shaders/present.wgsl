// SPDX-License-Identifier: Apache-2.0
// Placeholder output step until the post-processing chain: exposure and the
// Hejl-Burgess-Dawson curve, nothing else.

struct Params {
    exposure: f32,
    // 1 when the target is an sRGB format that encodes on write.
    srgb_target: u32,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let hdr = textureLoad(scene, vec2<i32>(pixel.xy), 0).rgb;
    let x = max(hdr * params.exposure - 0.004, vec3(0.0));
    // The curve's output is already display-encoded.
    var c = x * (6.2 * x + 0.5) / (x * (6.2 * x + 1.7) + 0.06);
    if params.srgb_target != 0u {
        c = select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
    }
    return vec4(c, 1.0);
}

// SPDX-License-Identifier: Apache-2.0
// The frame held, over the frame just composited, at an opacity. Appended
// to fullscreen.wgsl.

struct Params {
    opacity: f32,
    // WebGL2 binds uniforms in whole 16-byte rows.
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var held: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(textureLoad(held, vec2<i32>(pixel.xy), 0).rgb, params.opacity);
}

// SPDX-License-Identifier: Apache-2.0
// The next level of the chain at half the size, one bilinear sample at the
// shared corner of each 2×2 block.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(max(textureDimensions(source) / 2u, vec2(1u)));
    return textureSampleLevel(source, bilinear, pixel.xy / size, 0.0);
}

// SPDX-License-Identifier: Apache-2.0
// The next level of the chain at half the size, one bilinear sample at the
// shared corner of each 2×2 block. `fs_bright` keeps only the light past the
// bloom threshold.

struct Bright {
    tint: vec3<f32>,
    threshold: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<uniform> bright: Bright;

fn block(pixel: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(max(textureDimensions(source) / 2u, vec2(1u)));
    return textureSampleLevel(source, bilinear, pixel / size, 0.0);
}

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    return block(pixel.xy);
}

@fragment
fn fs_bright(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let c = block(pixel.xy).rgb;
    return vec4(max(c - bright.threshold, vec3(0.0)) * bright.tint, 0.0);
}

// SPDX-License-Identifier: Apache-2.0
// One direction of a separable blur at the source's size. Every tap after
// the centre is taken on both sides, between two texels, so one bilinear
// sample weighs both.

struct Tap {
    // In texels from the centre.
    offset: f32,
    weight: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<storage, read> taps: array<Tap>;

override VERTICAL: bool = false;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let uv = pixel.xy / size;
    let step = select(vec2(1.0 / size.x, 0.0), vec2(0.0, 1.0 / size.y), VERTICAL);
    var sum = textureSampleLevel(source, bilinear, uv, 0.0) * taps[0].weight;
    for (var i = 1u; i < arrayLength(&taps); i++) {
        let along = step * taps[i].offset;
        let pair = textureSampleLevel(source, bilinear, uv - along, 0.0)
            + textureSampleLevel(source, bilinear, uv + along, 0.0);
        sum += pair * taps[i].weight;
    }
    return sum;
}

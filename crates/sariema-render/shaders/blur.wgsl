// SPDX-License-Identifier: Apache-2.0
// One direction of a separable blur at the source's size. Every tap after
// the centre is taken on both sides, between two texels, so one bilinear
// sample weighs both.

struct Tap {
    // In texels from the centre.
    offset: f32,
    weight: f32,
}

struct Bright {
    tint: vec3<f32>,
    threshold: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<storage, read> taps: array<Tap>;
@group(0) @binding(3) var<uniform> bright: Bright;

override VERTICAL: bool = false;
// Keeps only the light past the bloom threshold, tinted, before blurring.
override BRIGHT: bool = false;

fn tap(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(source, bilinear, uv, 0.0);
    if BRIGHT {
        return vec4(max(c.rgb - bright.threshold, vec3(0.0)) * bright.tint, 0.0);
    }
    return c;
}

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let uv = pixel.xy / size;
    let step = select(vec2(1.0 / size.x, 0.0), vec2(0.0, 1.0 / size.y), VERTICAL);
    var sum = tap(uv) * taps[0].weight;
    for (var i = 1u; i < arrayLength(&taps); i++) {
        let along = step * taps[i].offset;
        sum += (tap(uv - along) + tap(uv + along)) * taps[i].weight;
    }
    return sum;
}

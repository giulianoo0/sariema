// SPDX-License-Identifier: Apache-2.0
// A separable blur at the source's size, in two passes, one per axis, or in
// one. Every tap after the centre is taken on both sides, between two
// texels, so one bilinear sample weighs both; and since bilinear filtering
// is separable, one sample at a horizontal and a vertical tap weighs in one
// pass what the two passes would give it. post.rs prepends `TAPS`, the
// centre then each tap after it as (offset in texels, weight), and `TAP_COUNT`.

struct Bright {
    tint: vec3<f32>,
    threshold: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<uniform> bright: Bright;

// 0: both axes in one pass; 1: horizontal; 2: vertical.
override AXES: u32 = 0u;
// Keeps only the light past the bloom threshold, tinted, before blurring.
// Horizontal passes only, so that it applies to samples filtered along rows.
override BRIGHT: bool = false;
// Reads the level above, twice the size, reduced as downsample.wgsl reduces
// it, so that one pass both reduces and blurs. One-pass blurs only.
override REDUCE: bool = false;

fn tap(uv: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(source, bilinear, uv, 0.0);
    if BRIGHT {
        return vec4(max(c.rgb - bright.threshold, vec3(0.0)) * bright.tint, 0.0);
    }
    return c;
}

// The level above reduced, at texel `t` of the level drawn, of `size`.
fn reduced(t: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(source, bilinear, (clamp(t, vec2(0.0), size - 1.0) + 0.5) / size, 0.0);
}

// The level drawn, of `size`, filtered bilinearly at `p` in its texels.
fn level_at(p: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    if !REDUCE {
        return tap(p / size);
    }
    let corner = floor(p - 0.5);
    let f = p - 0.5 - corner;
    let below = mix(reduced(corner, size), reduced(corner + vec2(1.0, 0.0), size), f.x);
    let above = mix(reduced(corner + vec2(0.0, 1.0), size), reduced(corner + 1.0, size), f.x);
    return mix(below, above, f.y);
}

// The signed offset and the weight of tap k of 2 TAP_COUNT - 1, the centre
// being tap TAP_COUNT - 1.
fn signed_tap(k: u32) -> vec2<f32> {
    let centre = TAP_COUNT - 1u;
    let after = k >= centre;
    let t = TAPS[select(centre - k, k - centre, after)];
    return vec2(select(-t.x, t.x, after), t.y);
}

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    var size = vec2<f32>(textureDimensions(source));
    if REDUCE {
        size = vec2<f32>(max(textureDimensions(source) / 2u, vec2(1u)));
    }
    if AXES == 0u {
        var sum = vec4(0.0);
        for (var j = 0u; j < 2u * TAP_COUNT - 1u; j++) {
            let v = signed_tap(j);
            for (var i = 0u; i < 2u * TAP_COUNT - 1u; i++) {
                let h = signed_tap(i);
                sum += level_at(pixel.xy + vec2(h.x, v.x), size) * (h.y * v.y);
            }
        }
        return sum;
    }
    let uv = pixel.xy / size;
    let step = select(vec2(1.0 / size.x, 0.0), vec2(0.0, 1.0 / size.y), AXES == 2u);
    var sum = tap(uv) * TAPS[0].y;
    for (var i = 1u; i < TAP_COUNT; i++) {
        let along = step * TAPS[i].x;
        sum += (tap(uv - along) + tap(uv + along)) * TAPS[i].y;
    }
    return sum;
}

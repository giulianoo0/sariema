// SPDX-License-Identifier: Apache-2.0
// The frame's last pass: miniature blur, bloom, the Hejl-Burgess-Dawson
// curve, the "real world" grade outside the drawn mask, and dither.

struct Params {
    exposure: f32,
    bloom_strength: f32,
    real: f32,
    night: f32,
    tilt: f32,
    // View depth in metres up to which the scene stays sharp.
    focus: f32,
    // 1 when the target is an sRGB format that encodes on write.
    srgb_target: u32,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
// View depth in metres.
@group(0) @binding(1) var view_depth: texture_2d<f32>;
// The chain's 1/2 and 1/4 levels as mips 0 and 1.
@group(0) @binding(2) var chain: texture_2d<f32>;
// Blurred light past the threshold, from 1/4 of the size down, a level per mip.
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var bilinear: sampler;
@group(0) @binding(5) var<uniform> params: Params;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    let uv = pixel.xy / vec2<f32>(textureDimensions(scene));
    // The bottom 28% of the screen blurs over a ramp of 0.2 of its height,
    // and the scene past `focus` over 4 m.
    let band = smoothstep(0.72, 0.92, uv.y);
    let beyond = saturate((textureLoad(view_depth, p, 0).r - params.focus) / 4.0);
    let blur = saturate(params.tilt * max(band * band, beyond));
    // Alpha, the drawn mask, blurs along with the colour.
    var c = mix(textureLoad(scene, p, 0), textureSampleLevel(chain, bilinear, uv, 0.0), sqrt(blur));
    c = mix(c, textureSampleLevel(chain, bilinear, uv, 1.0), blur * blur);
    var glow = vec3(0.0);
    for (var i = 0u; i < textureNumLevels(bloom); i++) {
        glow += textureSampleLevel(bloom, bilinear, uv, f32(i)).rgb;
    }
    let toned = hejl((c.rgb + glow * params.bloom_strength) * params.exposure);
    let graded = mix(toned, real_world(toned, uv), params.real * (1.0 - saturate(c.a)));
    var color = graded + (dither(pixel.xy) - 0.5) / 255.0;
    if params.srgb_target != 0u {
        color = select(pow((color + 0.055) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
    }
    return vec4(color, 1.0);
}

// Its output is already display-encoded.
fn hejl(c: vec3<f32>) -> vec3<f32> {
    let x = max(c - 0.004, vec3(0.0));
    return x * (6.2 * x + 0.5) / (x * (6.2 * x + 1.7) + 0.06);
}

// The Origem game's grade of display-encoded colour: 62% of the saturation
// by day, 45% by night, cooler and darker, under a light vignette.
fn real_world(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let grey = vec3(dot(c, vec3(0.299, 0.587, 0.114)));
    let day = mix(grey, c, 0.62) * vec3(0.98, 1.0, 1.02);
    let night = mix(grey, c, 0.45) * vec3(0.82, 0.92, 1.08) * 0.92;
    let off_centre = uv - 0.5;
    let vignette = 1.0 - dot(off_centre, off_centre) * (0.18 + 0.12 * params.night);
    return mix(day, night, params.night) * vignette;
}

// Interleaved gradient noise (Jimenez 2014) in [0, 1). Each channel is
// shifted as the paper shifts successive frames; uncorrelated channels split
// every grey step into finer coloured ones.
fn dither(pixel: vec2<f32>) -> vec3<f32> {
    let p = floor(pixel);
    return vec3(ign(p), ign(p + 5.588238), ign(p + 11.176476));
}

fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

// SPDX-License-Identifier: Apache-2.0
// The frame's last pass: ambient occlusion, miniature blur, bloom, the
// Hejl-Burgess-Dawson curve, the "real world" grade outside the drawn mask,
// and dither. Appended with ao_surface.wgsl.

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
    // Darkening of a fully occluded pixel; 0 when the occlusion is off.
    ao_strength: f32,
    // The bloom's mip count. While the bloom is off a one-texel black
    // texture is bound, whose single level the sampler clamps to.
    bloom_levels: u32,
    // WebGL2 binds uniforms in whole 16-byte rows.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
// View depth in metres, read when tilt > 0 or ao_strength > 0, and the
// scene blurred at 1/2 and 1/4 of its size, read when tilt > 0.
@group(0) @binding(1) var view_depth: texture_2d<f32>;
@group(0) @binding(2) var half_size: texture_2d<f32>;
@group(0) @binding(3) var quarter_size: texture_2d<f32>;
// Blurred light past the threshold, from 1/4 of the size down, a level per
// mip; one black texel when the bloom is off.
@group(0) @binding(4) var bloom: texture_2d<f32>;
@group(0) @binding(5) var bilinear: sampler;
@group(0) @binding(6) var<uniform> params: Params;
// The fraction of ambient light left and the view depth it is for, at
// 1/AO_DIVISOR of the size rounded up; read when ao_strength > 0.
@group(0) @binding(7) var occlusion: texture_2d<f32>;

override AO_DIVISOR: u32 = 4u;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    let uv = pixel.xy / vec2<f32>(textureDimensions(scene));
    var c = textureLoad(scene, p, 0);
    // Before the miniature blur, whose blurred levels have none, so that
    // the occlusion blurs out with the scene.
    if params.ao_strength > 0.0 {
        c = vec4(c.rgb * mix(1.0, ambient_left(p), params.ao_strength), c.a);
    }
    if params.tilt > 0.0 {
        // The bottom 28% of the screen blurs over a ramp of 0.2 of its
        // height, and the scene past `focus` over 4 m.
        let band = smoothstep(0.72, 0.92, uv.y);
        let beyond = saturate((textureLoad(view_depth, p, 0).r - params.focus) / 4.0);
        let blur = saturate(params.tilt * max(band * band, beyond));
        // Alpha, the drawn mask, blurs along with the colour.
        c = mix(c, textureSampleLevel(half_size, bilinear, uv, 0.0), sqrt(blur));
        c = mix(c, textureSampleLevel(quarter_size, bilinear, uv, 0.0), blur * blur);
    }
    var glow = vec3(0.0);
    for (var i = 0u; i < params.bloom_levels; i++) {
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

// The occlusion at `pixel` from its four nearest texels, weighted
// bilinearly and by how near their depth is to the pixel's so that none
// reaches across a silhouette; 1 when none lies on the pixel's surface.
fn ambient_left(pixel: vec2<i32>) -> f32 {
    let depth = textureLoad(view_depth, pixel, 0).r;
    let at = (vec2<f32>(pixel) - f32(AO_DIVISOR / 2u)) / f32(AO_DIVISOR);
    let first = vec2<i32>(floor(at));
    let f = at - floor(at);
    let last = vec2<i32>(textureDimensions(occlusion)) - 1;
    var sum = 0.0;
    var total = 0.0;
    for (var i = 0; i < 4; i++) {
        let corner = vec2(i & 1, i >> 1);
        let t = clamp(first + corner, vec2(0), last);
        let bilinear = select(1.0 - f, f, corner == vec2(1));
        let texel = textureLoad(occlusion, t, 0).rg;
        let w = bilinear.x * bilinear.y * same_surface(texel.y, depth);
        sum += w * texel.x;
        total += w;
    }
    return select(1.0, sum / total, total > 1e-3);
}

// Its output is already display-encoded.
fn hejl(c: vec3<f32>) -> vec3<f32> {
    let x = max(c - 0.004, vec3(0.0));
    return x * (6.2 * x + 0.5) / (x * (6.2 * x + 1.7) + 0.06);
}

// The ported game's grade of display-encoded colour: 62% of the saturation
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

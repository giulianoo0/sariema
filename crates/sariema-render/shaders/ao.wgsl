// SPDX-License-Identifier: Apache-2.0
// Ambient obscurance at 1/AO_DIVISOR of the target's size, after McGuire,
// Mara and Luebke, "Scalable Ambient Obscurance" (HPG 2012). Samples on a
// spiral within the radius count how far the surface they land on rises
// above the texel's plane, fading out at the radius so that an occluder
// further away, such as an object in front of a far wall, adds nothing.
// Writes the fraction of ambient light left and the view depth it is for.

struct Params {
    // The projection's x and y scale: clip x and y over view depth per unit
    // of view x and y.
    scale: vec2<f32>,
    // Metres.
    radius: f32,
    // View depth of the far plane, where only the background is.
    far: f32,
}

@group(0) @binding(0) var view_depth: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

// The occlusion's size is 1/AO_DIVISOR of the target's, rounded up.
override AO_DIVISOR: u32 = 4u;
override SAMPLES: u32 = 8u;

const TAU: f32 = 6.2831853;
const GOLDEN_ANGLE: f32 = 2.3999632;

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec2<f32> {
    let texel = vec2<i32>(frag.xy);
    let size = vec2<f32>(textureDimensions(view_depth));
    let last = vec2<i32>(size) - 1;
    // The full-size pixel at the texel's centre.
    let pixel = min(texel * i32(AO_DIVISOR) + i32(AO_DIVISOR / 2u), last);
    let p = position(pixel);
    if p.z >= params.far * 0.9999 {
        return vec2(1.0, p.z);
    }
    let n = normal(pixel, p);
    let r2 = params.radius * params.radius;
    // The radius in pixels at this depth.
    let reach = params.radius * params.scale.y * 0.5 * size.y / p.z;
    // A 4×4 pattern of turns that the blur averages away; it stays put on screen.
    let spin = TAU * bayer(texel) / 16.0;
    // Below this height above the plane, depth precision, not geometry, lifts a sample.
    let bias = 0.001 * p.z;
    var sum = 0.0;
    for (var i = 0u; i < SAMPLES; i++) {
        let along = (f32(i) + 0.5) / f32(SAMPLES);
        let angle = f32(i) * GOLDEN_ANGLE + spin;
        let offset = along * reach * vec2(cos(angle), sin(angle));
        let at = clamp(vec2<i32>(vec2<f32>(pixel) + 0.5 + offset), vec2(0), last);
        let v = position(at) - p;
        let vv = dot(v, v);
        let fade = max(1.0 - vv / r2, 0.0);
        sum += fade * max((dot(v, n) - bias) / (vv + 0.01 * r2), 0.0);
    }
    // Times the radius the sum no longer depends on the scene's scale; the 5 is the paper's.
    let occlusion = 5.0 * params.radius * sum / f32(SAMPLES);
    return vec2(1.0 - saturate(occlusion), p.z);
}

// View space with x right, y up and z the distance in front of the camera.
fn position(pixel: vec2<i32>) -> vec3<f32> {
    let depth = textureLoad(view_depth, pixel, 0).r;
    let size = vec2<f32>(textureDimensions(view_depth));
    let ndc = (vec2<f32>(pixel) + 0.5) / size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return vec3(ndc * depth / params.scale, depth);
}

// Facing the camera.
fn normal(pixel: vec2<i32>, p: vec3<f32>) -> vec3<f32> {
    let n = normalize(cross(tangent(pixel, p, vec2(1, 0)), tangent(pixel, p, vec2(0, 1))));
    return select(n, -n, dot(n, p) > 0.0);
}

// Towards the neighbour on `axis` that lies on the pixel's own plane: the
// side whose next two depths extend to the pixel's best, so that no tangent
// straddles an edge. 1/depth is affine on screen across a plane (Wu,
// "Accurate Normal Reconstruction from Depth Buffer", 2021).
fn tangent(pixel: vec2<i32>, p: vec3<f32>, axis: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(view_depth)) - 1;
    let ahead = position(clamp(pixel + axis, vec2(0), last));
    let behind = position(clamp(pixel - axis, vec2(0), last));
    let ahead2 = textureLoad(view_depth, clamp(pixel + 2 * axis, vec2(0), last), 0).r;
    let behind2 = textureLoad(view_depth, clamp(pixel - 2 * axis, vec2(0), last), 0).r;
    let miss_ahead = abs(2.0 / ahead.z - 1.0 / ahead2 - 1.0 / p.z);
    let miss_behind = abs(2.0 / behind.z - 1.0 / behind2 - 1.0 / p.z);
    return select(p - behind, ahead - p, miss_ahead < miss_behind);
}

// 0 to 15, each once in every 4×4 block.
fn bayer(t: vec2<i32>) -> f32 {
    let x = u32(t.x) & 3u;
    let y = u32(t.y) & 3u;
    let a = x ^ y;
    return f32(((a & 1u) << 3u) | ((y & 1u) << 2u) | (a & 2u) | ((y & 2u) >> 1u));
}

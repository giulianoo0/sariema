// SPDX-License-Identifier: Apache-2.0
// Particles and ground dots: a quad per instance, facing the camera or lying
// flat on the ground, drawn last in the scene pass with depth tested and not
// written. Appended to common.wgsl.

struct SpriteFrame {
    // Pixels per metre at one metre of view depth; the least size in pixels.
    scale: vec4<f32>,
}

struct SpriteIn {
    @location(0) position: vec3<f32>,
    @location(1) size: f32,
    @location(2) color: vec4<f32>,
    @location(3) rotation: f32,
    @location(4) additive: f32,
    @location(5) shape: u32,
    @location(6) flags: u32,
}

@group(1) @binding(0) var<uniform> sprites: SpriteFrame;
@group(1) @binding(1) var shapes: texture_2d_array<f32>;
@group(1) @binding(2) var shape_sampler: sampler;

const FLAT: u32 = 1u;

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
    @location(3) @interpolate(flat) additive: f32,
}

@vertex
fn vs_sprite(@builtin(vertex_index) i: u32, s: SpriteIn) -> SpriteOut {
    let corner = vec2(f32(i & 1u), f32(i >> 1u)) * 2.0 - 1.0;
    let c = cos(s.rotation);
    let sn = sin(s.rotation);
    let turned = vec2(corner.x * c - corner.y * sn, corner.x * sn + corner.y * c);
    let centre = frame.view_proj * vec4(s.position, 1.0);
    // A sprite smaller than the least size is drawn that size and as much
    // fainter, so a far one neither vanishes nor shimmers between pixels.
    let pixels = s.size * sprites.scale.x / max(centre.w, 1e-4);
    let grow = max(sprites.scale.y / max(pixels, 1e-6), 1.0);
    let half = 0.5 * s.size * grow;
    var offset: vec3<f32>;
    if (s.flags & FLAT) != 0u {
        offset = vec3(turned.x, 0.0, turned.y) * half;
    } else {
        let right = vec3(frame.view[0].x, frame.view[1].x, frame.view[2].x);
        let up = vec3(frame.view[0].y, frame.view[1].y, frame.view[2].y);
        offset = (right * turned.x + up * turned.y) * half;
    }
    var out: SpriteOut;
    out.clip = frame.view_proj * vec4(s.position + offset, 1.0);
    out.uv = corner * 0.5 + 0.5;
    out.color = vec4(s.color.rgb, s.color.a / grow);
    out.shape = s.shape;
    out.additive = s.additive;
    return out;
}

// Premultiplied: an additive sprite writes no opacity, so it only adds
// light; a covering one darkens what is behind by its opacity.
@fragment
fn fs_sprite(in: SpriteOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(shapes, shape_sampler, in.uv, in.shape).r;
    let a = in.color.a * coverage;
    return vec4(in.color.rgb * a, a * (1.0 - in.additive));
}

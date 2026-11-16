// SPDX-License-Identifier: Apache-2.0
// The scene's depth as view depth in metres, from each pixel's nearest
// sample. post.rs declares `SceneDepth` for the scene's sample count.

struct Projection {
    // View depth is b / (ndc_depth + a) for a perspective with w = -z_view.
    a: f32,
    b: f32,
}

@group(0) @binding(0) var scene_depth: SceneDepth;
@group(0) @binding(1) var<uniform> projection: Projection;

override SAMPLES: u32 = 1u;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) f32 {
    var nearest = 1.0;
    for (var s = 0u; s < SAMPLES; s++) {
        nearest = min(nearest, textureLoad(scene_depth, vec2<i32>(pixel.xy), s));
    }
    return projection.b / (nearest + projection.a);
}

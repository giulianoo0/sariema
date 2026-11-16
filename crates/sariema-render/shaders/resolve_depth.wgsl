// SPDX-License-Identifier: Apache-2.0
// The scene's depth as view depth in metres, from each pixel's nearest
// sample. post.rs declares `SceneDepth` and `load_depth` for the scene's
// sample count and path.

struct Projection {
    // View depth is b / (ndc_depth + a) for a perspective with w = -z_view.
    a: f32,
    b: f32,
    // WebGL2 binds uniforms in whole 16-byte rows.
    _pad: vec2<f32>,
}

@group(0) @binding(0) var scene_depth: SceneDepth;
@group(0) @binding(1) var<uniform> projection: Projection;

override SAMPLES: u32 = 1u;

@fragment
fn fs(@builtin(position) pixel: vec4<f32>) -> @location(0) f32 {
    var nearest = 1.0;
    for (var s = 0u; s < SAMPLES; s++) {
        nearest = min(nearest, load_depth(vec2<i32>(pixel.xy), s));
    }
    return projection.b / (nearest + projection.a);
}

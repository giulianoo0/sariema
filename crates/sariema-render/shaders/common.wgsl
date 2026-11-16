// SPDX-License-Identifier: Apache-2.0
// Declarations shared by every pass that draws room geometry.

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // xyz points towards the sun; w is its shadow slot, -1 for none.
    sun_dir: vec4<f32>,
    // Colour × intensity; zero without a sun.
    sun_color: vec4<f32>,
    sh: array<vec4<f32>, 9>,
    background_bottom: vec4<f32>,
    background_top: vec4<f32>,
    // Tiles across, tiles down, depth slices, tile size in pixels.
    grid: vec4<u32>,
    // A view depth z lies in slice floor(log(z) * x + y).
    slicing: vec4<f32>,
    // The light only actors receive: towards its fill in xyz, the fill's
    // wrap in w; its colour, the fill's intensity in w; then the rim, the
    // rim's power, the least brightness and the room tint.
    character_dir: vec4<f32>,
    character_color: vec4<f32>,
    character: vec4<f32>,
}

struct Draw {
    model: mat4x4<f32>,
    // Columns of the inverse transpose of the model's 3×3 part. The first's
    // w is the draw's presence: 1 drawn whole, 0 gone, between dissolving;
    // negative, it dissolves in the complementary pattern. The second's and
    // third's are its glow's fill and rim.
    normal_x: vec4<f32>,
    normal_y: vec4<f32>,
    normal_z: vec4<f32>,
}

fn draw_normal() -> mat3x3<f32> {
    return mat3x3(draw.normal_x.xyz, draw.normal_y.xyz, draw.normal_z.xyz);
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

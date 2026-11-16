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
}

struct Draw {
    model: mat4x4<f32>,
    normal: mat3x3<f32>,
}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> draw: Draw;

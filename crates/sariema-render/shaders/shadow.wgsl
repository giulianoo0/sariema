// SPDX-License-Identifier: Apache-2.0
// Depth of shadow casters into one atlas slot, set as the viewport. Appended
// to common.wgsl.

@group(2) @binding(0) var<uniform> light_view_proj: mat4x4<f32>;

@vertex
fn vs_shadow(v: VertexIn) -> @builtin(position) vec4<f32> {
    return light_view_proj * draw.model * vec4(v.position, 1.0);
}

// A triangle over the viewport at depth 1, clearing the slot.
@vertex
fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(corner * 2.0 - 1.0, 1.0, 1.0);
}

// SPDX-License-Identifier: Apache-2.0
// One triangle covering the target, for the post-processing passes.

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}

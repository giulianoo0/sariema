// SPDX-License-Identifier: Apache-2.0
// Depth of a skinned actor into a shadow slot. Appended after common.wgsl,
// skin.wgsl and shadow.wgsl.

@vertex
fn vs_actor_shadow(v: SkinnedIn) -> @builtin(position) vec4<f32> {
    return light_view_proj * actor_model() * skin(v.joints, v.weights) * vec4(v.position, 1.0);
}

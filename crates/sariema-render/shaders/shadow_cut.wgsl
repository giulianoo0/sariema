// SPDX-License-Identifier: Apache-2.0
// Depth of a caster in a cutaway that casts its shadow cut. Appended to
// common.wgsl, shadow.wgsl and cut.wgsl.

struct CasterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex
fn vs_shadow_cut(v: VertexIn) -> CasterOut {
    let world = draw.model * vec4(v.position, 1.0);
    return CasterOut(light_view_proj * world, world.xyz);
}

// Both faces cast, as a caster without a cut does.
@fragment
fn fs_shadow_cut(in: CasterOut) {
    if cut_drops(in.world, true, 1.0, in.world) {
        discard;
    }
}

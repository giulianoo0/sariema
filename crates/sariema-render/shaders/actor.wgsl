// SPDX-License-Identifier: Apache-2.0
// Actors in the scene pass, shaded by scene.wgsl's fs_main, and the disc
// shadow under them. Appended after common.wgsl, scene.wgsl and skin.wgsl.

@vertex
fn vs_actor(v: SkinnedIn) -> VertexOut {
    let skinned = skin(v.joints, v.weights);
    let world = actor_model() * skinned * vec4(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.world = world.xyz;
    out.normal = actor_normal_matrix() * skin_normal(skinned, v.normal);
    out.uv = v.uv * material.uv_scale + uv_offset();
    out.color = v.color;
    out.view_depth = -(frame.view * world).z;
    out.presence = 1.0;
    out.glow = vec2(actor[5].w, actor[6].w);
    return out;
}

// An opaque actor's depth, for the downlevel path's depth pass.
@vertex
fn vs_actor_depth(v: SkinnedIn) -> @builtin(position) vec4<f32> {
    let world = actor_model() * skin(v.joints, v.weights) * vec4(v.position, 1.0);
    return frame.view_proj * world;
}

// A face material shows the atlas cell the actor chose; others their own offset.
fn uv_offset() -> vec2<f32> {
    if material.face < 0.0 {
        return material.uv_offset;
    }
    let face = u32(material.face);
    let row = actor[8u + face / 2u];
    return select(row.xy, row.zw, (face & 1u) == 1u);
}

struct DiscOut {
    @builtin(position) clip: vec4<f32>,
    // Position on the disc, the rim at length 1.
    @location(0) at: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) softness: f32,
}

// A square on the horizontal plane through the actor's origin, raised by the
// disc's lift and as wide as its radius times the model's scale.
@vertex
fn vs_disc(@builtin(vertex_index) i: u32) -> DiscOut {
    var corners = array(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0),
    );
    let at = corners[i];
    let model = actor_model();
    let disc = actor[7];
    let radius = disc.x * length(model[0].xyz);
    let world = model[3].xyz + vec3(at.x * radius, disc.w, at.y * radius);
    return DiscOut(frame.view_proj * vec4(world, 1.0), at, disc.y, disc.z);
}

// Black at the disc's opacity, fading out over its soft rim; the blend
// darkens what is under it and the mask is left as it was.
@fragment
fn fs_disc(in: DiscOut) -> @location(0) vec4<f32> {
    let fade = 1.0 - smoothstep(1.0 - in.softness, 1.0, length(in.at));
    return vec4(0.0, 0.0, 0.0, in.opacity * fade);
}

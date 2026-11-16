// SPDX-License-Identifier: Apache-2.0
// An actor's frame data and its skinning. Appended after common.wgsl.

// One actor, in rows of four floats: the model matrix (rows 0 to 3), its
// normal matrix (rows 4 to 6; row 4's w is the first palette row), the disc
// shadow (row 7: radius, opacity, softness, lift), the face offsets (two per
// row from row 8), then four rows per joint of the palette.
@group(1) @binding(1) var<storage, read> actor: array<vec4<f32>>;

struct SkinnedIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
}

fn actor_matrix(row: u32) -> mat4x4<f32> {
    return mat4x4<f32>(actor[row], actor[row + 1u], actor[row + 2u], actor[row + 3u]);
}

fn actor_model() -> mat4x4<f32> {
    return actor_matrix(0u);
}

fn actor_normal_matrix() -> mat3x3<f32> {
    return mat3x3<f32>(actor[4].xyz, actor[5].xyz, actor[6].xyz);
}

// The joint matrices of the vertex, weighted, in the actor's model space.
fn skin(joints: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    let first = u32(actor[4].w);
    return actor_matrix(first + joints.x * 4u) * weights.x
        + actor_matrix(first + joints.y * 4u) * weights.y
        + actor_matrix(first + joints.z * 4u) * weights.z
        + actor_matrix(first + joints.w * 4u) * weights.w;
}

// A normal moves by the inverse transpose of the skin's 3×3 part. Its
// cofactor matrix is that times the determinant; the determinant's sign keeps
// the normal on its side and the length is normalised later.
fn skin_normal(m: mat4x4<f32>, n: vec3<f32>) -> vec3<f32> {
    let a = m[0].xyz;
    let b = m[1].xyz;
    let c = m[2].xyz;
    let cofactor = mat3x3<f32>(cross(b, c), cross(c, a), cross(a, b));
    return cofactor * n * sign(dot(a, cross(b, c)));
}

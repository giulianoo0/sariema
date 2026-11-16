// SPDX-License-Identifier: Apache-2.0
// scene.wgsl's per-frame arrays as textures of TABLE_WIDTH texels per row,
// on the downlevel path, which has no storage buffers; scene.rs prepends
// TABLE_WIDTH. A light takes four texels and a shadow slot six, laid out as
// in the storage buffers, their floats as bits.

@group(0) @binding(1) var light_table: texture_2d<u32>;
@group(0) @binding(2) var cluster_table: texture_2d<u32>;
@group(0) @binding(3) var index_table: texture_2d<u32>;
@group(0) @binding(7) var shadow_table: texture_2d<u32>;

fn table_texel(i: u32) -> vec2<u32> {
    return vec2(i % TABLE_WIDTH, i / TABLE_WIDTH);
}

fn light_at(i: u32) -> Light {
    let first = 4u * i;
    let a = textureLoad(light_table, table_texel(first), 0);
    let b = textureLoad(light_table, table_texel(first + 1u), 0);
    let c = textureLoad(light_table, table_texel(first + 2u), 0);
    let d = textureLoad(light_table, table_texel(first + 3u), 0);
    return Light(
        bitcast<vec3<f32>>(a.xyz), bitcast<f32>(a.w),
        bitcast<vec3<f32>>(b.xyz), bitcast<f32>(b.w),
        bitcast<vec3<f32>>(c.xyz), bitcast<f32>(c.w),
        bitcast<i32>(d.x), d.y,
    );
}

fn cluster_at(i: u32) -> u32 {
    return textureLoad(cluster_table, table_texel(i), 0).x;
}

fn light_index_at(i: u32) -> u32 {
    return textureLoad(index_table, table_texel(i), 0).x;
}

fn shadow_row(first: u32, k: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(textureLoad(shadow_table, table_texel(first + k), 0));
}

fn shadow_at(slot: i32) -> Shadow {
    let first = 6u * u32(slot);
    let to_atlas = mat4x4<f32>(shadow_row(first, 0u), shadow_row(first, 1u), shadow_row(first, 2u), shadow_row(first, 3u));
    return Shadow(to_atlas, shadow_row(first, 4u), shadow_row(first, 5u).xy);
}

// SPDX-License-Identifier: Apache-2.0
// scene.wgsl's per-frame arrays as storage buffers, on the full path.

@group(0) @binding(1) var<storage, read> lights: array<Light>;
// Per cluster: offset into light_indices << COUNT_BITS | light count.
@group(0) @binding(2) var<storage, read> clusters: array<u32>;
@group(0) @binding(3) var<storage, read> light_indices: array<u32>;
@group(0) @binding(7) var<storage, read> shadows: array<Shadow>;

fn light_at(i: u32) -> Light {
    return lights[i];
}

fn cluster_at(i: u32) -> u32 {
    return clusters[i];
}

fn light_index_at(i: u32) -> u32 {
    return light_indices[i];
}

fn shadow_at(slot: i32) -> Shadow {
    return shadows[slot];
}

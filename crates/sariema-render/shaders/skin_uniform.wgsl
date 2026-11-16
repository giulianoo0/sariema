// SPDX-License-Identifier: Apache-2.0
// An actor's record as a uniform buffer of ACTOR_ROWS rows, on the downlevel
// path, which has no storage buffers in vertex shaders; actors.rs prepends
// ACTOR_ROWS.

@group(1) @binding(1) var<uniform> actor: array<vec4<f32>, ACTOR_ROWS>;

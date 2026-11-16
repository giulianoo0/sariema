// SPDX-License-Identifier: Apache-2.0
// One direction of the occlusion's blur, skipping texels of other surfaces:
// a box of TAPS texels whose two end taps weigh half, so 5 or 9 taps average
// ao.wgsl's 4×4 pattern of turns away. Appended with ao_surface.wgsl.

@group(0) @binding(0) var occlusion: texture_2d<f32>;

override VERTICAL: bool = false;
override TAPS: u32 = 5u;

@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec2<f32> {
    let t = vec2<i32>(frag.xy);
    let last = vec2<i32>(textureDimensions(occlusion)) - 1;
    let depth = textureLoad(occlusion, t, 0).g;
    let axis = select(vec2(1, 0), vec2(0, 1), VERTICAL);
    let reach = i32(TAPS / 2u);
    var sum = 0.0;
    var total = 0.0;
    for (var k = -reach; k <= reach; k++) {
        let tap = textureLoad(occlusion, clamp(t + axis * k, vec2(0), last), 0).rg;
        let w = select(1.0, 0.5, abs(k) == reach) * same_surface(tap.y, depth);
        sum += w * tap.x;
        total += w;
    }
    return vec2(sum / total, depth);
}

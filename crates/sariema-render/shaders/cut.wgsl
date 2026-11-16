// SPDX-License-Identifier: Apache-2.0
// Cutaways: scenery on the camera's side of a line that follows the hero,
// cut away above a height or dissolved whole. Appended after common.wgsl to
// the passes that draw room geometry; a draw in no cutaway is bound to one
// that is off.

struct Cut {
    // Across the floor towards the camera in xy, where the line sits along
    // it in z, the height above which it cuts in w.
    plane: vec4<f32>,
    // The hero across the floor in xy, the radius within which it cuts in z
    // (0 for no limit), the softness in w.
    around: vec4<f32>,
    // The colour of the inside of what is cut; w is 1 while it shows.
    cap: vec4<f32>,
    // x is 1 while it cuts; y is 1 when it dissolves what it reaches
    // instead of cutting it.
    on: vec4<f32>,
}

@group(1) @binding(2) var<uniform> cut: Cut;

// Where a point of the draw lies across the floor for the cut: on the wall
// it belongs to, for a draw judged whole.
fn cut_ground(world: vec3<f32>) -> vec2<f32> {
    return select(world.xz, draw.cut.xy, draw.cut.z > 0.0);
}

// How far into the cut a point is across the floor: 1 once a softness past
// the line and as far inside the radius, 0 short of the line or outside it.
fn cut_reach(world: vec3<f32>) -> f32 {
    let ground = cut_ground(world);
    let soft = cut.around.w;
    let past = dot(ground, cut.plane.xy) - cut.plane.z;
    var reach = select(step(0.0, past), saturate(past / soft), soft > 0.0);
    if cut.around.z > 0.0 {
        let inside = cut.around.z - distance(ground, cut.around.xy);
        reach *= select(step(0.0, inside), saturate(inside / soft), soft > 0.0);
    }
    return reach;
}

// Whether a fragment of the draw is dropped. Cutting: above the cut, whose
// height rises steeply where it fades in, or the back of a single-sided draw
// drawn with both sides for its cap, outside the cut. Dissolving: where the
// pixel's `threshold`, in [0, 1), falls under the reach.
fn cut_drops(world: vec3<f32>, front: bool, threshold: f32) -> bool {
    let reach = select(0.0, cut_reach(world), cut.on.x > 0.0);
    if cut.on.y > 0.0 {
        return threshold < reach;
    }
    let above = reach > 0.0 && world.y > cut.plane.w + (1.0 - reach) / reach * cut.around.w;
    return above || (!front && draw.cut.w > 0.0 && reach == 0.0);
}

// Whether a fragment shows the cap: the back of what is cut, seen through it.
fn cut_capped(world: vec3<f32>, front: bool) -> bool {
    return !front && cut.on.x > 0.0 && cut.cap.w > 0.0 && cut_reach(world) > 0.0;
}

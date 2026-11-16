// SPDX-License-Identifier: Apache-2.0
// The share of a colour that fog takes `distance` metres from the camera,
// for `fog` holding where it begins, the inverse of its ramp's length (0 for
// none) and its density: exponential past where it begins, or the ramp,
// whichever is thicker.
fn fog_share(distance: f32, fog: vec4<f32>) -> f32 {
    let beyond = max(distance - fog.x, 0.0);
    return max(1.0 - exp(-fog.z * beyond), saturate(beyond * fog.y));
}

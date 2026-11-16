// SPDX-License-Identifier: Apache-2.0

//! glTF animations as clips: linear or step keys, cubic splines resampled,
//! redundant keys dropped and rotations quantized.

use glam::{Quat, Vec3, Vec4};
use sariema_format::Aabb;
use sariema_format::actor::{Channel, Clip, Interpolation, Property, Transform, Values};

use super::skeleton::Basis;
use super::{ActorSource, KeySettings, matches};
use crate::ImportError;
use crate::gltf::{Gltf, invalid};
use crate::kit::{extra, truthy};

/// Keys of one channel. Rotations are unit quaternions, except a cubic
/// spline's raw values and tangents before [`resample_cubic`].
enum Keys {
    Vec3(Vec<Vec3>),
    Quat(Vec<Quat>),
}

fn unit(keys: Keys) -> Keys {
    match keys {
        Keys::Quat(q) => Keys::Quat(q.into_iter().map(Quat::normalize).collect()),
        vectors => vectors,
    }
}

/// The clips of `gltf`; `joint_of` maps a node to the joint it drives, and
/// channels on other nodes are skipped.
pub(super) fn read(
    gltf: &Gltf,
    joint_of: &dyn Fn(usize) -> Option<u32>,
    rest: &[Transform],
    bases: &[Option<Basis>],
    source: &ActorSource,
) -> Result<Vec<Clip>, ImportError> {
    let keys = &source.keys;
    gltf.doc
        .animations
        .iter()
        .enumerate()
        .map(|(index, animation)| {
            let name = animation.name.clone().unwrap_or_else(|| format!("clip{index}"));
            let bad = |why: String| invalid(format!("clip {name}: {why}"));
            let mut channels: Vec<Channel> = Vec::new();
            let mut duration = 0.0f32;
            let mut skipped = 0;
            for channel in &animation.channels {
                let property = match channel.target.path.as_str() {
                    "translation" => Property::Translation,
                    "rotation" => Property::Rotation,
                    "scale" => Property::Scale,
                    _ => {
                        skipped += 1;
                        continue;
                    }
                };
                let Some(joint) = channel.target.node.and_then(joint_of) else {
                    skipped += 1;
                    continue;
                };
                let sampler = animation
                    .samplers
                    .get(channel.sampler)
                    .ok_or_else(|| bad(format!("sampler {} does not exist", channel.sampler)))?;
                let (times, _) = gltf.floats(sampler.input, &["SCALAR"])?;
                if times.iter().any(|t| !t.is_finite() || *t < 0.0) || !times.is_sorted() || times.is_empty() {
                    return Err(bad("key times must be finite, at least 0 and ascending".into()));
                }
                let kind = if property == Property::Rotation { "VEC4" } else { "VEC3" };
                let (values, _) = gltf.floats(sampler.output, &[kind])?;
                let read = match property {
                    Property::Rotation => Keys::Quat(
                        values.as_chunks::<4>().0.iter().map(|&c| Quat::from_array(c)).collect(),
                    ),
                    _ => Keys::Vec3(values.as_chunks::<3>().0.iter().map(|&c| Vec3::from_array(c)).collect()),
                };
                let (interpolation, times, read) = match sampler.interpolation.as_deref() {
                    None | Some("LINEAR") => (Interpolation::Linear, times, unit(read)),
                    Some("STEP") => (Interpolation::Step, times, unit(read)),
                    Some("CUBICSPLINE") => {
                        let (t, k) = resample_cubic(&times, read, keys.cubic_resample_fps);
                        (Interpolation::Linear, t, k)
                    }
                    Some(other) => return Err(bad(format!("interpolation {other} is not glTF"))),
                };
                let read = match (bases[joint as usize], read) {
                    (None, read) => read,
                    (Some(b), Keys::Quat(q)) => Keys::Quat(q.into_iter().map(|r| b.rotation(r)).collect()),
                    (Some(b), Keys::Vec3(v)) if property == Property::Translation => {
                        Keys::Vec3(v.into_iter().map(|t| b.translation(t)).collect())
                    }
                    (Some(b), Keys::Vec3(v)) => Keys::Vec3(v.into_iter().map(|s| b.scale(s)).collect()),
                };
                let count = match &read {
                    Keys::Vec3(v) => v.len(),
                    Keys::Quat(q) => q.len(),
                };
                if count != times.len() {
                    return Err(bad(format!("{count} values for {} key times", times.len())));
                }
                if channels.iter().any(|c| c.joint == joint && c.property == property) {
                    return Err(bad(format!("two channels animate the {property:?} of joint {joint}")));
                }
                duration = duration.max(times[times.len() - 1]);
                if let Some(channel) = reduce(joint, property, interpolation, &times, read, &rest[joint as usize], keys) {
                    channels.push(channel);
                }
            }
            if skipped > 0 {
                log::warn!("clip {name}: {skipped} channels drive no joint or morph weights and were skipped");
            }
            let looping = match &source.looping {
                Some(patterns) => patterns.iter().any(|p| matches(p, &name)),
                None => truthy(extra(&animation.extras, "loop")),
            };
            Ok(Clip {
                name,
                duration,
                looping,
                channels,
                markers: Vec::new(),
                faces: Vec::new(),
                bounds: Aabb {
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
                ground_speed: None,
            })
        })
        .collect()
}

/// Cubic Hermite splines (glTF stores in-tangent, value, out-tangent per key)
/// sampled `fps` times a second, and at the last key.
fn resample_cubic(times: &[f32], keys: Keys, fps: f32) -> (Vec<f32>, Keys) {
    let (first, last) = (times[0], times[times.len() - 1]);
    let samples = (((last - first) * fps).ceil() as usize).max(1);
    let at: Vec<f32> = (0..=samples)
        .map(|i| first + (last - first) * i as f32 / samples as f32)
        .collect();
    let segment = |t: f32| {
        let k = times
            .partition_point(|&k| k <= t)
            .clamp(1, times.len().max(2) - 1);
        let (t0, t1) = (times[k - 1], times[k.min(times.len() - 1)]);
        let dt = t1 - t0;
        let s = if dt > 0.0 { (t - t0) / dt } else { 0.0 };
        (k - 1, k.min(times.len() - 1), s, dt)
    };
    let hermite = |s: f32| {
        let (s2, s3) = (s * s, s * s * s);
        (
            2.0 * s3 - 3.0 * s2 + 1.0,
            s3 - 2.0 * s2 + s,
            -2.0 * s3 + 3.0 * s2,
            s3 - s2,
        )
    };
    let keys = match keys {
        Keys::Vec3(v) => Keys::Vec3(
            at.iter()
                .map(|&t| {
                    let (a, b, s, dt) = segment(t);
                    let (h00, h10, h01, h11) = hermite(s);
                    v[3 * a + 1] * h00
                        + v[3 * a + 2] * (h10 * dt)
                        + v[3 * b + 1] * h01
                        + v[3 * b] * (h11 * dt)
                })
                .collect(),
        ),
        Keys::Quat(q) => Keys::Quat(
            at.iter()
                .map(|&t| {
                    let (a, b, s, dt) = segment(t);
                    let (h00, h10, h01, h11) = hermite(s);
                    let v = Vec4::from(q[3 * a + 1]) * h00
                        + Vec4::from(q[3 * a + 2]) * (h10 * dt)
                        + Vec4::from(q[3 * b + 1]) * h01
                        + Vec4::from(q[3 * b]) * (h11 * dt);
                    Quat::from_vec4(v).normalize()
                })
                .collect(),
        ),
    };
    (at, keys)
}

/// The channel with only the keys its neighbours cannot reproduce within
/// the tolerance, or `None` when one key equal to the rest pose is left.
fn reduce(
    joint: u32,
    property: Property,
    interpolation: Interpolation,
    times: &[f32],
    keys: Keys,
    rest: &Transform,
    settings: &KeySettings,
) -> Option<Channel> {
    let (kept, values) = match keys {
        Keys::Vec3(v) => {
            let tolerance = match property {
                Property::Translation => settings.translation_tolerance_m,
                _ => settings.scale_tolerance,
            };
            let error = |a: &Vec3, b: &Vec3| (*a - *b).abs().max_element();
            let kept = decimate(
                times,
                &v,
                interpolation,
                tolerance,
                |a, b, t| a.lerp(*b, t),
                error,
            );
            let rest = Vec3::from(match property {
                Property::Translation => rest.translation,
                _ => rest.scale,
            });
            if kept.len() == 1 && error(&v[kept[0]], &rest) <= tolerance {
                return None;
            }
            let values = kept.iter().map(|&k| v[k].to_array()).collect();
            (kept, Values::Vec3(values))
        }
        Keys::Quat(q) => {
            let tolerance = settings.rotation_tolerance_deg.to_radians();
            let error = |a: &Quat, b: &Quat| a.angle_between(*b);
            let kept = decimate(
                times,
                &q,
                interpolation,
                tolerance,
                |a, b, t| a.slerp(*b, t),
                error,
            );
            if kept.len() == 1 && error(&q[kept[0]], &Quat::from_array(rest.rotation)) <= tolerance
            {
                return None;
            }
            let values = if settings.quantize_rotations {
                Values::QuatSnorm16(kept.iter().map(|&k| q[k].to_array().map(snorm16)).collect())
            } else {
                Values::Quat(kept.iter().map(|&k| q[k].to_array()).collect())
            };
            (kept, values)
        }
    };
    Some(Channel {
        joint,
        property,
        interpolation,
        times: kept.iter().map(|&k| times[k]).collect(),
        values,
    })
}

fn snorm16(c: f32) -> i16 {
    (c.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

/// Indices of the keys to keep: Ramer–Douglas–Peucker on time, so every
/// dropped key stays within `tolerance` of what the kept ones interpolate.
/// A step channel keeps the keys where the value changes. One key is left
/// when they are all equal.
fn decimate<T>(
    times: &[f32],
    values: &[T],
    interpolation: Interpolation,
    tolerance: f32,
    lerp: impl Fn(&T, &T, f32) -> T,
    error: impl Fn(&T, &T) -> f32,
) -> Vec<usize> {
    let n = values.len();
    if n <= 1 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    if interpolation == Interpolation::Step {
        let mut last = 0;
        for k in 1..n {
            if error(&values[k], &values[last]) > tolerance {
                keep[k] = true;
                last = k;
            }
        }
    } else {
        keep[n - 1] = true;
        let mut spans = vec![(0, n - 1)];
        while let Some((a, b)) = spans.pop() {
            let span = times[b] - times[a];
            let worst = (a + 1..b)
                .map(|k| {
                    let t = if span > 0.0 {
                        (times[k] - times[a]) / span
                    } else {
                        0.0
                    };
                    (k, error(&lerp(&values[a], &values[b], t), &values[k]))
                })
                .max_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((k, e)) = worst
                && e > tolerance
            {
                keep[k] = true;
                spans.extend([(a, k), (k, b)]);
            }
        }
        if keep.iter().filter(|&&k| k).count() == 2
            && error(&values[0], &values[n - 1]) <= tolerance
        {
            keep[n - 1] = false;
        }
    }
    (0..n).filter(|&k| keep[k]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(values: &[f32], tolerance: f32) -> Vec<usize> {
        let times: Vec<f32> = (0..values.len()).map(|i| i as f32).collect();
        let v: Vec<Vec3> = values.iter().map(|&y| Vec3::new(0.0, y, 0.0)).collect();
        let error = |a: &Vec3, b: &Vec3| (*a - *b).abs().max_element();
        decimate(
            &times,
            &v,
            Interpolation::Linear,
            tolerance,
            |a, b, t| a.lerp(*b, t),
            error,
        )
    }

    #[test]
    fn keys_on_a_line_are_dropped_and_corners_kept() {
        assert_eq!(kept(&[0.0, 1.0, 2.0, 3.0], 1e-4), [0, 3]);
        assert_eq!(kept(&[0.0, 1.0, 2.0, 1.0, 0.0], 1e-4), [0, 2, 4]);
        assert_eq!(kept(&[0.5; 6], 1e-4), [0]);
        assert_eq!(kept(&[0.0, 0.05, 0.0], 0.1), [0]);
    }

    #[test]
    fn every_dropped_key_stays_within_the_tolerance() {
        let values: Vec<f32> = (0..60).map(|i| (i as f32 * 0.05).sin()).collect();
        let keep = kept(&values, 0.01);
        assert!(keep.len() < values.len() / 2, "{keep:?}");
        for w in keep.windows(2) {
            for k in w[0]..w[1] {
                let t = (k - w[0]) as f32 / (w[1] - w[0]) as f32;
                let line = values[w[0]] + (values[w[1]] - values[w[0]]) * t;
                assert!((line - values[k]).abs() <= 0.01 + 1e-6, "key {k}");
            }
        }
    }

    #[test]
    fn step_keys_are_kept_where_the_value_changes() {
        let times = [0.0, 1.0, 2.0, 3.0];
        let v = [1.0, 1.0, 2.0, 2.0].map(|y| Vec3::new(y, 0.0, 0.0));
        let error = |a: &Vec3, b: &Vec3| (*a - *b).abs().max_element();
        let keep = decimate(&times, &v, Interpolation::Step, 1e-4, |a, _, _| *a, error);
        assert_eq!(keep, [0, 2]);
    }

    #[test]
    fn a_cubic_spline_with_flat_tangents_eases_between_keys() {
        let zero = Vec3::ZERO;
        let keys = Keys::Vec3(vec![zero, zero, zero, zero, Vec3::X, zero]);
        let (times, Keys::Vec3(v)) = resample_cubic(&[0.0, 1.0], keys, 4.0) else {
            unreachable!("vectors stay vectors");
        };
        assert_eq!(times, [0.0, 0.25, 0.5, 0.75, 1.0]);
        let x: Vec<f32> = v.iter().map(|p| p.x).collect();
        assert_eq!(x[0], 0.0);
        assert!(
            (x[2] - 0.5).abs() < 1e-6 && x[1] < 0.25 && (x[4] - 1.0).abs() < 1e-6,
            "{x:?}"
        );
    }
}

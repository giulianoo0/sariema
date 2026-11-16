// SPDX-License-Identifier: Apache-2.0

//! Skeletal animation on the CPU: sampling clips into local poses, layered
//! blending with crossfades and joint masks, events, fixed-step playback with
//! interpolation for rendering, sockets and the skinning palette.
//!
//! A [`Rig`] is built once per actor pack and shared; an [`Animator`] holds
//! one actor's playback and every buffer it needs, so stepping and
//! evaluating allocate nothing. Results depend only on the commands given and
//! the number of steps taken, never on wall-clock time.

mod animator;
mod mask;
mod motion;
mod pose;
mod rig;

pub use animator::{AnimSettings, Animator, Easing, Event, EventKind, LayerBlend, Play, PlayMode};
pub use mask::JointMask;
pub use motion::ground_speed;
pub use pose::Transform;
pub use rig::{ClipId, ClipInfo, FaceId, JointId, Rig, SocketId};

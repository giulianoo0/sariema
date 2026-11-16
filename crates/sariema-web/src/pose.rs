// SPDX-License-Identifier: Apache-2.0

//! Actors the page places and plays a clip on, posed at the frame's
//! interpolated time with buffers kept from frame to frame.

use glam::Mat4;
use sariema_anim::{ClipId, Rig, Transform};
use sariema_format::actor::ActorPack;

/// A loaded pack's skeleton and inverse bind matrices.
pub struct Model {
    rig: Rig,
    inverse_bind: Vec<Mat4>,
}

impl Model {
    pub fn new(pack: &ActorPack) -> Result<Model, String> {
        Ok(Model {
            rig: Rig::new(pack)?,
            inverse_bind: (pack.joints.iter())
                .map(|j| Mat4::from_cols_array(&j.inverse_bind))
                .collect(),
        })
    }

    pub fn clip(&self, name: &str) -> Option<ClipId> {
        self.rig.clip(name)
    }

    pub fn joints(&self) -> usize {
        self.rig.joints()
    }
}

/// What a clip plays from: its time at simulation time `since_s`, and how
/// fast it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playing {
    pub clip: ClipId,
    pub time_s: f32,
    pub since_s: f64,
    pub speed: f32,
}

pub struct Posed {
    pub model: usize,
    pub transform: Mat4,
    pub visible: bool,
    pub playing: Option<Playing>,
    local: Vec<Transform>,
    model_space: Vec<Mat4>,
    palette: Vec<Mat4>,
}

impl Posed {
    pub fn new(model: usize, joints: usize) -> Posed {
        Posed {
            model,
            transform: Mat4::IDENTITY,
            visible: false,
            playing: None,
            local: vec![Transform::IDENTITY; joints],
            model_space: vec![Mat4::IDENTITY; joints],
            palette: vec![Mat4::IDENTITY; joints],
        }
    }

    /// The palette at simulation time `now_s`: the clip sampled where it has
    /// got to, or the bind pose without one.
    pub fn palette(&mut self, model: &Model, now_s: f64) -> &[Mat4] {
        match self.playing {
            Some(p) => {
                let time = p.time_s + ((now_s - p.since_s) * p.speed as f64) as f32;
                model.rig.sample(p.clip, time, &mut self.local);
                model.rig.model_space(&self.local, &mut self.model_space);
                for ((out, m), inverse) in (self.palette.iter_mut())
                    .zip(&self.model_space)
                    .zip(&model.inverse_bind)
                {
                    *out = *m * *inverse;
                }
            }
            None => self.palette.fill(Mat4::IDENTITY),
        }
        &self.palette
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sariema_format::actor::demo::demo_arm;

    #[test]
    fn a_clip_advances_with_simulation_time_at_its_speed() {
        let pack = demo_arm();
        let model = Model::new(&pack).unwrap();
        let bend = model.clip("bend").unwrap();
        let mut moving = Posed::new(0, pack.joints.len());
        moving.playing = Some(Playing {
            clip: bend,
            time_s: 0.0,
            since_s: 10.0,
            speed: 0.5,
        });
        let mut still = Posed::new(0, pack.joints.len());
        still.playing = Some(Playing {
            clip: bend,
            time_s: 0.5,
            since_s: 0.0,
            speed: 0.0,
        });
        let at = moving.palette(&model, 11.0).to_vec();
        assert_eq!(at, still.palette(&model, 99.0));
        assert_ne!(at, moving.palette(&model, 10.0));
    }

    #[test]
    fn without_a_clip_the_palette_is_the_bind_pose() {
        let pack = demo_arm();
        let model = Model::new(&pack).unwrap();
        let mut posed = Posed::new(0, pack.joints.len());
        assert!(
            posed
                .palette(&model, 3.0)
                .iter()
                .all(|m| *m == Mat4::IDENTITY)
        );
    }
}

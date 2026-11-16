// SPDX-License-Identifier: Apache-2.0

use crate::rig::{JointId, Rig};

/// How much a layer drives each joint, from 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct JointMask {
    weights: Vec<f32>,
}

impl JointMask {
    /// No joint.
    pub fn none(rig: &Rig) -> JointMask {
        JointMask {
            weights: vec![0.0; rig.joints()],
        }
    }

    /// `root` and every joint below it.
    pub fn subtree(rig: &Rig, root: JointId) -> JointMask {
        let mut mask = JointMask::none(rig);
        mask.set_subtree(rig, root, 1.0);
        mask
    }

    /// Sets `weight`, clamped to 0..1, on `root` and every joint below it.
    pub fn set_subtree(&mut self, rig: &Rig, root: JointId, weight: f32) {
        let weight = weight.clamp(0.0, 1.0);
        for (j, w) in self.weights.iter_mut().enumerate() {
            let mut at = Some(JointId(j as u32));
            while let Some(joint) = at {
                if joint == root {
                    *w = weight;
                    break;
                }
                at = rig.parent(joint);
            }
        }
    }

    pub fn weights(&self) -> &[f32] {
        &self.weights
    }
}

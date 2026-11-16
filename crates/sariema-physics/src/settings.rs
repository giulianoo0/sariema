// SPDX-License-Identifier: Apache-2.0

//! Every number that bounds or tunes the simulation. The defaults come from
//! `docs/superpowers/research/2026-11-15-movement-collision.md`; `docs/physics.md`
//! explains each one. Lengths are metres, angles degrees unless the name says
//! radians, speeds metres per second.

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields, default)
)]
pub struct Settings {
    /// Seconds simulated by one [`crate::World::step`]. A host that steps
    /// the world sets it, so a game's settings leave it out.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub step_seconds: f32,
    pub collision: CollisionSettings,
    pub controller: ControllerSettings,
    pub nav: NavSettings,
    pub move_to: MoveToSettings,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields, default)
)]
pub struct CollisionSettings {
    /// Gap the controller leaves between an actor and what stopped it.
    pub skin: f32,
    /// Shapes closer than this count as touching rather than apart.
    pub contact_tolerance: f32,
    /// A sweep stops refining the time of contact once the gap is below this.
    pub sweep_tolerance: f32,
    /// Refinement steps of one sweep against one part.
    pub sweep_iterations: u32,
    /// A shape touching another is stopped only when it moves into it at
    /// more than this cosine; smaller is rounding, taken as sliding along.
    pub tangent_tolerance: f32,
    /// A contact normal within this angle of a triangle's face normal is
    /// taken as the face normal, so seams and vertices of a flat surface
    /// slide like its faces.
    pub flat_edge_deg: f32,
    /// Push-out passes after a teleport before it is rejected.
    pub depenetration_iterations: u32,
    /// Total push-out allowed after a teleport before it is rejected.
    pub max_depenetration: f32,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields, default)
)]
pub struct ControllerSettings {
    /// Downward acceleration, m/s².
    pub gravity: f32,
    pub max_fall_speed: f32,
    /// Sweeps per move before the rest of the motion is dropped.
    pub slide_iterations: u32,
    /// Tallest ledge walked onto without jumping.
    pub step_height: f32,
    /// How far below its feet an actor that was standing looks for ground to stay on.
    pub ground_snap: f32,
    /// Steepest ground an actor stands and walks on.
    pub max_slope_deg: f32,
    /// Input within this angle of pushing straight into a wall may be helped round a corner.
    pub corner_assist_angle_deg: f32,
    /// How far sideways a corner may be for the help to apply.
    pub corner_assist_distance: f32,
    /// Sideways speed of the help, as a fraction of the actor's speed.
    pub corner_assist_speed: f32,
    /// Sideways offsets tried, evenly spaced up to `corner_assist_distance`.
    pub corner_assist_probes: u32,
    /// Fraction of the speed kept when sliding along a wall at
    /// `wall_slide_full_angle_deg` or shallower, instead of the cosine.
    pub wall_slide_preservation: f32,
    /// Angle between input and wall from which the full preservation applies;
    /// it fades to plain projection at `90 − corner_assist_angle_deg`.
    pub wall_slide_full_angle_deg: f32,
    /// Stick deflection, as a fraction of its radius, that moves nothing.
    pub stick_dead_zone: f32,
    /// Stick deflection that gives full speed.
    pub stick_full: f32,
    /// Exponent of the speed curve between dead zone and full; 1 is linear.
    pub stick_exponent: f32,
    /// Steps to reach full speed from rest and to stop; 0 is instant.
    pub accel_steps: u32,
    /// Rate, per second, at which facing closes on its target: each step
    /// covers `1 − e^(−rate·dt)` of the remaining angle.
    pub yaw_rate: f32,
    /// Changes of the wanted facing smaller than this, in radians, are ignored.
    pub yaw_filter: f32,
    /// Radians per second an actor turns at most, whatever `yaw_rate` asks;
    /// infinite leaves turning to `yaw_rate` alone. A host that caps turning
    /// sets it, so a game's settings leave it out.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub max_yaw_speed: f32,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields, default)
)]
pub struct NavSettings {
    /// Side of a walkable-grid cell.
    pub cell_size: f32,
    /// Walkable floors stacked in one grid column at most.
    pub max_layers: u32,
    /// How far from a start or target to look for the nearest walkable cell.
    pub snap_radius: f32,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(deny_unknown_fields, default)
)]
pub struct MoveToSettings {
    /// Distance to the target that counts as arrived.
    pub arrive_distance: f32,
    /// Distance to an intermediate waypoint at which the next one is taken.
    pub waypoint_distance: f32,
    /// Facing error, in radians, that counts as turned.
    pub face_tolerance: f32,
    /// Steps of too little progress before a scripted move reports `blocked`.
    pub stuck_steps: u32,
    /// Progress below this fraction of the expected travel counts as too little.
    pub stuck_progress: f32,
}

impl Settings {
    /// Why these settings cannot run, if they cannot: a size or count that
    /// would make a step or a grid divide by zero or never end, a length or
    /// angle out of range.
    pub fn validate(&self) -> Result<(), String> {
        let (c, k, n, m) = (&self.collision, &self.controller, &self.nav, &self.move_to);
        let length = |x: f32| x.is_finite() && x >= 0.0;
        let positive = |x: f32| x.is_finite() && x > 0.0;
        let fraction = |x: f32| (0.0..=1.0).contains(&x);
        let checks = [
            ("collision.skin", length(c.skin)),
            ("collision.contact_tolerance", length(c.contact_tolerance)),
            ("collision.sweep_tolerance", length(c.sweep_tolerance)),
            ("collision.sweep_iterations", c.sweep_iterations > 0),
            ("collision.tangent_tolerance", length(c.tangent_tolerance)),
            ("collision.max_depenetration", length(c.max_depenetration)),
            ("controller.gravity", length(k.gravity)),
            ("controller.max_fall_speed", length(k.max_fall_speed)),
            ("controller.slide_iterations", k.slide_iterations > 0),
            ("controller.step_height", length(k.step_height)),
            ("controller.ground_snap", length(k.ground_snap)),
            (
                "controller.max_slope_deg",
                length(k.max_slope_deg) && k.max_slope_deg < 90.0,
            ),
            (
                "controller.corner_assist_distance",
                length(k.corner_assist_distance),
            ),
            (
                "controller.wall_slide_preservation",
                fraction(k.wall_slide_preservation),
            ),
            ("controller.stick_dead_zone", length(k.stick_dead_zone)),
            (
                "controller.stick_full",
                k.stick_full > k.stick_dead_zone && k.stick_full <= 1.0,
            ),
            ("controller.stick_exponent", positive(k.stick_exponent)),
            ("controller.yaw_rate", length(k.yaw_rate)),
            ("controller.yaw_filter", length(k.yaw_filter)),
            ("nav.cell_size", positive(n.cell_size)),
            ("nav.max_layers", n.max_layers > 0),
            ("nav.snap_radius", length(n.snap_radius)),
            ("move_to.arrive_distance", length(m.arrive_distance)),
            ("move_to.waypoint_distance", length(m.waypoint_distance)),
            ("move_to.face_tolerance", length(m.face_tolerance)),
            ("move_to.stuck_steps", m.stuck_steps > 0),
            ("move_to.stuck_progress", fraction(m.stuck_progress)),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((name, _)) => Err(format!("physics.{name} is out of range")),
            None => Ok(()),
        }
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            step_seconds: 1.0 / 60.0,
            collision: CollisionSettings::default(),
            controller: ControllerSettings::default(),
            nav: NavSettings::default(),
            move_to: MoveToSettings::default(),
        }
    }
}

impl Default for CollisionSettings {
    fn default() -> CollisionSettings {
        CollisionSettings {
            skin: 0.01,
            contact_tolerance: 1e-4,
            sweep_tolerance: 1e-5,
            sweep_iterations: 24,
            tangent_tolerance: 1e-4,
            flat_edge_deg: 0.5,
            depenetration_iterations: 8,
            max_depenetration: 1.0,
        }
    }
}

impl Default for ControllerSettings {
    fn default() -> ControllerSettings {
        ControllerSettings {
            gravity: 9.81,
            max_fall_speed: 30.0,
            slide_iterations: 4,
            step_height: 0.20,
            ground_snap: 0.25,
            max_slope_deg: 40.0,
            corner_assist_angle_deg: 25.0,
            corner_assist_distance: 0.15,
            corner_assist_speed: 0.6,
            corner_assist_probes: 3,
            wall_slide_preservation: 0.85,
            wall_slide_full_angle_deg: 45.0,
            stick_dead_zone: 0.15,
            stick_full: 0.7,
            stick_exponent: 1.0,
            accel_steps: 0,
            yaw_rate: 14.0,
            yaw_filter: 0.1,
            max_yaw_speed: f32::INFINITY,
        }
    }
}

impl Default for NavSettings {
    fn default() -> NavSettings {
        NavSettings {
            cell_size: 0.1,
            max_layers: 4,
            snap_radius: 0.5,
        }
    }
}

impl Default for MoveToSettings {
    fn default() -> MoveToSettings {
        MoveToSettings {
            arrive_distance: 0.02,
            waypoint_distance: 0.1,
            face_tolerance: 0.05,
            stuck_steps: 30,
            stuck_progress: 0.2,
        }
    }
}

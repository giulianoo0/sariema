// SPDX-License-Identifier: Apache-2.0

//! The build-time rules of a cutscene, checked by abstract execution: each
//! block is followed with what is known of the fade, of every role's
//! presence, and of whether a camera cut happened in the current instant.
//!
//! - A visible actor never moves discontinuously: a `teleport` of a role
//!   that may be visible, or was hidden without a step passing since, must
//!   declare its cover, and the cover must hold:
//!   `cut` needs a shot with blend 0 earlier in the same instant (no block
//!   that takes time in between), `fade` needs the screen known to be fully
//!   faded. (`E_CUTSCENE_TELEPORT`, `E_CUTSCENE_COVER`)
//! - Every role declares its start place, unless the cutscene spawns it
//!   before using it. (`E_CUTSCENE_START`, `E_CUTSCENE_ABSENT`, `E_CUTSCENE_SPAWN`)
//! - Parallel branches never move the same role or set the same flag, so
//!   skipping gives the same end state as playing. (`E_CUTSCENE_RACE`)
//! - No walk is faster than an actor may be seen to move: neither a `walk`
//!   nor, for a role with a start, `cutscene.start_walk_speed` exceeds the
//!   project's `motion.max_visible_speed`. (`E_CUTSCENE_SPEED`)

use std::collections::BTreeSet;

use sariema_format::script::cutscene::{Await, Block, Cover, Cutscene, Shot};

use crate::settings::Settings;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    pub code: &'static str,
    pub cutscene: String,
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cutscene {}, {}: {}",
            self.cutscene, self.at, self.message
        )
    }
}

/// Every error code the check can give.
pub const CODES: [&str; 7] = [
    "E_CUTSCENE_TELEPORT",
    "E_CUTSCENE_COVER",
    "E_CUTSCENE_START",
    "E_CUTSCENE_ABSENT",
    "E_CUTSCENE_SPAWN",
    "E_CUTSCENE_RACE",
    "E_CUTSCENE_SPEED",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presence {
    /// Shown, or may be.
    Visible,
    /// Hidden, but maybe shown when the current step began.
    Hiding,
    /// Hidden for at least one whole step.
    Hidden,
    /// Not spawned yet, never declared a start.
    Unborn,
    Despawned,
}

#[derive(Clone, Debug, PartialEq)]
struct Known {
    /// `None` when the fade level cannot be known here.
    fade: Option<f32>,
    roles: Vec<Presence>,
    cut: bool,
}

#[derive(Default)]
struct Writes {
    moved: BTreeSet<u16>,
    flags: BTreeSet<u32>,
    fade: bool,
    shown: BTreeSet<u16>,
}

struct Checker<'a> {
    c: &'a Cutscene,
    max_speed: f32,
    errors: Vec<CheckError>,
}

impl Checker<'_> {
    fn fail(&mut self, code: &'static str, at: &str, message: String) {
        self.errors.push(CheckError {
            code,
            cutscene: self.c.name.clone(),
            at: at.into(),
            message,
        });
    }

    fn walks(&mut self, speed: f32, what: &str, at: &str) {
        let max = self.max_speed;
        if speed.abs() > max {
            self.fail(
                "E_CUTSCENE_SPEED",
                at,
                format!("{what} {speed} m/s is faster than motion.max_visible_speed ({max} m/s)"),
            );
        }
    }

    fn name(&self, role: u16) -> &str {
        &self.c.roles[role as usize].name
    }

    fn uses(&mut self, k: &Known, role: u16, at: &str) {
        match k.roles[role as usize] {
            Presence::Unborn => self.fail(
                "E_CUTSCENE_START",
                at,
                format!(
                    "role {} has no start place and is used before a spawn; declare where it starts",
                    self.name(role)
                ),
            ),
            Presence::Despawned => {
                self.fail("E_CUTSCENE_ABSENT", at, format!("role {} is used after its despawn", self.name(role)))
            }
            _ => {}
        }
    }

    fn list(&mut self, blocks: &[Block], at: &str, k: &mut Known) {
        for (i, b) in blocks.iter().enumerate() {
            self.block(b, &format!("{at}[{i}]"), k);
        }
    }

    fn block(&mut self, block: &Block, at: &str, k: &mut Known) {
        match block {
            Block::Sequence(blocks) => self.list(blocks, &format!("{at}.sequence"), k),
            Block::Parallel(branches) => self.parallel(branches, at, k),
            Block::Branch {
                then, otherwise, ..
            } => {
                let (mut a, mut b) = (k.clone(), k.clone());
                self.list(then, &format!("{at}.branch.then"), &mut a);
                self.list(otherwise, &format!("{at}.branch.else"), &mut b);
                k.fade = if a.fade == b.fade { a.fade } else { None };
                for (r, (x, y)) in k.roles.iter_mut().zip(a.roles.iter().zip(&b.roles)) {
                    *r = match (*x, *y) {
                        (x, y) if x == y => x,
                        (
                            Presence::Hidden | Presence::Hiding,
                            Presence::Hidden | Presence::Hiding,
                        ) => Presence::Hiding,
                        _ => Presence::Visible,
                    };
                }
                k.cut = false;
            }
            Block::Wait(t) => {
                k.cut &= *t <= 0.0;
                if *t > 0.0 {
                    elapse(k);
                }
            }
            Block::WaitFor { on, .. } => {
                if let Await::Arrived(r)
                | Await::ClipFinished(r)
                | Await::ClipMarker { role: r, .. } = on
                {
                    self.uses(k, *r, &format!("{at}.wait_for"));
                }
                k.cut = false;
                elapse(k);
            }
            Block::Shot { shot, blend, .. } => {
                let at = format!("{at}.shot");
                match *shot {
                    Shot::Follow { subject, .. } => self.uses(k, subject, &at),
                    Shot::Frame { a, b, .. } => {
                        self.uses(k, a, &at);
                        self.uses(k, b, &at);
                    }
                    Shot::Fixed { .. } => {}
                }
                k.cut |= *blend <= 0.0;
            }
            Block::Walk { role, speed, .. } => {
                let at = format!("{at}.walk");
                self.uses(k, *role, &at);
                self.walks(*speed, "walk speed", &at);
                k.cut = false;
            }
            Block::Face { role, .. } => {
                self.uses(k, *role, &format!("{at}.face"));
                k.cut = false;
            }
            Block::Line { speaker, .. } => {
                self.uses(k, *speaker, &format!("{at}.line"));
                k.cut = false;
                elapse(k);
            }
            Block::Clip { role, .. } => self.uses(k, *role, &format!("{at}.clip")),
            Block::Hinge { role, .. } => {
                self.uses(k, *role, &format!("{at}.hinge"));
                k.cut = false;
                elapse(k);
            }
            Block::LookAt { role, target } => {
                self.uses(k, *role, &format!("{at}.look_at"));
                if let Some(t) = target {
                    self.uses(k, *t, &format!("{at}.look_at"));
                }
            }
            Block::Sound { at: Some(r), .. } => self.uses(k, *r, &format!("{at}.sound")),
            Block::Fade { to, duration, .. } => {
                k.fade = Some(*to);
                k.cut &= *duration <= 0.0;
            }
            Block::Show { role } | Block::Hide { role } => {
                let show = matches!(block, Block::Show { .. });
                self.uses(
                    k,
                    *role,
                    &format!("{at}.{}", if show { "show" } else { "hide" }),
                );
                let r = &mut k.roles[*role as usize];
                *r = match (show, *r) {
                    (true, _) => Presence::Visible,
                    (false, Presence::Hidden) => Presence::Hidden,
                    (false, _) => Presence::Hiding,
                };
            }
            Block::Despawn { role } => {
                self.uses(k, *role, &format!("{at}.despawn"));
                k.roles[*role as usize] = Presence::Despawned;
            }
            Block::Spawn { role, .. } => {
                if !matches!(
                    k.roles[*role as usize],
                    Presence::Unborn | Presence::Despawned
                ) {
                    self.fail(
                        "E_CUTSCENE_SPAWN",
                        &format!("{at}.spawn"),
                        format!(
                            "role {} may already be in the world; spawn only a role without a start place, or after its despawn",
                            self.name(*role)
                        ),
                    );
                }
                k.roles[*role as usize] = Presence::Visible;
            }
            Block::Teleport { role, cover, .. } => {
                self.teleport(*role, *cover, &format!("{at}.teleport"), k)
            }
            Block::Sound { .. }
            | Block::Music { .. }
            | Block::Letterbox { .. }
            | Block::SetFlag { .. }
            | Block::Call { .. }
            | Block::TimeScale { .. }
            | Block::CameraShake { .. } => {}
        }
    }

    fn teleport(&mut self, role: u16, cover: Option<Cover>, at: &str, k: &Known) {
        self.uses(k, role, at);
        if !matches!(k.roles[role as usize], Presence::Visible | Presence::Hiding) {
            return;
        }
        let name = self.name(role).to_string();
        match cover {
            None => self.fail(
                "E_CUTSCENE_TELEPORT",
                at,
                format!(
                    "role {name} may be on screen and would jump; walk it instead, hide it and wait before the teleport, or declare \"cover\": \"cut\" right after a shot with blend 0 or \"fade\" under a fade to 1"
                ),
            ),
            Some(Cover::Cut) if !k.cut => self.fail(
                "E_CUTSCENE_COVER",
                at,
                format!("teleport of {name} declares a cut, but no shot with blend 0 comes before it in the same instant"),
            ),
            Some(Cover::Fade) if k.fade != Some(1.0) => self.fail(
                "E_CUTSCENE_COVER",
                at,
                format!(
                    "teleport of {name} declares a fade, but the screen is not known to be fully faded here (fade is {})",
                    k.fade.map_or("unknown".into(), |f| f.to_string())
                ),
            ),
            Some(_) => {}
        }
    }

    fn parallel(&mut self, branches: &[Vec<Block>], at: &str, k: &mut Known) {
        let writes: Vec<Writes> = branches.iter().map(|b| writes_of(b)).collect();
        for i in 0..writes.len() {
            for j in i + 1..writes.len() {
                let roles: Vec<String> = writes[i]
                    .moved
                    .intersection(&writes[j].moved)
                    .map(|r| self.name(*r).to_string())
                    .collect();
                if !roles.is_empty() {
                    self.fail(
                        "E_CUTSCENE_RACE",
                        &format!("{at}.parallel"),
                        format!("branches {i} and {j} both move {}", roles.join(", ")),
                    );
                }
                if writes[i]
                    .flags
                    .intersection(&writes[j].flags)
                    .next()
                    .is_some()
                {
                    self.fail(
                        "E_CUTSCENE_RACE",
                        &format!("{at}.parallel"),
                        format!("branches {i} and {j} set the same flag"),
                    );
                }
            }
        }
        let faders = writes.iter().filter(|w| w.fade).count();
        let entry = k.clone();
        let mut exits = Vec::new();
        for (b, blocks) in branches.iter().enumerate() {
            let mut kb = k.clone();
            if faders > usize::from(writes[b].fade) {
                kb.fade = None;
            }
            self.list(blocks, &format!("{at}.parallel[{b}]"), &mut kb);
            exits.push(kb);
        }
        k.fade = match faders {
            0 => k.fade,
            1 => exits[writes.iter().position(|w| w.fade).expect("one fades")].fade,
            _ => None,
        };
        // The parallel lasts as long as its longest branch: a role no branch
        // touches is as hidden as the branch that hid it longest left it.
        let mut waited = entry;
        for exit in &exits {
            for (w, e) in waited.roles.iter_mut().zip(&exit.roles) {
                if *w == Presence::Hiding && *e == Presence::Hidden {
                    *w = Presence::Hidden;
                }
            }
        }
        for r in 0..k.roles.len() as u16 {
            let changed: Vec<usize> = (0..writes.len())
                .filter(|&b| writes[b].shown.contains(&r) || writes[b].moved.contains(&r))
                .collect();
            k.roles[r as usize] = match changed.as_slice() {
                [] => waited.roles[r as usize],
                [b] => exits[*b].roles[r as usize],
                _ => Presence::Visible,
            };
        }
        k.cut = false;
    }
}

/// A block that surely lasts at least one step ran: a role hidden in the
/// instant it began has now been hidden for a whole step.
fn elapse(k: &mut Known) {
    for r in &mut k.roles {
        if *r == Presence::Hiding {
            *r = Presence::Hidden;
        }
    }
}

fn writes_of(blocks: &[Block]) -> Writes {
    let mut w = Writes::default();
    collect(blocks, &mut w);
    w
}

fn collect(blocks: &[Block], w: &mut Writes) {
    for b in blocks {
        match b {
            Block::Sequence(bs) => collect(bs, w),
            Block::Parallel(branches) => branches.iter().for_each(|bs| collect(bs, w)),
            Block::Branch {
                then, otherwise, ..
            } => {
                collect(then, w);
                collect(otherwise, w);
            }
            Block::Walk { role, .. }
            | Block::Teleport { role, .. }
            | Block::Spawn { role, .. }
            | Block::Despawn { role }
            | Block::Face { role, .. } => {
                w.moved.insert(*role);
            }
            Block::Show { role } | Block::Hide { role } => {
                w.shown.insert(*role);
            }
            Block::SetFlag { flag, .. } => {
                w.flags.insert(*flag);
            }
            Block::Fade { .. } => w.fade = true,
            _ => {}
        }
    }
}

/// The rules of `c` for a game run with `settings`.
pub fn check(c: &Cutscene, settings: &Settings) -> Vec<CheckError> {
    let mut k = Known {
        fade: None,
        roles: c
            .roles
            .iter()
            .map(|r| {
                if r.start.is_some() {
                    Presence::Visible
                } else {
                    Presence::Unborn
                }
            })
            .collect(),
        cut: false,
    };
    let mut checker = Checker {
        c,
        max_speed: settings.motion.max_visible_speed,
        errors: Vec::new(),
    };
    if let Some(role) = c.roles.iter().find(|r| r.start.is_some()) {
        checker.walks(
            settings.cutscene.start_walk_speed,
            "cutscene.start_walk_speed",
            &format!("roles.{}.start", role.name),
        );
    }
    checker.list(&c.body, "steps", &mut k);
    checker.errors
}

// SPDX-License-Identifier: Apache-2.0

//! The HUD stage: the screen the game drives with `ui_*` commands, its
//! joystick and buttons read as the player's input, and the layer the
//! engine draws over it from the runtime's screen state: the dialogue line,
//! the cinema bars and the fade. While a cutscene holds control the game's
//! HUD steps aside.

use glam::Vec2;
use sariema_format::script::{Command, Controller, Entity, Event, name_id};
use sariema_runtime::input::{Actions, InputSettings};
use sariema_runtime::outbox::UiOp;
use sariema_runtime::state::State;
use sariema_ui::{
    Color, Direction, DrawList, Justify, Length, NodeDesc, NodeId, Prop, Radius, SafeArea, Sides,
    Ui, UiDesc, UiError,
};

use crate::settings::HudSettings;

/// The group of a root node that stays on screen through a cutscene.
pub const KEEP_IN_CUTSCENES: &str = "cutscene";

/// The bars are laid out at their full height and scaled from their outer
/// edge to the letterbox's, and the line is moved down by what they lack,
/// so a step that only moves them lays nothing out again.
struct Layer {
    fade: NodeId,
    /// The top bar and the bottom one.
    bars: [NodeId; 2],
    /// The bottom bar and, riding on it, the dialogue line.
    dock: NodeId,
    /// The dialogue box and its text; none without a font.
    line: Option<(NodeId, NodeId)>,
    /// The bars' laid-out height, at letterbox 1, in UI units.
    full: f32,
    /// The bars' drawn height last set, in UI units.
    bar: f32,
}

/// A node of the game's root that steps aside in cutscenes, and whether
/// the game wants it shown.
struct Panel {
    node: NodeId,
    shown: bool,
}

pub struct Hud {
    pub ui: Ui,
    /// Node name ids, for the game's `ui_*` commands.
    nodes: Vec<(u32, NodeId)>,
    animations: Vec<(u32, String)>,
    joystick: Option<NodeId>,
    /// Button nodes named after an input button, with its index.
    buttons: Vec<(NodeId, usize)>,
    held: Vec<bool>,
    layer: Layer,
    panels: Vec<Panel>,
    /// Whether the panels are hidden for a cutscene.
    aside: bool,
    /// The line shown, so its text is set once.
    shown: Option<u32>,
    /// The fade at the end of the step before.
    last_fade: f32,
    settings: HudSettings,
    pub rejected: Vec<Event<'static>>,
}

impl Hud {
    /// The HUD described by `description`, reading the files it names with
    /// `load`; with none, a bare screen that only carries the engine's layer.
    pub fn new(
        description: Option<&str>,
        load: impl FnMut(&str) -> Result<Vec<u8>, String>,
        input: &InputSettings,
        settings: HudSettings,
    ) -> Result<Hud, UiError> {
        let desc: UiDesc = match description {
            Some(text) => serde_json::from_str(text).map_err(|e| UiError::Schema(e.to_string()))?,
            None => UiDesc::default(),
        };
        let mut ui = Ui::from_desc(&desc, load)?;
        let nodes = ui.names().map(|(n, id)| (name_id(n), id)).collect();
        let joystick = ui
            .names()
            .map(|(_, id)| id)
            .find(|&id| ui.stick(id).is_some());
        let buttons = input
            .buttons
            .iter()
            .enumerate()
            .filter_map(|(i, b)| ui.node(&b.name).map(|id| (id, i)))
            .collect();
        let panels = desc
            .nodes
            .iter()
            .zip(ui.children(ui.root()))
            .filter(|(d, _)| !d.groups.iter().any(|g| g == KEEP_IN_CUTSCENES))
            .map(|(d, node)| Panel {
                node,
                shown: d.visible.unwrap_or(true),
            })
            .collect();
        let font = desc.fonts.keys().next().cloned();
        let layer = layer(&mut ui, font.as_deref(), &settings)?;
        ui.set_screen(settings.width, settings.height, SafeArea::default());
        let mut hud = Hud {
            nodes,
            animations: desc
                .animations
                .keys()
                .map(|n| (name_id(n), n.clone()))
                .collect(),
            joystick,
            buttons,
            held: vec![false; input.buttons.len()],
            layer,
            panels,
            aside: false,
            shown: None,
            last_fade: 0.0,
            settings,
            rejected: Vec::new(),
            ui,
        };
        hud.fit_bars();
        let root = hud.ui.root();
        hud.ui.snap(root)?;
        Ok(hud)
    }

    /// Shapes a dialogue line ahead, so showing it shapes nothing.
    pub fn prepare_line(&mut self, text: &str) -> Result<(), UiError> {
        match self.layer.line {
            Some((_, node)) => self.ui.prepare_text(node, text),
            None => Ok(()),
        }
    }

    pub fn bare(input: &InputSettings, settings: HudSettings) -> Hud {
        Hud::new(None, |f| Err(f.to_owned()), input, settings)
            .expect("the engine's own layer is valid")
    }

    /// Lays the screen out at its new size at once, so the next step does
    /// not.
    pub fn resize(&mut self, width: f32, height: f32, safe: SafeArea) {
        self.ui.set_screen(width, height, safe);
        self.fit_bars();
        let root = self.ui.root();
        self.ui.snap(root).expect("the root lives");
    }

    pub fn pointer_down(&mut self, pointer: u64, x: f32, y: f32) {
        self.ui.pointer_down(pointer, x, y);
    }

    pub fn pointer_move(&mut self, pointer: u64, x: f32, y: f32) {
        self.ui.pointer_move(pointer, x, y);
    }

    pub fn pointer_up(&mut self, pointer: u64, x: f32, y: f32) {
        self.ui.pointer_up(pointer, x, y);
    }

    /// Runs the screen's step and gives the joystick, while held, as the
    /// touch stick (y up), and the buttons held, ORed into `actions`.
    pub fn input(&mut self, actions: &mut Actions) -> Option<Vec2> {
        for event in self.ui.step() {
            let (node, down) = match *event {
                sariema_ui::Event::Press { node, .. } => (node, true),
                sariema_ui::Event::Release { node, .. } => (node, false),
                _ => continue,
            };
            if let Some(&(_, i)) = self.buttons.iter().find(|(id, _)| *id == node) {
                self.held[i] = down;
            }
        }
        for (button, &held) in actions.buttons.iter_mut().zip(&self.held) {
            *button |= held;
        }
        let [x, y] = self.joystick.and_then(|id| self.ui.stick(id))?;
        (x != 0.0 || y != 0.0).then_some(Vec2::new(x, -y))
    }

    /// The game's HUD commands of the step, and the engine's layer set to
    /// the step's screen state; `docs/engine.md` has the fade contract.
    pub fn step(&mut self, s: &State) {
        for op in &s.outbox.ui {
            let result = match op {
                UiOp::Visible { node, visible } => {
                    self.node(*node).and_then(|id| self.show(id, *visible))
                }
                UiOp::Text { node, text } => {
                    self.node(*node).and_then(|id| self.ui.set_text(id, text))
                }
                UiOp::Value { node, value } => self
                    .node(*node)
                    .and_then(|id| self.ui.set(id, Prop::Progress, *value)),
                UiOp::Play { animation } => {
                    match self.animations.iter().find(|(n, _)| n == animation) {
                        Some((_, name)) => self.ui.play(name),
                        None => Err(UiError::Unknown(format!("animation {animation:#010x}"))),
                    }
                }
            };
            if let Err(e) = result {
                self.rejected.push(Event::Rejected {
                    opcode: opcode(op),
                    code: e.code(),
                    entity: Entity::NONE,
                });
            }
        }
        self.step_aside(s);
        self.draw_screen(s);
    }

    /// Shows or hides a node for the game; a panel hidden for a cutscene
    /// takes the game's wish when it returns.
    fn show(&mut self, id: NodeId, shown: bool) -> Result<(), UiError> {
        match self.panels.iter_mut().find(|p| p.node == id) {
            Some(panel) => {
                panel.shown = shown;
                self.ui.set_visible(id, shown && !self.aside)
            }
            None => self.ui.set_visible(id, shown),
        }
    }

    /// Hides the panels while a cutscene holds control, and until the
    /// cinema bars it leaves behind are gone, so no bar ever covers them.
    fn step_aside(&mut self, s: &State) {
        let cutscene = s.control.by == Controller::Cutscene;
        let bars = s.screen.letterbox.value() > 0.0;
        let aside = self.settings.hide_in_cutscenes && (cutscene || (self.aside && bars));
        if aside == self.aside {
            return;
        }
        self.aside = aside;
        for p in &self.panels {
            self.ui
                .set_visible(p.node, p.shown && !aside)
                .expect("the game's root nodes live");
        }
    }

    fn node(&self, name: u32) -> Result<NodeId, UiError> {
        self.nodes
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, id)| id)
            .ok_or_else(|| UiError::Unknown(format!("node {name:#010x}")))
    }

    /// Lays the bars out at their full height for the screen, when it
    /// changed, and has the next step place them and the line again.
    fn fit_bars(&mut self) -> bool {
        let screen = self.ui.screen();
        let full = self.settings.bar_fraction * screen.height / screen.scale.max(f32::EPSILON);
        if full == self.layer.full {
            return false;
        }
        for id in self.layer.bars {
            self.ui
                .set(id, Prop::Height, full)
                .expect("the engine's layer nodes live");
        }
        self.layer.full = full;
        self.layer.bar = f32::NAN;
        true
    }

    fn draw_screen(&mut self, s: &State) {
        let mut moved = self.fit_bars();
        let full = self.layer.full;
        let ui = &mut self.ui;
        let fade = self.layer.fade;
        let colour = s.screen.fade_color;
        let set = |ui: &mut Ui, id, prop, v| {
            ui.set(id, prop, v).expect("the engine's layer nodes live");
        };
        let now = s.screen.fade.value();
        set(ui, fade, Prop::Opacity, now.max(self.last_fade));
        self.last_fade = now;
        set(ui, fade, Prop::Red, colour.x);
        set(ui, fade, Prop::Green, colour.y);
        set(ui, fade, Prop::Blue, colour.z);
        let bar = s.screen.letterbox.value() * full;
        if moved || bar != self.layer.bar {
            let scale = if full > 0.0 { bar / full } else { 0.0 };
            for id in self.layer.bars {
                set(ui, id, Prop::ScaleY, scale);
                ui.set_visible(id, bar > 0.0)
                    .expect("the engine's layer nodes live");
            }
            if let Some((frame, _)) = self.layer.line {
                set(ui, frame, Prop::Y, full - bar);
            }
            self.layer.bar = bar;
        }
        if let Some((frame, text)) = self.layer.line {
            let line = s.dialogue.line.as_ref();
            let id = line.map(|l| l.line);
            if id != self.shown {
                if let Some(l) = line {
                    ui.set_text(text, &l.text)
                        .expect("the engine's layer nodes live");
                }
                ui.set_visible(frame, line.is_some())
                    .expect("the engine's layer nodes live");
                self.shown = id;
                moved = true;
            }
        }
        if moved {
            for id in [self.layer.bars[0], self.layer.dock] {
                ui.snap(id).expect("the engine's layer nodes live");
            }
        }
    }

    pub fn draw(&mut self, alpha: f32) -> &DrawList {
        self.ui.draw(alpha)
    }

    /// The opacity the fade is drawn at until the next step.
    pub fn fade(&self) -> f32 {
        self.ui.value(self.layer.fade, Prop::Opacity).unwrap_or(0.0)
    }
}

/// The engine's layer over the game's nodes: the top bar, the bottom bar
/// with the dialogue line riding on it, so the line is never under the bar,
/// then the fade over everything.
fn layer(ui: &mut Ui, font: Option<&str>, settings: &HudSettings) -> Result<Layer, UiError> {
    let root = ui.root();
    let zero = Some(Length::Units(0.0));
    let black = Some(Color::BLACK);
    let top = ui.create(
        root,
        &NodeDesc {
            absolute: true,
            left: zero,
            right: zero,
            top: zero,
            height: zero,
            fill: black,
            origin: Some([0.5, 0.0]),
            visible: Some(false),
            ..NodeDesc::default()
        },
    )?;
    let dock = ui.create(
        root,
        &NodeDesc {
            absolute: true,
            left: zero,
            right: zero,
            bottom: zero,
            direction: Some(Direction::Column),
            ..NodeDesc::default()
        },
    )?;
    let line = match font {
        Some(font) => {
            let inset = Length::Percent(settings.line_inset);
            let [v, h] = settings.line_padding.map(Length::Units);
            let frame = ui.create(
                dock,
                &NodeDesc {
                    margin: Some(Sides::Four([
                        Length::Units(0.0),
                        inset,
                        Length::Units(settings.line_margin),
                        inset,
                    ])),
                    padding: Some(Sides::Two([v, h])),
                    radius: Some(Radius::All(settings.line_radius)),
                    fill: Some(settings.line_fill),
                    justify: Some(Justify::Center),
                    visible: Some(false),
                    ..NodeDesc::default()
                },
            )?;
            let text = ui.create(
                frame,
                &NodeDesc {
                    text: Some(" ".into()),
                    font: Some(font.into()),
                    size: Some(settings.line_size),
                    fill: Some(settings.line_color),
                    ..NodeDesc::default()
                },
            )?;
            Some((frame, text))
        }
        None => None,
    };
    let bottom = ui.create(
        dock,
        &NodeDesc {
            height: zero,
            fill: black,
            origin: Some([0.5, 1.0]),
            visible: Some(false),
            ..NodeDesc::default()
        },
    )?;
    let fade = ui.create(
        root,
        &NodeDesc {
            absolute: true,
            left: zero,
            top: zero,
            right: zero,
            bottom: zero,
            fill: black,
            opacity: Some(0.0),
            ..NodeDesc::default()
        },
    )?;
    Ok(Layer {
        fade,
        bars: [top, bottom],
        dock,
        line,
        full: 0.0,
        bar: 0.0,
    })
}

fn opcode(op: &UiOp) -> u16 {
    match op {
        UiOp::Visible { .. } => Command::UiVisible {
            node: 0,
            visible: false,
        }
        .opcode(),
        UiOp::Text { .. } => Command::UiText { node: 0, text: "" }.opcode(),
        UiOp::Value { .. } => Command::UiValue {
            node: 0,
            value: 0.0,
        }
        .opcode(),
        UiOp::Play { .. } => Command::UiPlay { animation: 0 }.opcode(),
    }
}

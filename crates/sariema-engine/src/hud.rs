// SPDX-License-Identifier: Apache-2.0

//! The HUD stage: the screen the game drives with `ui_*` commands, its
//! joystick and buttons read as the player's input, and the layer the
//! engine draws over it from the runtime's screen state: the dialogue line,
//! the cinema bars and the fade.

use glam::Vec2;
use sariema_format::script::{Command, Entity, Event, name_id};
use sariema_runtime::input::{Actions, InputSettings};
use sariema_runtime::outbox::UiOp;
use sariema_runtime::state::State;
use sariema_ui::{DrawList, NodeDesc, NodeId, Prop, SafeArea, Ui, UiDesc, UiError};
use serde_json::json;

use crate::settings::HudSettings;

struct Layer {
    fade: NodeId,
    bars: [NodeId; 2],
    /// The dialogue box and its text; none without a font.
    line: Option<(NodeId, NodeId)>,
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
        let font = desc.fonts.keys().next().cloned();
        let layer = layer(&mut ui, font.as_deref(), &settings)?;
        ui.set_screen(settings.width, settings.height, SafeArea::default());
        Ok(Hud {
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
            shown: None,
            last_fade: 0.0,
            settings,
            rejected: Vec::new(),
            ui,
        })
    }

    pub fn bare(input: &InputSettings, settings: HudSettings) -> Hud {
        Hud::new(None, |f| Err(f.to_owned()), input, settings)
            .expect("the engine's own layer is valid")
    }

    pub fn resize(&mut self, width: f32, height: f32, safe: SafeArea) {
        self.ui.set_screen(width, height, safe);
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
    /// the step's screen state. The fade and the bars are set, not
    /// animated, so a frame never draws them between two steps' values. The
    /// fade is drawn at the larger of this step's value and the one before:
    /// the frames up to the next step show this step's moves, and the
    /// no-jump rule lets a step move anything at once when the screen was
    /// fully faded at its start or at its end.
    pub fn step(&mut self, s: &State) {
        for op in &s.outbox.ui {
            let result = match op {
                UiOp::Visible { node, visible } => self
                    .node(*node)
                    .and_then(|id| self.ui.set_visible(id, *visible)),
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
        self.draw_screen(s);
    }

    fn node(&self, name: u32) -> Result<NodeId, UiError> {
        self.nodes
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, id)| id)
            .ok_or_else(|| UiError::Unknown(format!("node {name:#010x}")))
    }

    fn draw_screen(&mut self, s: &State) {
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
        let screen = ui.screen();
        let bar = s.screen.letterbox.value() * self.settings.bar_fraction * screen.height
            / screen.scale.max(f32::EPSILON);
        for id in self.layer.bars {
            set(ui, id, Prop::Height, bar);
        }
        let Some((frame, text)) = self.layer.line else {
            return;
        };
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

fn layer(ui: &mut Ui, font: Option<&str>, settings: &HudSettings) -> Result<Layer, UiError> {
    let root = ui.root();
    let create = |ui: &mut Ui, parent: NodeId, value: serde_json::Value| {
        let desc: NodeDesc =
            serde_json::from_value(value).map_err(|e| UiError::Schema(e.to_string()))?;
        ui.create(parent, &desc)
    };
    let line = match font {
        Some(font) => {
            let frame = create(
                ui,
                root,
                json!({
                    "absolute": true, "left": "12%", "right": "12%", "bottom": settings.line_margin,
                    "padding": [10, 18], "radius": 12, "fill": "#000000B3", "justify": "center"
                }),
            )?;
            let text = create(
                ui,
                frame,
                json!({ "text": " ", "font": font, "size": settings.line_size, "fill": "#FFFFFF" }),
            )?;
            ui.set_visible(frame, false)?;
            Some((frame, text))
        }
        None => None,
    };
    let bar = |edge: &str| json!({ "absolute": true, "left": 0, "right": 0, edge: 0, "height": 0, "fill": "#000000" });
    let bars = [
        create(ui, root, bar("top"))?,
        create(ui, root, bar("bottom"))?,
    ];
    let fade = create(
        ui,
        root,
        json!({ "absolute": true, "left": 0, "top": 0, "right": 0, "bottom": 0, "fill": "#000000", "opacity": 0 }),
    )?;
    Ok(Layer { fade, bars, line })
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

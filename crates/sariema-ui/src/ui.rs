// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::path::Path;

use taffy::prelude::{AvailableSpace, Dimension, LengthPercentageAuto, Position, Size, TaffyTree};
use taffy::{Rect, Style, compute_leaf_layout};

use crate::UiError;
use crate::anim::{self, Anim, Ease, Motion, SpringRest};
use crate::atlas::{Atlas, Cell};
use crate::color::Color;
use crate::desc::{NodeDesc, Track, UiDesc};
use crate::draw::DrawList;
use crate::input::{Event, Pointer};
use crate::node::{self, Gradient, Kind, Node, NodeId, PROPS, Prop, Text};
use crate::settings::{SafeArea, UiSettings};
use crate::text::{Font, Line, break_lines};
use crate::widget::{Button, Joystick, Widget, WidgetDesc};

/// A decoded image, premultiplied RGBA8; its texture is `1 + index`.
pub struct ImageData {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The screen in physical pixels and how UI units map onto it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub width: f32,
    pub height: f32,
    pub safe: SafeArea,
    /// Pixels per UI unit.
    pub scale: f32,
}

/// A property change for [`Ui::animate`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    pub to: f32,
    pub motion: Motion,
    /// Start value. `None`: the value when the animation starts.
    pub from: Option<f32>,
    /// Seconds before it starts.
    pub delay: f32,
}

impl Animation {
    pub fn tween(to: f32, duration: f32, ease: Ease) -> Animation {
        Animation {
            to,
            motion: Motion::Tween { duration, ease },
            from: None,
            delay: 0.0,
        }
    }

    pub fn spring(to: f32, response: f32, damping: f32) -> Animation {
        Animation {
            to,
            motion: Motion::Spring { response, damping },
            from: None,
            delay: 0.0,
        }
    }

    /// Reveals a text node's characters; set on [`Prop::Reveal`].
    pub fn typewriter(rate: f32, pause: f32) -> Animation {
        Animation {
            to: f32::INFINITY,
            motion: Motion::Typewriter { rate, pause },
            from: None,
            delay: 0.0,
        }
    }

    pub fn from(self, from: f32) -> Animation {
        Animation {
            from: Some(from),
            ..self
        }
    }

    pub fn delay(self, delay: f32) -> Animation {
        Animation { delay, ..self }
    }
}

pub struct Ui {
    pub(crate) settings: UiSettings,
    pub(crate) nodes: Vec<Node>,
    free: Vec<u32>,
    names: HashMap<String, u32>,
    pub(crate) taffy: TaffyTree<u32>,
    pub(crate) root: u32,
    pub(crate) fonts: Vec<Font>,
    font_names: HashMap<String, u32>,
    pub(crate) images: Vec<ImageData>,
    image_names: HashMap<String, u32>,
    pub(crate) atlas: Atlas,
    glyphs: HashMap<(u32, u16), Option<Cell>>,
    pub(crate) anims: Vec<Anim>,
    animations: HashMap<String, Vec<Track>>,
    pub(crate) screen: Screen,
    pub(crate) layout_dirty: bool,
    pub(crate) widgets: Vec<u32>,
    pub(crate) pointers: Vec<Pointer>,
    pub(crate) pending: Vec<Event>,
    pub(crate) events: Vec<Event>,
    pub(crate) focus: Option<u32>,
    pub(crate) draw: DrawList,
}

impl Ui {
    pub fn new(settings: UiSettings) -> Result<Ui, UiError> {
        check_settings(&settings)?;
        let [w, h] = settings.reference;
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        let root_style = Style {
            position: Position::Relative,
            size: Size {
                width: Dimension::length(w),
                height: Dimension::length(h),
            },
            ..Style::default()
        };
        let root_taffy = taffy.new_leaf(root_style).expect("a new tree has room");
        let root = Node {
            generation: 0,
            live: true,
            fresh: true,
            taffy: root_taffy,
            parent: None,
            children: Vec::new(),
            name: None,
            kind: Kind::Box,
            border: None,
            gradient: None,
            radius: [[0.0; 2]; 4],
            shadow: None,
            origin: [0.5, 0.5],
            clip: false,
            visible: true,
            interactive: false,
            focus: None,
            safe: false,
            cur: node::initial_values(Color::TRANSPARENT),
            prev: node::initial_values(Color::TRANSPARENT),
            rect: [0.0; 4],
            rect_prev: [0.0; 4],
            content: [0.0; 4],
            widget: None,
        };
        Ok(Ui {
            atlas: Atlas::new(settings.atlas_size, settings.sdf_spread),
            anims: Vec::with_capacity(settings.animations),
            pointers: Vec::with_capacity(settings.pointers),
            screen: Screen {
                width: w,
                height: h,
                safe: SafeArea::default(),
                scale: 1.0,
            },
            settings,
            nodes: vec![root],
            free: Vec::new(),
            names: HashMap::new(),
            taffy,
            root: 0,
            fonts: Vec::new(),
            font_names: HashMap::new(),
            images: Vec::new(),
            image_names: HashMap::new(),
            glyphs: HashMap::new(),
            animations: HashMap::new(),
            layout_dirty: true,
            widgets: Vec::new(),
            pending: Vec::new(),
            events: Vec::new(),
            focus: None,
            draw: DrawList::default(),
        })
    }

    /// Builds a screen from its description, reading the files it names
    /// through `load` (relative paths as written in the description).
    pub fn from_desc(
        desc: &UiDesc,
        mut load: impl FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<Ui, UiError> {
        let mut ui = Ui::new(desc.settings.clone())?;
        for (name, font) in &desc.fonts {
            let data = load(&font.file).map_err(UiError::Io)?;
            ui.add_font(name, data, font.weight)?;
        }
        for (name, file) in &desc.images {
            let data = load(file).map_err(UiError::Io)?;
            let decoded = image::load_from_memory(&data)
                .map_err(|e| UiError::Image(format!("{name}: {file}: {e}")))?
                .to_rgba8();
            let (w, h) = decoded.dimensions();
            ui.add_image(name, w, h, decoded.into_raw())?;
        }
        let root = ui.root();
        for node in &desc.nodes {
            ui.create(root, node)?;
        }
        for (name, tracks) in &desc.animations {
            ui.add_animation(name, tracks.clone())?;
        }
        Ok(ui)
    }

    /// Reads a description file; the files it names are relative to it.
    pub fn load(path: &Path) -> Result<Ui, UiError> {
        let io = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
        let text = std::fs::read_to_string(path).map_err(|e| UiError::Io(io(path, e)))?;
        let desc: UiDesc = serde_json::from_str(&text)
            .map_err(|e| UiError::Schema(format!("{}: {e}", path.display())))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Ui::from_desc(&desc, |file| {
            let p = base.join(file);
            std::fs::read(&p).map_err(|e| io(&p, e))
        })
    }

    pub fn settings(&self) -> &UiSettings {
        &self.settings
    }

    pub fn add_font(
        &mut self,
        name: &str,
        data: Vec<u8>,
        weight: Option<f32>,
    ) -> Result<(), UiError> {
        if self.font_names.contains_key(name) {
            return Err(UiError::Schema(format!("two fonts are named {name:?}")));
        }
        let font = Font::new(data, weight).map_err(|e| UiError::Font(format!("{name}: {e}")))?;
        self.font_names.insert(name.into(), self.fonts.len() as u32);
        self.fonts.push(font);
        Ok(())
    }

    /// Adds an image from straight-alpha RGBA8; returns its texture id.
    pub fn add_image(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        mut rgba: Vec<u8>,
    ) -> Result<u32, UiError> {
        if self.image_names.contains_key(name) {
            return Err(UiError::Schema(format!("two images are named {name:?}")));
        }
        if rgba.len() != width as usize * height as usize * 4 || width == 0 || height == 0 {
            return Err(UiError::Image(format!(
                "{name}: not {width} × {height} RGBA8"
            )));
        }
        for px in rgba.as_chunks_mut::<4>().0 {
            let a = u16::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
        self.image_names
            .insert(name.into(), self.images.len() as u32);
        self.images.push(ImageData {
            name: name.into(),
            width,
            height,
            rgba,
        });
        Ok(self.images.len() as u32)
    }

    /// Adds a named animation for [`Ui::play`]; its nodes must exist.
    pub fn add_animation(&mut self, name: &str, tracks: Vec<Track>) -> Result<(), UiError> {
        let schema = |why: String| UiError::Schema(format!("animation {name:?}: {why}"));
        for (i, t) in tracks.iter().enumerate() {
            let nodes = t.node.iter().chain(&t.nodes);
            if t.node.is_some() == !t.nodes.is_empty() {
                return Err(schema(format!("track {i} needs one of node or nodes")));
            }
            for n in nodes {
                if !self.names.contains_key(n) {
                    return Err(UiError::Unknown(format!(
                        "node {n:?} in animation {name:?}"
                    )));
                }
            }
            let motions = [
                t.duration.is_some(),
                t.spring.is_some(),
                t.typewriter.is_some(),
            ];
            if motions.iter().filter(|m| **m).count() != 1 {
                return Err(schema(format!(
                    "track {i} needs exactly one of duration, spring and typewriter"
                )));
            }
            if t.typewriter.is_some() != (t.prop == crate::desc::Target::Reveal) {
                return Err(schema(format!(
                    "track {i}: a typewriter animates reveal, and only it"
                )));
            }
            if t.typewriter.is_none() && t.to.is_none() {
                return Err(schema(format!("track {i} has no to")));
            }
            if t.after && i > 0 && tracks[i - 1].duration.is_none() {
                return Err(schema(format!(
                    "track {i} starts after a track without a duration"
                )));
            }
        }
        self.animations.insert(name.into(), tracks);
        Ok(())
    }

    pub fn root(&self) -> NodeId {
        self.id(self.root)
    }

    /// The node's children, in drawing order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let children = match self.live(id) {
            Ok(i) => self.nodes[i as usize].children.as_slice(),
            Err(_) => &[],
        };
        children.iter().map(|&i| self.id(i))
    }

    pub fn node(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).map(|&i| self.id(i))
    }

    /// Named nodes, in no particular order.
    /// Named nodes, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = (&str, NodeId)> {
        self.names.iter().map(|(n, &i)| (n.as_str(), self.id(i)))
    }

    pub fn name(&self, id: NodeId) -> Option<&str> {
        self.live(id)
            .ok()
            .and_then(|i| self.nodes[i as usize].name.as_deref())
    }

    /// Live nodes, the root included.
    pub fn len(&self) -> usize {
        self.nodes.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 1
    }

    pub(crate) fn id(&self, index: u32) -> NodeId {
        NodeId {
            index,
            generation: self.nodes[index as usize].generation,
        }
    }

    pub(crate) fn live(&self, id: NodeId) -> Result<u32, UiError> {
        match self.nodes.get(id.index as usize) {
            Some(n) if n.live && n.generation == id.generation => Ok(id.index),
            _ => Err(UiError::Unknown(format!("node {}", id.index))),
        }
    }

    /// Creates `desc` and its children under `parent`; nothing is created
    /// if any of them fails.
    pub fn create(&mut self, parent: NodeId, desc: &NodeDesc) -> Result<NodeId, UiError> {
        let parent = self.live(parent)?;
        let index = self.create_one(parent, desc)?;
        if let Err(e) = self.attach_widgets(index, desc) {
            self.remove_index(index);
            return Err(e);
        }
        self.layout_dirty = true;
        Ok(self.id(index))
    }

    fn create_one(&mut self, parent: u32, desc: &NodeDesc) -> Result<u32, UiError> {
        let index = self.new_node(parent, desc)?;
        for child in &desc.children {
            if let Err(e) = self.create_one(index, child) {
                self.remove_index(index);
                return Err(e);
            }
        }
        Ok(index)
    }

    fn new_node(&mut self, parent: u32, desc: &NodeDesc) -> Result<u32, UiError> {
        let label = desc.name.as_deref().unwrap_or("(unnamed)");
        if let Some(name) = &desc.name
            && self.names.contains_key(name)
        {
            return Err(UiError::Schema(format!("two nodes are named {name:?}")));
        }
        let kinds = [
            desc.text.is_some(),
            desc.image.is_some(),
            desc.ring.is_some(),
            desc.stroke.is_some(),
        ];
        if kinds.iter().filter(|k| **k).count() > 1 {
            return Err(UiError::Schema(format!(
                "node {label} has more than one of text, image, ring and stroke"
            )));
        }
        if let Some(g) = &desc.gradient {
            g.check()
                .map_err(|why| UiError::Schema(format!("node {label}: {why}")))?;
        }
        let (kind, fill) = self.kind(desc, label)?;
        let index = self.free.last().copied().unwrap_or(self.nodes.len() as u32);
        let style = desc.style();
        let taffy = if matches!(kind, Kind::Text(_)) {
            self.taffy.new_leaf_with_context(style, index)
        } else {
            self.taffy.new_leaf(style)
        }
        .expect("taffy refuses nothing on a new leaf");
        self.taffy
            .add_child(self.nodes[parent as usize].taffy, taffy)
            .expect("the parent is live");
        let mut cur = node::initial_values(desc.fill.unwrap_or(fill));
        let (sx, sy) = match desc.scale {
            None => (1.0, 1.0),
            Some(crate::desc::Scale::Uniform(s)) => (s, s),
            Some(crate::desc::Scale::Each([x, y])) => (x, y),
        };
        cur[Prop::X as usize] = desc.x;
        cur[Prop::Y as usize] = desc.y;
        cur[Prop::ScaleX as usize] = sx;
        cur[Prop::ScaleY as usize] = sy;
        cur[Prop::Rotation as usize] = desc.rotation;
        cur[Prop::Opacity as usize] = desc.opacity.unwrap_or(1.0);
        cur[Prop::Progress as usize] = desc.progress.unwrap_or(1.0);
        cur[Prop::Reveal as usize] = desc.reveal.unwrap_or(f32::INFINITY);
        let generation = self.nodes.get(index as usize).map_or(0, |n| n.generation);
        let node = Node {
            generation,
            live: true,
            fresh: true,
            taffy,
            parent: Some(parent),
            children: Vec::new(),
            name: desc.name.clone(),
            kind,
            border: desc.border,
            gradient: desc.gradient,
            radius: desc.radius.map_or([[0.0; 2]; 4], |r| r.corners()),
            shadow: desc.shadow,
            origin: desc.origin.unwrap_or([0.5, 0.5]),
            clip: desc.clip,
            visible: desc.visible.unwrap_or(true),
            interactive: desc.interactive || desc.widget.is_some(),
            focus: desc.focus,
            safe: desc.safe,
            cur,
            prev: cur,
            rect: [0.0; 4],
            rect_prev: [0.0; 4],
            content: [0.0; 4],
            widget: None,
        };
        if index as usize == self.nodes.len() {
            self.nodes.push(node);
        } else {
            self.free.pop();
            self.nodes[index as usize] = node;
        }
        if let Some(name) = &desc.name {
            self.names.insert(name.clone(), index);
        }
        self.nodes[parent as usize].children.push(index);
        if desc.safe {
            self.place_safe(index);
        }
        Ok(index)
    }

    fn kind(&mut self, desc: &NodeDesc, label: &str) -> Result<(Kind, Color), UiError> {
        if let Some(text) = &desc.text {
            let font = match &desc.font {
                Some(f) => *self
                    .font_names
                    .get(f)
                    .ok_or_else(|| UiError::Unknown(format!("font {f:?} for node {label}")))?,
                None if self.fonts.len() == 1 => 0,
                None => {
                    return Err(UiError::Schema(format!(
                        "text node {label} names no font, and there is not exactly one"
                    )));
                }
            };
            let mut t = Text {
                text: String::new(),
                font,
                size: desc.size.unwrap_or(16.0),
                line_height: desc.line_height.unwrap_or(1.2),
                align: desc.text_align.unwrap_or_default(),
                max_lines: desc.max_lines.unwrap_or(u32::MAX),
                outline: desc.outline,
                shadow: desc.text_shadow,
                glyphs: Vec::new(),
                lines: Vec::new(),
                chars: 0,
            };
            self.shape(&mut t, text)?;
            return Ok((Kind::Text(Box::new(t)), Color::BLACK));
        }
        if let Some(name) = &desc.image {
            let image = *self
                .image_names
                .get(name)
                .ok_or_else(|| UiError::Unknown(format!("image {name:?} for node {label}")))?;
            let data = &self.images[image as usize];
            return Ok((
                Kind::Image(node::Image {
                    image,
                    region: desc.region.unwrap_or([
                        0.0,
                        0.0,
                        data.width as f32,
                        data.height as f32,
                    ]),
                    slice: desc.slice,
                    slice_scale: desc.slice_scale.unwrap_or(1.0),
                }),
                Color::WHITE,
            ));
        }
        if let Some(ring) = &desc.ring {
            return Ok((
                Kind::Ring(node::Ring {
                    width: ring.width,
                    start: ring.start,
                    track: ring.track,
                }),
                Color::WHITE,
            ));
        }
        if let Some(stroke) = &desc.stroke {
            return Ok((
                Kind::Stroke(node::Stroke {
                    points: stroke.points.clone(),
                    viewbox: stroke.viewbox,
                    width: stroke.width,
                }),
                Color::BLACK,
            ));
        }
        Ok((Kind::Box, Color::TRANSPARENT))
    }

    fn shape(&mut self, t: &mut Text, text: &str) -> Result<(), UiError> {
        let font = &self.fonts[t.font as usize];
        if !font.shape(
            t.font,
            text,
            &mut self.glyphs,
            &mut self.atlas,
            self.settings.sdf_px,
            &mut t.glyphs,
        ) {
            return Err(UiError::AtlasFull);
        }
        t.text.clear();
        t.text.push_str(text);
        t.chars = text.chars().count() as u32;
        Ok(())
    }

    fn attach_widgets(&mut self, index: u32, desc: &NodeDesc) -> Result<(), UiError> {
        if let Some(w) = &desc.widget {
            let find = |ui: &Ui, name: &str, role: &str| {
                ui.names.get(name).copied().ok_or_else(|| {
                    UiError::Unknown(format!(
                        "{role} node {name:?} of a {}",
                        match w {
                            WidgetDesc::Joystick(_) => "joystick",
                            WidgetDesc::Button(_) => "button",
                        }
                    ))
                })
            };
            let widget = match w {
                WidgetDesc::Joystick(j) => Widget::Joystick(Joystick {
                    base: find(self, &j.base, "base")?,
                    pin: find(self, &j.pin, "pin")?,
                    desc: j.clone(),
                    pointer: None,
                    center: [0.0; 2],
                    output: [0.0; 2],
                }),
                WidgetDesc::Button(b) => Widget::Button(Button {
                    face: match &b.face {
                        Some(f) => find(self, f, "face")?,
                        None => index,
                    },
                    rings: b
                        .ring
                        .iter()
                        .map(|r| find(self, r, "ring"))
                        .collect::<Result<_, _>>()?,
                    desc: b.clone(),
                    pointer: None,
                    held: false,
                }),
            };
            self.nodes[index as usize].widget = Some(widget);
            self.widgets.push(index);
        }
        let children = self.nodes[index as usize].children.clone();
        for (child, child_desc) in children.into_iter().zip(&desc.children) {
            self.attach_widgets(child, child_desc)?;
        }
        Ok(())
    }

    /// Removes a node and its descendants. The root cannot be removed.
    pub fn remove(&mut self, id: NodeId) -> Result<(), UiError> {
        let index = self.live(id)?;
        if index == self.root {
            return Err(UiError::Schema("the root cannot be removed".into()));
        }
        self.remove_index(index);
        Ok(())
    }

    fn remove_index(&mut self, index: u32) {
        let children = std::mem::take(&mut self.nodes[index as usize].children);
        for child in children {
            self.remove_index(child);
        }
        let node = &mut self.nodes[index as usize];
        self.taffy
            .remove(node.taffy)
            .expect("live nodes are in the tree");
        if let Some(name) = node.name.take() {
            self.names.remove(&name);
        }
        node.live = false;
        node.generation += 1;
        node.kind = Kind::Box;
        node.widget = None;
        let parent = node.parent.take();
        if let Some(p) = parent {
            self.nodes[p as usize].children.retain(|&c| c != index);
        }
        self.free.push(index);
        self.anims.retain(|a| a.node != index);
        self.widgets.retain(|&w| w != index);
        self.pointers.retain(|p| p.node != index);
        if self.focus == Some(index) {
            self.focus = None;
        }
        self.layout_dirty = true;
    }

    /// Sets a property at once, stopping any animation of it.
    pub fn set(&mut self, id: NodeId, prop: Prop, value: f32) -> Result<(), UiError> {
        let index = self.live(id)?;
        self.set_index(index, prop, value);
        Ok(())
    }

    pub(crate) fn set_index(&mut self, index: u32, prop: Prop, value: f32) {
        for a in &mut self.anims {
            if a.node == index && a.prop == prop as u8 && a.started {
                a.done = true;
            }
        }
        self.anims.retain(|a| !a.done);
        let node = &mut self.nodes[index as usize];
        node.cur[prop as usize] = value;
        node.prev[prop as usize] = value;
        if prop.lays_out() {
            self.sync_size(index);
        }
    }

    pub fn set_fill(&mut self, id: NodeId, color: Color) -> Result<(), UiError> {
        let index = self.live(id)?;
        for (p, v) in [Prop::Red, Prop::Green, Prop::Blue, Prop::Alpha]
            .into_iter()
            .zip(color.0)
        {
            self.set_index(index, p, v);
        }
        Ok(())
    }

    /// Fills a box with `gradient` instead of its fill, or with its fill
    /// again when `None`.
    pub fn set_gradient(&mut self, id: NodeId, gradient: Option<Gradient>) -> Result<(), UiError> {
        let index = self.live(id)?;
        if let Some(g) = &gradient {
            g.check().map_err(UiError::Schema)?;
        }
        self.nodes[index as usize].gradient = gradient;
        Ok(())
    }

    /// Lays out now and keeps the node and everything under it where they
    /// are until the next step, instead of moving them there from where
    /// they were when this step began.
    pub fn snap(&mut self, id: NodeId) -> Result<(), UiError> {
        let index = self.live(id)?;
        if self.layout_dirty {
            self.layout();
        }
        self.snap_index(index);
        Ok(())
    }

    fn snap_index(&mut self, index: u32) {
        let node = &mut self.nodes[index as usize];
        node.rect_prev = node.rect;
        for i in 0..node.children.len() {
            let child = self.nodes[index as usize].children[i];
            self.snap_index(child);
        }
    }

    pub fn set_visible(&mut self, id: NodeId, visible: bool) -> Result<(), UiError> {
        let index = self.live(id)?;
        self.nodes[index as usize].visible = visible;
        Ok(())
    }

    /// Replaces a text node's text; lays out again.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), UiError> {
        let index = self.live(id)?;
        let Kind::Text(mut t) = std::mem::replace(&mut self.nodes[index as usize].kind, Kind::Box)
        else {
            return Err(UiError::Schema(format!("node {} is not a text", index)));
        };
        let shaped = self.shape(&mut t, text);
        let node = &mut self.nodes[index as usize];
        node.kind = Kind::Text(t);
        self.taffy.mark_dirty(node.taffy).expect("live");
        self.layout_dirty = true;
        shaped
    }

    pub fn text(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[self.live(id).ok()? as usize].kind {
            Kind::Text(t) => Some(&t.text),
            _ => None,
        }
    }

    /// Characters of a text node its font has no glyph for; they draw as
    /// the font's missing-glyph box.
    pub fn missing_glyphs(&self, id: NodeId) -> Option<usize> {
        match &self.nodes[self.live(id).ok()? as usize].kind {
            Kind::Text(t) => Some(
                t.glyphs
                    .iter()
                    .filter(|g| g.glyph == 0 && g.kind == crate::text::GlyphKind::Ink)
                    .count(),
            ),
            _ => None,
        }
    }

    /// The lines of a text node after the last layout.
    pub fn lines(&self, id: NodeId) -> Option<&[Line]> {
        match &self.nodes[self.live(id).ok()? as usize].kind {
            Kind::Text(t) => Some(&t.lines),
            _ => None,
        }
    }

    pub fn value(&self, id: NodeId, prop: Prop) -> Option<f32> {
        Some(self.nodes[self.live(id).ok()? as usize].get(prop))
    }

    /// The node's layout box in UI units from the screen's top-left,
    /// without its transform: x, y, width, height.
    pub fn rect(&self, id: NodeId) -> Option<[f32; 4]> {
        let mut i = self.live(id).ok()?;
        let [_, _, w, h] = self.nodes[i as usize].rect;
        let (mut x, mut y) = (0.0, 0.0);
        loop {
            let n = &self.nodes[i as usize];
            x += n.rect[0];
            y += n.rect[1];
            match n.parent {
                Some(p) => i = p,
                None => return Some([x, y, w, h]),
            }
        }
    }

    pub fn animate(&mut self, id: NodeId, prop: Prop, animation: Animation) -> Result<(), UiError> {
        let index = self.live(id)?;
        self.push_anim(index, prop, animation)
    }

    fn push_anim(&mut self, index: u32, prop: Prop, a: Animation) -> Result<(), UiError> {
        if self.anims.len() >= self.settings.animations {
            return Err(UiError::Animations(self.settings.animations));
        }
        let typewriter = matches!(a.motion, Motion::Typewriter { .. });
        if typewriter
            && (prop != Prop::Reveal || !matches!(self.nodes[index as usize].kind, Kind::Text(_)))
        {
            return Err(UiError::Schema(
                "a typewriter animates the reveal of a text".into(),
            ));
        }
        let motion = if self.settings.reduced_motion && (prop.moves() || typewriter) {
            Motion::Tween {
                duration: 0.0,
                ease: Ease::LINEAR,
            }
        } else {
            a.motion
        };
        let to = if typewriter { f32::INFINITY } else { a.to };
        // Like CSS's fill-mode backwards: an explicit start value holds
        // through the delay, unless an earlier animation drives the property.
        if let Some(from) = a.from
            && !self
                .anims
                .iter()
                .any(|o| o.node == index && o.prop == prop as u8)
        {
            self.set_index(index, prop, from);
        }
        self.anims.push(Anim::new(
            index,
            prop as u8,
            a.from,
            to,
            motion,
            a.delay.max(0.0),
        ));
        Ok(())
    }

    /// Starts a named animation from the description or [`Ui::add_animation`].
    pub fn play(&mut self, name: &str) -> Result<(), UiError> {
        let tracks = self
            .animations
            .get(name)
            .ok_or_else(|| UiError::Unknown(format!("animation {name:?}")))?
            .clone();
        let mut end = 0.0f32;
        for t in &tracks {
            let base = if t.after { end } else { 0.0 };
            let nodes: Vec<u32> = t
                .node
                .iter()
                .chain(&t.nodes)
                .map(|n| {
                    self.names.get(n).copied().ok_or_else(|| {
                        UiError::Unknown(format!("node {n:?} in animation {name:?}"))
                    })
                })
                .collect::<Result<_, _>>()?;
            let motion = match (t.duration, t.spring, t.typewriter) {
                (Some(duration), _, _) => Motion::Tween {
                    duration,
                    ease: t.ease.unwrap_or(Ease::OUT),
                },
                (_, Some(s), _) => s.into(),
                (_, _, Some(w)) => Motion::Typewriter {
                    rate: w.rate,
                    pause: w.pause,
                },
                _ => unreachable!("checked in add_animation"),
            };
            for (k, &node) in nodes.iter().enumerate() {
                let delay = base + t.delay + t.stagger * k as f32;
                for (c, &prop) in t.prop.props().iter().enumerate() {
                    let pick = |v: Option<crate::desc::Value>| v.and_then(|v| v.get(c));
                    self.push_anim(
                        node,
                        prop,
                        Animation {
                            to: pick(t.to).unwrap_or(f32::INFINITY),
                            motion,
                            from: pick(t.from),
                            delay,
                        },
                    )?;
                }
                if let Some(d) = t.duration {
                    end = end.max(delay + d);
                }
            }
        }
        Ok(())
    }

    /// Ends every animation of a node at its target; a typewriter shows
    /// the whole text.
    pub fn finish(&mut self, id: NodeId) -> Result<(), UiError> {
        let index = self.live(id)?;
        let chars = match &self.nodes[index as usize].kind {
            Kind::Text(t) => t.chars as f32,
            _ => 0.0,
        };
        let mut ends = [None; PROPS];
        for a in &mut self.anims {
            if a.node == index {
                a.done = true;
                ends[a.prop as usize] = Some(match a.motion {
                    Motion::Typewriter { .. } => chars,
                    _ => a.to,
                });
            }
        }
        self.anims.retain(|a| !a.done);
        for (p, v) in Prop::ALL.into_iter().zip(ends) {
            if let Some(v) = v {
                self.set_index(index, p, v);
            }
        }
        Ok(())
    }

    /// Animations running or waiting to start.
    pub fn animating(&self) -> usize {
        self.anims.len()
    }

    /// Sets the screen size in physical pixels and its safe area. The
    /// reference size, turned to the screen's orientation, is scaled to fit;
    /// the longer side gets the extra room.
    pub fn set_screen(&mut self, width: f32, height: f32, safe: SafeArea) {
        let [a, b] = self.settings.reference;
        let (rw, rh) = if (width >= height) == (a >= b) {
            (a, b)
        } else {
            (b, a)
        };
        let scale = (width / rw).min(height / rh);
        self.screen = Screen {
            width,
            height,
            safe,
            scale,
        };
        let root = self.nodes[self.root as usize].taffy;
        let mut style = self.taffy.style(root).expect("root").clone();
        style.size = Size {
            width: Dimension::length(width / scale),
            height: Dimension::length(height / scale),
        };
        self.taffy.set_style(root, style).expect("root");
        for i in 0..self.nodes.len() as u32 {
            if self.nodes[i as usize].live && self.nodes[i as usize].safe {
                self.place_safe(i);
            }
        }
        self.layout_dirty = true;
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    fn place_safe(&mut self, index: u32) {
        let s = self.screen;
        let t = self.nodes[index as usize].taffy;
        let mut style = self.taffy.style(t).expect("live").clone();
        let unit = |px: f32| LengthPercentageAuto::length(px / s.scale);
        style.position = Position::Absolute;
        style.inset = Rect {
            left: unit(s.safe.left),
            right: unit(s.safe.right),
            top: unit(s.safe.top),
            bottom: unit(s.safe.bottom),
        };
        self.taffy.set_style(t, style).expect("live");
    }

    fn sync_size(&mut self, index: u32) {
        let node = &self.nodes[index as usize];
        let dim = |v: f32| {
            if v.is_nan() {
                Dimension::auto()
            } else {
                Dimension::length(v)
            }
        };
        let size = Size {
            width: dim(node.get(Prop::Width)),
            height: dim(node.get(Prop::Height)),
        };
        let t = node.taffy;
        let mut style = self.taffy.style(t).expect("live").clone();
        if style.size != size {
            style.size = size;
            self.taffy.set_style(t, style).expect("live");
            self.layout_dirty = true;
        }
    }

    /// Advances input and animation by one simulation step and returns
    /// what happened since the last step.
    pub fn step(&mut self) -> &[Event] {
        self.events.clear();
        self.events.append(&mut self.pending);
        for n in self.nodes.iter_mut().filter(|n| n.live) {
            n.prev = n.cur;
            n.rect_prev = n.rect;
        }
        self.advance_anims();
        self.step_widgets();
        if self.layout_dirty {
            self.layout();
        }
        &self.events
    }

    fn advance_anims(&mut self) {
        let dt = self.settings.step_s;
        let rest = SpringRest {
            hz: self.settings.spring_hz,
            delta: self.settings.spring_rest_delta,
            speed: self.settings.spring_rest_speed,
        };
        // Starts first, so an animation taken over this step does not also
        // move this step.
        for i in 0..self.anims.len() {
            let mut a = self.anims[i];
            if a.started || a.done {
                continue;
            }
            a.delay -= dt;
            if a.delay <= 1e-6 {
                self.start(i, &mut a);
            }
            self.anims[i] = a;
        }
        for i in 0..self.anims.len() {
            let mut a = self.anims[i];
            if !a.started || a.done {
                continue;
            }
            let t = if a.lead {
                a.lead = false;
                (-a.delay).max(0.0)
            } else {
                dt
            };
            let p = a.prop as usize;
            let node = &mut self.nodes[a.node as usize];
            let before = node.cur[p];
            let text = match &node.kind {
                Kind::Text(t) => t.text.as_str(),
                _ => "",
            };
            let (v, done) = anim::advance(&mut a, before, t, &rest, text);
            node.cur[p] = v;
            if !matches!(a.motion, Motion::Spring { .. }) && t > 0.0 {
                a.velocity = (v - before) / t;
            }
            a.done = done;
            self.anims[i] = a;
            if Prop::ALL[p].lays_out() {
                self.sync_size(a.node);
            }
        }
        self.anims.retain(|a| !a.done);
    }

    /// Takes over the property from any animation running on it, keeping its
    /// velocity, and fixes the start value.
    fn start(&mut self, i: usize, a: &mut Anim) {
        for (j, other) in self.anims.iter_mut().enumerate() {
            if j != i
                && other.started
                && !other.done
                && other.node == a.node
                && other.prop == a.prop
            {
                a.velocity = other.velocity;
                other.done = true;
            }
        }
        let p = a.prop as usize;
        let node = &mut self.nodes[a.node as usize];
        let mut start = a.from.unwrap_or(node.cur[p]);
        if let Motion::Typewriter { .. } = a.motion {
            let chars = match &node.kind {
                Kind::Text(t) => t.chars as f32,
                _ => 0.0,
            };
            if a.from.is_none() && start >= chars {
                start = 0.0;
            }
            a.to = chars;
        }
        if start.is_nan() {
            start = match Prop::ALL[p] {
                Prop::Width => node.rect[2],
                _ => node.rect[3],
            };
        }
        if start != node.cur[p] {
            node.cur[p] = start;
            node.prev[p] = start;
        }
        a.start_value = start;
        a.started = true;
        a.lead = true;
    }

    pub(crate) fn layout(&mut self) {
        let s = self.screen;
        let root = self.nodes[self.root as usize].taffy;
        let nodes = &self.nodes;
        self.taffy
            .compute_layout_with_measure(
                root,
                Size {
                    width: AvailableSpace::Definite(s.width / s.scale),
                    height: AvailableSpace::Definite(s.height / s.scale),
                },
                |inputs, _, context, style| match context {
                    Some(&mut i) => compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, available| measure(&nodes[i as usize], known, available),
                    ),
                    None => compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| Size::ZERO),
                },
            )
            .expect("the root is live");
        for n in self.nodes.iter_mut().filter(|n| n.live) {
            let l = self.taffy.layout(n.taffy).expect("live");
            n.rect = [l.location.x, l.location.y, l.size.width, l.size.height];
            let left = l.padding.left + l.border.left;
            let top = l.padding.top + l.border.top;
            n.content = [
                left,
                top,
                l.size.width - left - l.padding.right - l.border.right,
                l.size.height - top - l.padding.bottom - l.border.bottom,
            ];
            if n.fresh {
                n.rect_prev = n.rect;
                n.fresh = false;
            }
            if let Kind::Text(t) = &mut n.kind {
                break_lines(&t.glyphs, t.size, n.content[2], t.max_lines, &mut t.lines);
            }
        }
        self.layout_dirty = false;
    }

    /// The glyph atlas: side and one byte per texel.
    pub fn atlas(&self) -> (u32, &[u8]) {
        (self.atlas.size, &self.atlas.texels)
    }

    /// Whether glyphs were added since the last call.
    pub fn take_atlas_changed(&mut self) -> bool {
        std::mem::take(&mut self.atlas.dirty)
    }

    pub fn images(&self) -> &[ImageData] {
        &self.images
    }

    pub(crate) fn font(&self, index: u32) -> &Font {
        &self.fonts[index as usize]
    }
}

fn measure(node: &Node, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
    let Kind::Text(t) = &node.kind else {
        return Size::ZERO;
    };
    let max_width = known.width.unwrap_or(match available.width {
        AvailableSpace::Definite(w) => w,
        AvailableSpace::MinContent => 0.0,
        AvailableSpace::MaxContent => f32::INFINITY,
    });
    let mut lines = Vec::new();
    break_lines(&t.glyphs, t.size, max_width, t.max_lines, &mut lines);
    let widest = lines.iter().map(|l| l.width).fold(0.0, f32::max);
    Size {
        width: known.width.unwrap_or(widest),
        height: known
            .height
            .unwrap_or(lines.len() as f32 * t.size * t.line_height),
    }
}

fn check_settings(s: &UiSettings) -> Result<(), UiError> {
    let bad = |what: &str| Err(UiError::Schema(format!("settings: {what}")));
    let positive = |v: f32| v.is_finite() && v > 0.0;
    if !s.reference.iter().all(|&v| positive(v)) {
        return bad("reference must be two positive sizes");
    }
    if !positive(s.step_s) || !positive(s.spring_hz) {
        return bad("step_s and spring_hz must be positive");
    }
    if s.atlas_size == 0 || !positive(s.sdf_px) || !positive(s.sdf_spread) {
        return bad("atlas_size, sdf_px and sdf_spread must be positive");
    }
    Ok(())
}

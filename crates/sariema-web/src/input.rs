// SPDX-License-Identifier: Apache-2.0

//! The page's pointer, touch and keyboard events in the engine's terms.
//! Pointer Events carry mouse, pen and touch alike, each finger with its own
//! id; positions arrive in CSS pixels and leave in the physical pixels the
//! renderer and the UI use. Events wait in a queue for the next fixed step,
//! so every step sees the input of whole frames.

use sariema_ui::Ui;

/// A key by its place on the keyboard, the DOM's `KeyboardEvent.code`, so
/// that a layout or a shift key never changes what a game reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Space,
    Enter,
    Escape,
    Tab,
    Shift,
    /// `A` to `Z`.
    Letter(u8),
    /// `0` to `9` on the main row.
    Digit(u8),
}

impl Key {
    /// The key of a DOM `KeyboardEvent.code`; `None` for keys the engine
    /// does not read.
    pub fn from_code(code: &str) -> Option<Key> {
        let key = match code {
            "ArrowUp" => Key::Up,
            "ArrowDown" => Key::Down,
            "ArrowLeft" => Key::Left,
            "ArrowRight" => Key::Right,
            "Space" => Key::Space,
            "Enter" | "NumpadEnter" => Key::Enter,
            "Escape" => Key::Escape,
            "Tab" => Key::Tab,
            "ShiftLeft" | "ShiftRight" => Key::Shift,
            _ => {
                let byte = |prefix: &str| {
                    let rest = code.strip_prefix(prefix)?.as_bytes();
                    (rest.len() == 1).then(|| rest[0])
                };
                if let Some(c) = byte("Key").filter(u8::is_ascii_uppercase) {
                    Key::Letter(c)
                } else {
                    Key::Digit(byte("Digit").filter(u8::is_ascii_digit)? - b'0')
                }
            }
        };
        Some(key)
    }
}

/// One input event; positions in physical pixels from the canvas's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    PointerDown {
        id: u64,
        x: f32,
        y: f32,
    },
    PointerMove {
        id: u64,
        x: f32,
        y: f32,
    },
    PointerUp {
        id: u64,
        x: f32,
        y: f32,
    },
    /// The pointer was taken away, by a system gesture or the page losing focus.
    PointerCancel {
        id: u64,
    },
    KeyDown(Key),
    KeyUp(Key),
}

/// The events of the frames since the last step.
#[derive(Debug, Default)]
pub struct Input {
    queued: Vec<InputEvent>,
    /// Physical pixels per CSS pixel.
    scale: f32,
    shift: bool,
}

impl Input {
    pub fn new() -> Input {
        Input {
            scale: 1.0,
            ..Input::default()
        }
    }

    pub fn set_scale(&mut self, physical_per_css: f32) {
        self.scale = physical_per_css;
    }

    pub fn pointer(&mut self, kind: PointerKind, id: u64, css_x: f32, css_y: f32) {
        let (x, y) = (css_x * self.scale, css_y * self.scale);
        self.queued.push(match kind {
            PointerKind::Down => InputEvent::PointerDown { id, x, y },
            PointerKind::Move => InputEvent::PointerMove { id, x, y },
            PointerKind::Up => InputEvent::PointerUp { id, x, y },
            PointerKind::Cancel => InputEvent::PointerCancel { id },
        });
    }

    /// Queues a key by its DOM code; false for a key the engine does not read.
    pub fn key(&mut self, code: &str, down: bool) -> bool {
        let Some(key) = Key::from_code(code) else {
            return false;
        };
        self.queued.push(match down {
            true => InputEvent::KeyDown(key),
            false => InputEvent::KeyUp(key),
        });
        true
    }

    pub fn queued(&self) -> &[InputEvent] {
        &self.queued
    }

    /// Hands the queued events to `ui` and empties the queue: pointers as
    /// they are, Tab and Shift+Tab to move the focus, Enter and Space to
    /// activate what has it.
    pub fn deliver(&mut self, mut ui: Option<&mut Ui>) {
        for event in &self.queued {
            match *event {
                InputEvent::KeyDown(Key::Shift) => self.shift = true,
                InputEvent::KeyUp(Key::Shift) => self.shift = false,
                _ => {}
            }
            let Some(ui) = ui.as_deref_mut() else {
                continue;
            };
            match *event {
                InputEvent::PointerDown { id, x, y } => ui.pointer_down(id, x, y),
                InputEvent::PointerMove { id, x, y } => ui.pointer_move(id, x, y),
                InputEvent::PointerUp { id, x, y } => ui.pointer_up(id, x, y),
                InputEvent::PointerCancel { id } => ui.pointer_cancel(id),
                InputEvent::KeyDown(Key::Tab) if self.shift => ui.focus_prev(),
                InputEvent::KeyDown(Key::Tab) => ui.focus_next(),
                InputEvent::KeyDown(Key::Enter | Key::Space) => ui.activate(),
                _ => {}
            }
        }
        self.queued.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Down,
    Move,
    Up,
    Cancel,
}

impl PointerKind {
    /// The kind of a DOM pointer event type.
    pub fn from_dom(event_type: &str) -> Option<PointerKind> {
        match event_type {
            "pointerdown" => Some(PointerKind::Down),
            "pointermove" => Some(PointerKind::Move),
            "pointerup" => Some(PointerKind::Up),
            "pointercancel" => Some(PointerKind::Cancel),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_name_places_not_characters() {
        assert_eq!(Key::from_code("KeyW"), Some(Key::Letter(b'W')));
        assert_eq!(Key::from_code("Digit7"), Some(Key::Digit(7)));
        assert_eq!(Key::from_code("ArrowLeft"), Some(Key::Left));
        assert_eq!(Key::from_code("NumpadEnter"), Some(Key::Enter));
        for unread in ["", "Key", "KeyWW", "Keyw", "Digit", "F5", "MetaLeft"] {
            assert_eq!(Key::from_code(unread), None, "{unread}");
        }
    }

    #[test]
    fn pointers_arrive_in_physical_pixels() {
        let mut input = Input::new();
        input.set_scale(1.5);
        input.pointer(PointerKind::Down, 7, 100.0, 20.0);
        input.pointer(PointerKind::Cancel, 7, 0.0, 0.0);
        assert_eq!(
            input.queued(),
            [
                InputEvent::PointerDown {
                    id: 7,
                    x: 150.0,
                    y: 30.0
                },
                InputEvent::PointerCancel { id: 7 }
            ]
        );
        assert!(!input.key("F12", true));
        assert_eq!(input.queued().len(), 2);
        input.deliver(None);
        assert!(input.queued().is_empty());
    }

    #[test]
    fn shift_is_held_while_no_ui_is_loaded() {
        let mut input = Input::new();
        assert!(input.key("ShiftLeft", true));
        input.deliver(None);
        assert!(input.shift);
        assert!(input.key("ShiftLeft", false));
        input.deliver(None);
        assert!(!input.shift);
    }

    #[test]
    fn dom_pointer_types_map_to_kinds() {
        assert_eq!(PointerKind::from_dom("pointerup"), Some(PointerKind::Up));
        assert_eq!(
            PointerKind::from_dom("pointercancel"),
            Some(PointerKind::Cancel)
        );
        assert_eq!(PointerKind::from_dom("click"), None);
    }
}

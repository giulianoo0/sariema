// SPDX-License-Identifier: Apache-2.0

//! The spans of a `clip_path`: the stretches of its clip within which the
//! actor's root moves, as the command's `spans` bytes carry them.

use super::wire::{Ease, Sink};

/// Bytes of one span in a `clip_path`'s `spans`.
pub const CLIP_SPAN: usize = 34;

const MOVES: u8 = 1;
const TURNS: u8 = 2;

/// A stretch of a `clip_path`'s clip, `from` to `until` seconds of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipSpan {
    pub from: f32,
    pub until: f32,
    /// The clip's rate within the span: 0.4 plays it in slow motion there.
    pub rate: f32,
    /// Where the root goes over the span, from where it stands when the
    /// span begins; `None` leaves it there.
    pub to: Option<[f32; 3]>,
    /// The yaw, in degrees, it turns to over the span; `None` keeps it.
    pub yaw: Option<f32>,
    pub ease: Ease,
    /// Metres the root rises above the straight line halfway: a jump.
    pub arc: f32,
}

impl ClipSpan {
    /// Appends the span to a `clip_path`'s `spans`.
    pub fn put(&self, out: &mut impl Sink) {
        let flags =
            if self.to.is_some() { MOVES } else { 0 } | if self.yaw.is_some() { TURNS } else { 0 };
        out.put(&self.from.to_le_bytes());
        out.put(&self.until.to_le_bytes());
        out.put(&self.rate.to_le_bytes());
        out.put(&[flags, self.ease as u8]);
        for v in self.to.unwrap_or([0.0; 3]) {
            out.put(&v.to_le_bytes());
        }
        out.put(&self.yaw.unwrap_or(0.0).to_le_bytes());
        out.put(&self.arc.to_le_bytes());
    }

    /// The span [`ClipSpan::put`] wrote; `None` for an ease no curve has.
    pub fn read(bytes: &[u8; CLIP_SPAN]) -> Option<ClipSpan> {
        let f = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let flags = bytes[12];
        Some(ClipSpan {
            from: f(0),
            until: f(4),
            rate: f(8),
            ease: Ease::from_u8(bytes[13])?,
            to: (flags & MOVES != 0).then(|| [f(14), f(18), f(22)]),
            yaw: (flags & TURNS != 0).then(|| f(26)),
            arc: f(30),
        })
    }
}

/// The spans of a `clip_path`'s `spans`, each `None` when unreadable; `None`
/// when its length is not a whole number of spans.
pub fn clip_spans(bytes: &[u8]) -> Option<impl Iterator<Item = Option<ClipSpan>> + Clone + '_> {
    let (spans, rest) = bytes.as_chunks::<CLIP_SPAN>();
    rest.is_empty().then(|| spans.iter().map(ClipSpan::read))
}

// SPDX-License-Identifier: Apache-2.0

use sariema_format::script::{CLIP_SPAN, ClipSpan, Ease, clip_spans};

#[test]
fn clip_spans_read_back_what_was_put_and_refuse_a_torn_list() {
    let spans = [
        ClipSpan {
            from: 0.167,
            until: 0.3,
            rate: 1.0,
            to: Some([4.5, 0.5, -51.33]),
            yaw: None,
            ease: Ease::Linear,
            arc: 0.0,
        },
        ClipSpan {
            from: 0.433,
            until: 0.667,
            rate: 0.4,
            to: None,
            yaw: Some(-90.0),
            ease: Ease::InQuad,
            arc: 0.25,
        },
    ];
    let mut bytes = Vec::new();
    spans.iter().for_each(|s| s.put(&mut bytes));
    assert_eq!(bytes.len(), 2 * CLIP_SPAN);
    let read: Vec<_> = clip_spans(&bytes).unwrap().collect();
    assert_eq!(read, [Some(spans[0]), Some(spans[1])]);
    assert!(clip_spans(&bytes[1..]).is_none());
    bytes[13] = 200;
    assert_eq!(
        clip_spans(&bytes).unwrap().next(),
        Some(None),
        "an ease no curve has"
    );
}

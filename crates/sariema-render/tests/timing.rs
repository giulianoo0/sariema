// SPDX-License-Identifier: Apache-2.0

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use sariema_core::room_camera;
use sariema_format::RoomPack;
use sariema_format::demo::demo_room;
use sariema_render::{FrameTimings, Headless, Quality};

const W: u32 = 480;
const H: u32 = 270;

fn headless(room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, Quality::medium()).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

fn camera(room: &RoomPack) -> sariema_core::Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

#[test]
fn frame_stats_count_passes_draws_and_uploads() {
    let room = demo_room();
    let mut h = headless(&room);
    h.draw(&camera(&room), 0.0);
    let first = h.renderer().last_frame_stats();
    h.draw(&camera(&room), 0.0);
    let second = h.renderer().last_frame_stats();

    assert_eq!(
        first.passes,
        second.passes + 1,
        "the shadow atlas draws once"
    );
    assert_eq!(second.passes, second.post_passes + 1, "{second:?}");
    let visible = room.draws.iter().filter(|d| d.visible).count() as u32;
    let in_view = visible - second.draws_culled;
    // The background and one per post pass, besides the room's draws.
    assert_eq!(second.draws, 1 + in_view + second.post_passes, "{second:?}");
    assert!(first.draws > second.draws, "{first:?} {second:?}");
    assert!(first.bytes_uploaded > 0, "{first:?}");
    assert_eq!(second.bytes_uploaded, 0, "nothing changed: {second:?}");
}

#[test]
fn frames_read_no_clock_unless_given_one() {
    let room = demo_room();
    let mut h = headless(&room);
    h.draw(&camera(&room), 0.0);
    assert_eq!(h.renderer().last_frame_timings(), FrameTimings::default());
}

static READS: AtomicU32 = AtomicU32::new(0);

fn counting_clock() -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    READS.fetch_add(1, Ordering::Relaxed);
    ORIGIN.get_or_init(Instant::now).elapsed()
}

#[test]
fn timings_cover_the_frame_with_the_clock_given() {
    let room = demo_room();
    let mut h = headless(&room);
    h.renderer().time_cpu_stages(Some(counting_clock));
    h.draw(&camera(&room), 0.0);
    let t = h.renderer().last_frame_timings();
    let stages = [t.cull, t.bin, t.record, t.post, t.finish, t.submit];
    assert!(stages.iter().all(|s| !s.is_zero()), "{t:?}");
    assert!(READS.load(Ordering::Relaxed) > 0);

    h.renderer().time_cpu_stages(None);
    let reads = READS.load(Ordering::Relaxed);
    h.draw(&camera(&room), 0.0);
    assert_eq!(READS.load(Ordering::Relaxed), reads);
    assert_eq!(h.renderer().last_frame_timings(), FrameTimings::default());
}

#[test]
fn gpu_times_name_every_pass_of_every_timed_frame() {
    let room = demo_room();
    let mut h = headless(&room);
    let untimed = {
        h.draw(&camera(&room), 0.0);
        h.image()
    };
    if !h.renderer().time_gpu_passes(3) {
        assert!(h.renderer().gpu_pass_times().unwrap().is_empty());
        return;
    }
    for _ in 0..4 {
        h.draw(&camera(&room), 0.0);
    }
    assert_eq!(h.image(), untimed, "timing changes nothing drawn");
    let passes = h.renderer().last_frame_stats().passes as usize;
    let frames = h.renderer().gpu_pass_times().unwrap();
    assert_eq!(frames.len(), 3);
    for frame in &frames {
        assert_eq!(frame.len(), passes, "{frame:?}");
        assert_eq!(frame[0].label, "scene");
        assert_eq!(frame.last().unwrap().label, "composite");
        assert!(frame.iter().all(|p| p.end_ms >= p.start_ms), "{frame:?}");
    }
    assert_eq!(frames[0][0].start_ms, 0.0, "one origin for every frame");
    let ends: Vec<f64> = frames.iter().map(|f| f.last().unwrap().end_ms).collect();
    assert!(ends.is_sorted() && ends[0] < ends[2], "{ends:?}");
}

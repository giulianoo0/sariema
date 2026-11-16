// SPDX-License-Identifier: Apache-2.0

//! Audio on the web without threads or shared memory, which WebViews often
//! lack: the page's main thread mixes ahead with `Feeding::Manual` and posts
//! the blocks to an AudioWorklet, which plays them and reports how many
//! frames it has played. The mixer stays a lead ahead of what is heard.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeedSettings {
    /// Seconds mixed ahead of what the worklet has played: the latency of a
    /// sound started now, and how long the main thread may stall before the
    /// output runs dry. Default 0.1.
    pub lead_s: f32,
    /// Frames per block posted to the worklet, a multiple of its 128-frame
    /// quantum. Default 1024.
    pub block_frames: u32,
}

impl Default for FeedSettings {
    fn default() -> FeedSettings {
        FeedSettings {
            lead_s: 0.1,
            block_frames: 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Feed {
    settings: FeedSettings,
    sample_rate: u32,
    mixed: u64,
    played: u64,
}

impl Feed {
    pub fn new(settings: FeedSettings, sample_rate: u32) -> Feed {
        let quantum = 128;
        let settings = FeedSettings {
            block_frames: settings.block_frames.max(quantum).next_multiple_of(quantum),
            ..settings
        };
        Feed {
            settings,
            sample_rate,
            mixed: 0,
            played: 0,
        }
    }

    /// The mixer frame of a command to take effect as soon as it can: the
    /// next one to be mixed.
    pub fn now(&self) -> u64 {
        self.mixed
    }

    /// Takes the worklet's count of frames played; it never goes back.
    pub fn played(&mut self, frames: u64) {
        self.played = self.played.max(frames).min(self.mixed);
    }

    /// Blocks to mix now to stay `lead_s` ahead of what has been played.
    pub fn due_blocks(&self) -> u32 {
        let lead = (self.settings.lead_s.max(0.0) * self.sample_rate as f32).ceil() as u64;
        let queued = self.mixed - self.played;
        let block = self.settings.block_frames as u64;
        lead.saturating_sub(queued).div_ceil(block) as u32
    }

    pub fn block_frames(&self) -> u32 {
        self.settings.block_frames
    }

    /// Counts a block mixed and posted.
    pub fn mixed_block(&mut self) {
        self.mixed += self.settings.block_frames as u64;
    }

    /// Frames mixed and not yet played.
    pub fn queued(&self) -> u64 {
        self.mixed - self.played
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fills_the_lead_then_follows_what_is_played() {
        let mut feed = Feed::new(FeedSettings::default(), 48_000);
        // 0.1 s at 48 kHz is 4800 frames, five blocks of 1024.
        assert_eq!(feed.due_blocks(), 5);
        for _ in 0..5 {
            feed.mixed_block();
        }
        assert_eq!(feed.due_blocks(), 0);
        feed.played(1024);
        assert_eq!(feed.due_blocks(), 1);
        assert_eq!(feed.now(), 5 * 1024);
    }

    #[test]
    fn played_never_goes_back_nor_past_what_was_mixed() {
        let mut feed = Feed::new(FeedSettings::default(), 48_000);
        feed.mixed_block();
        feed.played(900);
        feed.played(100);
        assert_eq!(feed.queued(), 124);
        feed.played(5000);
        assert_eq!(feed.queued(), 0);
    }

    #[test]
    fn blocks_are_whole_quanta() {
        let odd = FeedSettings {
            block_frames: 1000,
            ..FeedSettings::default()
        };
        assert_eq!(Feed::new(odd, 44_100).block_frames(), 1024);
        let tiny = FeedSettings {
            block_frames: 1,
            ..FeedSettings::default()
        };
        assert_eq!(Feed::new(tiny, 44_100).block_frames(), 128);
    }
}

// SPDX-License-Identifier: Apache-2.0

//! What a frame costs: CPU time per stage of [`crate::Renderer::render`], and
//! GPU time per render pass from timestamp queries where the device has them.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use crate::RenderError;

/// CPU time of each stage of the last [`crate::Renderer::render`]. wgpu
/// records a pass's commands as they come and encodes them for the GPU in
/// `finish`, so most of a pass's cost lands there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTimings {
    /// Light culling and shadow slot assignment.
    pub cull: Duration,
    /// Light clustering and the frame's buffer writes.
    pub bin: Duration,
    /// Recording the shadow and scene passes.
    pub record: Duration,
    /// Recording the post-processing passes.
    pub post: Duration,
    /// `CommandEncoder::finish`.
    pub finish: Duration,
    /// `Queue::submit`.
    pub submit: Duration,
}

/// One render pass of one frame on the GPU, in milliseconds from the start
/// of the frame's first pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassTime {
    pub label: &'static str,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// Query sets cycled through by consecutive frames.
const RING: u32 = 4;
/// Frames between a frame and the one whose command buffer resolves its
/// queries. Metal tracks no hazard on the samples, so a resolve recorded
/// right after a pass can run before the pass's last sample is written.
const LAG: u32 = 2;

/// Timestamps at the start and end of every pass of a number of frames,
/// resolved into one buffer read once at the end.
pub(crate) struct GpuTimer {
    sets: Vec<wgpu::QuerySet>,
    resolved: wgpu::Buffer,
    /// Bytes per frame in `resolved`.
    stride: u64,
    passes_per_frame: u32,
    frames: u32,
    /// The frame being recorded; `frames` once all are.
    frame: Cell<u32>,
    labels: RefCell<Vec<Vec<&'static str>>>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, passes_per_frame: u32, frames: u32) -> GpuTimer {
        let queries = passes_per_frame * 2;
        let stride = (queries as u64 * size_of::<u64>() as u64)
            .next_multiple_of(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
        GpuTimer {
            sets: (0..RING)
                .map(|_| {
                    device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("pass timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count: queries,
                    })
                })
                .collect(),
            resolved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timestamps"),
                size: stride * frames as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            stride,
            passes_per_frame,
            frames,
            frame: Cell::new(0),
            labels: RefCell::new(vec![Vec::new(); frames as usize]),
        }
    }

    /// The timestamp writes of the frame's next pass; `None` once the frame
    /// has used its queries or every frame is timed.
    pub fn pass(&self, label: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let f = self.frame.get();
        let mut labels = self.labels.borrow_mut();
        let frame = labels.get_mut(f as usize)?;
        let k = frame.len() as u32;
        if k == self.passes_per_frame {
            return None;
        }
        frame.push(label);
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.sets[(f % RING) as usize],
            beginning_of_pass_write_index: Some(2 * k),
            end_of_pass_write_index: Some(2 * k + 1),
        })
    }

    /// Closes the frame, copying out the timestamps of the frame [`LAG`] before.
    pub fn end_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        let frame = self.frame.get();
        if frame >= self.frames {
            return;
        }
        if let Some(earlier) = frame.checked_sub(LAG) {
            self.resolve(encoder, earlier);
        }
        self.frame.set(frame + 1);
    }

    fn resolve(&self, encoder: &mut wgpu::CommandEncoder, frame: u32) {
        let passes = self.labels.borrow()[frame as usize].len() as u32;
        if passes > 0 {
            encoder.resolve_query_set(
                &self.sets[(frame % RING) as usize],
                0..passes * 2,
                &self.resolved,
                frame as u64 * self.stride,
            );
        }
    }

    /// The passes of every frame timed so far; waits for the GPU.
    pub fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<Vec<PassTime>>, RenderError> {
        let frames = self.frame.get();
        if frames == 0 {
            return Ok(Vec::new());
        }
        let wait = || {
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map(drop)
                .map_err(|e| RenderError::Device(e.to_string()))
        };
        wait()?;
        let size = frames as u64 * self.stride;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass timestamps read back"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        for frame in frames.saturating_sub(LAG)..frames {
            self.resolve(&mut encoder, frame);
        }
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &staging, 0, size);
        queue.submit([encoder.finish()]);
        staging.map_async(wgpu::MapMode::Read, .., |_| ());
        wait()?;
        let mapped = staging
            .get_mapped_range(..)
            .map_err(|e| RenderError::Device(e.to_string()))?;
        let ticks: Vec<u64> = mapped
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| u64::from_le_bytes(*b))
            .collect();
        let ms_per_tick = queue.get_timestamp_period() as f64 / 1e6;
        let per_frame = self.stride as usize / size_of::<u64>();
        let labels = self.labels.borrow();
        Ok((0..frames as usize)
            .map(|f| {
                let ticks = &ticks[f * per_frame..];
                let first = ticks[0];
                let ms = |t: u64| t.saturating_sub(first) as f64 * ms_per_tick;
                (labels[f].iter().enumerate())
                    .map(|(k, &label)| PassTime {
                        label,
                        start_ms: ms(ticks[2 * k]),
                        end_ms: ms(ticks[2 * k + 1].max(ticks[2 * k])),
                    })
                    .collect()
            })
            .collect())
    }
}

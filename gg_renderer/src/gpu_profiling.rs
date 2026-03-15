use ash::vk::{self, Handle};

use super::MAX_FRAMES_IN_FLIGHT;
use gg_core::error::{EngineError, EngineResult};

/// Maximum number of sequential timestamp markers per frame.
const MAX_TIMESTAMPS: u32 = 16;

/// A named GPU timing measurement result.
#[derive(Clone, Debug)]
pub struct GpuTimingResult {
    pub name: &'static str,
    pub time_ms: f32,
}

/// GPU timestamp query profiler using Vulkan timestamp queries.
///
/// Records sequential timestamps at key points in the render frame's command
/// buffer. Results are read back with 1-frame latency (after the fence wait
/// for the same frame-in-flight slot).
///
/// Call [`begin_frame`] after the fence wait, [`timestamp`] at each measurement
/// point during command recording, and query [`results`] for display.
pub struct GpuProfiler {
    device: ash::Device,
    query_pools: [vk::QueryPool; MAX_FRAMES_IN_FLIGHT],
    timestamp_period_ns: f32,

    // Per-frame recording state (indexed by current_frame).
    query_counts: [u32; MAX_FRAMES_IN_FLIGHT],
    query_names: [Vec<&'static str>; MAX_FRAMES_IN_FLIGHT],

    // Latest completed frame results.
    results: Vec<GpuTimingResult>,
    total_frame_ms: f32,

    enabled: bool,
}

impl GpuProfiler {
    pub fn new(device: &ash::Device, timestamp_period_ns: f32) -> EngineResult<Self> {
        let mut query_pools = [vk::QueryPool::null(); MAX_FRAMES_IN_FLIGHT];

        for pool in &mut query_pools {
            let create_info = vk::QueryPoolCreateInfo::default()
                .query_type(vk::QueryType::TIMESTAMP)
                .query_count(MAX_TIMESTAMPS);

            *pool = unsafe { device.create_query_pool(&create_info, None) }.map_err(|e| {
                EngineError::Gpu(format!("Failed to create GPU timestamp query pool: {e}"))
            })?;
        }

        Ok(Self {
            device: device.clone(),
            query_pools,
            timestamp_period_ns,
            query_counts: [0; MAX_FRAMES_IN_FLIGHT],
            query_names: std::array::from_fn(|_| Vec::new()),
            results: Vec::new(),
            total_frame_ms: 0.0,
            enabled: false,
        })
    }

    /// Enable or disable GPU profiling.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.results.clear();
            self.total_frame_ms = 0.0;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Begin a new frame. Reads back results from the previous use of this
    /// frame slot, then resets the query pool for new recording.
    ///
    /// Call AFTER `wait_for_fences` so the query pool data is valid.
    pub fn begin_frame(&mut self, cmd_buf: vk::CommandBuffer, current_frame: usize) {
        if !self.enabled {
            return;
        }

        let pool = self.query_pools[current_frame];
        let count = self.query_counts[current_frame];

        // Read back results from the previous frame that used this slot.
        if count >= 2 {
            let mut timestamps = vec![0u64; count as usize];
            let result = unsafe {
                self.device.get_query_pool_results(
                    pool,
                    0,
                    &mut timestamps,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )
            };

            if result.is_ok() {
                self.results.clear();
                let names = &self.query_names[current_frame];

                for i in 1..count as usize {
                    let delta = timestamps[i].wrapping_sub(timestamps[i - 1]);
                    let time_ms = delta as f64 * self.timestamp_period_ns as f64 / 1_000_000.0;
                    self.results.push(GpuTimingResult {
                        name: names.get(i - 1).copied().unwrap_or("unknown"),
                        time_ms: time_ms as f32,
                    });
                }

                let total_delta = timestamps[count as usize - 1].wrapping_sub(timestamps[0]);
                self.total_frame_ms =
                    (total_delta as f64 * self.timestamp_period_ns as f64 / 1_000_000.0) as f32;
            }
        }

        // Reset for new recording.
        self.query_counts[current_frame] = 0;
        self.query_names[current_frame].clear();

        unsafe {
            self.device
                .cmd_reset_query_pool(cmd_buf, pool, 0, MAX_TIMESTAMPS);
        }
    }

    /// Record a timestamp at the current point in the command buffer.
    /// The name labels the region BETWEEN this timestamp and the next one.
    pub fn timestamp(
        &mut self,
        cmd_buf: vk::CommandBuffer,
        current_frame: usize,
        name: &'static str,
    ) {
        if !self.enabled {
            return;
        }

        let idx = self.query_counts[current_frame];
        if idx >= MAX_TIMESTAMPS {
            return;
        }

        let pool = self.query_pools[current_frame];
        unsafe {
            self.device.cmd_write_timestamp(
                cmd_buf,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                pool,
                idx,
            );
        }

        self.query_names[current_frame].push(name);
        self.query_counts[current_frame] = idx + 1;
    }

    /// Get the timing results from the most recently completed frame.
    /// Each result represents the time between consecutive timestamps.
    pub fn results(&self) -> &[GpuTimingResult] {
        &self.results
    }

    /// Total GPU frame time in milliseconds (first timestamp to last).
    pub fn total_frame_ms(&self) -> f32 {
        self.total_frame_ms
    }

    fn destroy(&mut self) {
        for pool in &self.query_pools {
            if !pool.is_null() {
                unsafe {
                    self.device.destroy_query_pool(*pool, None);
                }
            }
        }
    }
}

impl Drop for GpuProfiler {
    fn drop(&mut self) {
        self.destroy();
    }
}

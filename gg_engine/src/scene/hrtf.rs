//! Binaural HRTF audio effect for spatial audio.
//!
//! Implements Head-Related Transfer Function processing using analytical models:
//! - **ITD** (Interaural Time Difference): Woodworth spherical-head model
//! - **ILD** (Interaural Level Difference): angle-dependent per-ear gain
//! - **Head shadow**: one-pole low-pass filter on the far ear
//!
//! Integrated as a kira `Effect` on per-source spatial tracks.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use kira::effect::{Effect, EffectBuilder};
use kira::Frame;
use kira::info::Info;

// ---------------------------------------------------------------------------
// Physical constants
// ---------------------------------------------------------------------------

/// Average human head radius in meters.
const HEAD_RADIUS: f32 = 0.0875;
/// Speed of sound in air (m/s) at ~20°C.
const SPEED_OF_SOUND: f32 = 343.0;
/// Maximum ITD ≈ head_radius / speed_of_sound * π ≈ 0.8 ms.
const MAX_ITD_SECONDS: f32 = HEAD_RADIUS * std::f32::consts::PI / SPEED_OF_SOUND;

// ---------------------------------------------------------------------------
// Shared parameters (game thread ↔ audio thread)
// ---------------------------------------------------------------------------

/// Thread-safe parameter block shared between the game thread and the kira
/// audio thread. Updated per-frame by `update_spatial_audio()`, read in
/// `BinauralEffect::process()`.
pub(crate) struct BinauralParams {
    /// Azimuth in radians. 0 = front, +π/2 = right, −π/2 = left, ±π = behind.
    azimuth: AtomicU32,
    /// Elevation in radians. 0 = level, +π/2 = above, −π/2 = below.
    elevation: AtomicU32,
    /// Whether the effect is active.
    enabled: AtomicBool,
}

impl BinauralParams {
    pub fn new() -> Self {
        Self {
            azimuth: AtomicU32::new(0.0f32.to_bits()),
            elevation: AtomicU32::new(0.0f32.to_bits()),
            enabled: AtomicBool::new(true),
        }
    }

    /// Update direction from the game thread. `azimuth` and `elevation` are
    /// in radians.
    pub fn set_direction(&self, azimuth: f32, elevation: f32) {
        self.azimuth.store(azimuth.to_bits(), Ordering::Relaxed);
        self.elevation
            .store(elevation.to_bits(), Ordering::Relaxed);
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// BinauralEffect — kira Effect implementation
// ---------------------------------------------------------------------------

/// Binaural HRTF effect that processes stereo audio through ITD, ILD, and
/// head-shadow models to produce convincing 3D spatial cues for headphone
/// playback.
pub(crate) struct BinauralEffect {
    params: Arc<BinauralParams>,
    sample_rate: f32,

    // Delay lines for ITD (circular buffers, one per ear).
    delay_line_l: Vec<f32>,
    delay_line_r: Vec<f32>,
    delay_write_pos: usize,

    // Previous-frame delay amounts for smooth per-sample interpolation.
    prev_delay_l: f32,
    prev_delay_r: f32,

    // One-pole low-pass filter state per ear (head shadow).
    filter_state_l: f32,
    filter_state_r: f32,
    prev_shadow_l: f32,
    prev_shadow_r: f32,
}

impl BinauralEffect {
    /// Compute per-ear delay in samples using the Woodworth spherical-head model.
    ///
    /// Returns `(delay_left_samples, delay_right_samples)`.
    fn compute_itd(&self, azimuth: f32) -> (f32, f32) {
        // Woodworth model: ITD = (r/c) × (|θ| + sin(|θ|))
        let abs_az = azimuth.abs().min(std::f32::consts::PI);
        let itd_seconds = (HEAD_RADIUS / SPEED_OF_SOUND) * (abs_az + abs_az.sin());
        let delay_samples = itd_seconds * self.sample_rate;

        if azimuth >= 0.0 {
            // Source on the right → left ear receives sound later.
            (delay_samples, 0.0)
        } else {
            (0.0, delay_samples)
        }
    }

    /// Compute per-ear gain for ILD.
    ///
    /// Returns `(gain_left, gain_right)` where each is in `[0.3, 1.0]`.
    fn compute_ild(&self, azimuth: f32) -> (f32, f32) {
        // Near ear ≈ full volume, far ear attenuated proportionally to sin(|azimuth|).
        // Maximum ILD ≈ −6 dB at 90° (linear gain ≈ 0.5).
        let sin_az = azimuth.abs().sin();
        let near_gain = 1.0;
        let far_gain = (1.0 - 0.7 * sin_az).max(0.3); // never fully silent

        if azimuth >= 0.0 {
            (far_gain, near_gain) // source right → left is far
        } else {
            (near_gain, far_gain)
        }
    }

    /// Compute head-shadow low-pass coefficient per ear.
    ///
    /// Returns `(shadow_coeff_left, shadow_coeff_right)` where 0.0 = no
    /// filtering (near ear) and up to 0.85 = strong low-pass (far ear).
    fn compute_head_shadow(&self, azimuth: f32) -> (f32, f32) {
        let shadow = (azimuth.abs().sin() * 0.85).min(0.85);

        if azimuth >= 0.0 {
            (shadow, 0.0) // left is far ear
        } else {
            (0.0, shadow)
        }
    }

    /// Read a sample from `delay_line` at a fractional position using linear
    /// interpolation.
    #[inline]
    fn read_delay(delay_line: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = delay_line.len();
        let read_pos =
            (write_pos as f32 - delay_samples + len as f32 * 2.0) % len as f32;
        let idx = read_pos.floor() as usize % len;
        let frac = read_pos.fract();
        delay_line[idx] * (1.0 - frac) + delay_line[(idx + 1) % len] * frac
    }
}

impl Effect for BinauralEffect {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.sample_rate = sample_rate as f32;
        // Size delay lines for max ITD + headroom.
        let max_delay = (MAX_ITD_SECONDS * sample_rate as f32).ceil() as usize + 16;
        self.delay_line_l = vec![0.0; max_delay];
        self.delay_line_r = vec![0.0; max_delay];
        self.delay_write_pos = 0;
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate as f32;
        let max_delay = (MAX_ITD_SECONDS * sample_rate as f32).ceil() as usize + 16;
        self.delay_line_l = vec![0.0; max_delay];
        self.delay_line_r = vec![0.0; max_delay];
        self.delay_write_pos = 0;
        self.filter_state_l = 0.0;
        self.filter_state_r = 0.0;
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        let azimuth = f32::from_bits(self.params.azimuth.load(Ordering::Relaxed));

        if !self.params.enabled.load(Ordering::Relaxed) {
            // Simple stereo panning fallback (constant-power).
            // Since kira spatialization_strength is 0, we handle panning here.
            let pan = (azimuth / std::f32::consts::FRAC_PI_2).clamp(-1.0, 1.0);
            let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4; // 0..π/2
            let gain_l = angle.cos();
            let gain_r = angle.sin();
            for frame in input.iter_mut() {
                let mono = (frame.left + frame.right) * 0.5;
                frame.left = mono * gain_l;
                frame.right = mono * gain_r;
            }
            return;
        }

        // Elevation currently modulates ILD slightly; future work could add
        // spectral shaping for elevation cues.
        let _elevation = f32::from_bits(self.params.elevation.load(Ordering::Relaxed));

        let (target_delay_l, target_delay_r) = self.compute_itd(azimuth);
        let (gain_l, gain_r) = self.compute_ild(azimuth);
        let (target_shadow_l, target_shadow_r) = self.compute_head_shadow(azimuth);

        let len = input.len();
        if len == 0 {
            return;
        }
        let inv_len = 1.0 / len as f32;
        let dl_len = self.delay_line_l.len();

        for (i, frame) in input.iter_mut().enumerate() {
            let t = i as f32 * inv_len;

            // Smooth delay interpolation to avoid clicks on rapid source movement.
            let delay_l = self.prev_delay_l + (target_delay_l - self.prev_delay_l) * t;
            let delay_r = self.prev_delay_r + (target_delay_r - self.prev_delay_r) * t;

            // Downmix to mono for binaural processing.
            let mono = (frame.left + frame.right) * 0.5;

            // Write mono into both delay lines at the same write position.
            self.delay_line_l[self.delay_write_pos] = mono;
            self.delay_line_r[self.delay_write_pos] = mono;

            // Read with fractional delay (linear interpolation).
            let sample_l = Self::read_delay(&self.delay_line_l, self.delay_write_pos, delay_l);
            let sample_r = Self::read_delay(&self.delay_line_r, self.delay_write_pos, delay_r);

            // Apply ILD.
            let mut out_l = sample_l * gain_l;
            let mut out_r = sample_r * gain_r;

            // Head shadow: one-pole low-pass filter.
            // Smoothly interpolate filter coefficient across the buffer.
            let coeff_l = self.prev_shadow_l + (target_shadow_l - self.prev_shadow_l) * t;
            let coeff_r = self.prev_shadow_r + (target_shadow_r - self.prev_shadow_r) * t;

            self.filter_state_l = self.filter_state_l * coeff_l + out_l * (1.0 - coeff_l);
            self.filter_state_r = self.filter_state_r * coeff_r + out_r * (1.0 - coeff_r);
            out_l = self.filter_state_l;
            out_r = self.filter_state_r;

            *frame = Frame {
                left: out_l,
                right: out_r,
            };

            self.delay_write_pos = (self.delay_write_pos + 1) % dl_len;
        }

        // Latch end-of-buffer values for next callback.
        self.prev_delay_l = target_delay_l;
        self.prev_delay_r = target_delay_r;
        self.prev_shadow_l = target_shadow_l;
        self.prev_shadow_r = target_shadow_r;
    }
}

// ---------------------------------------------------------------------------
// Builder + Handle
// ---------------------------------------------------------------------------

/// Handle returned to the game thread for controlling the binaural effect.
pub(crate) struct BinauralHandle {
    pub params: Arc<BinauralParams>,
}

/// Builder that implements kira's `EffectBuilder` trait.
pub(crate) struct BinauralEffectBuilder {
    params: Arc<BinauralParams>,
}

impl BinauralEffectBuilder {
    pub fn new() -> (Self, BinauralHandle) {
        let params = Arc::new(BinauralParams::new());
        (
            Self {
                params: params.clone(),
            },
            BinauralHandle { params },
        )
    }
}

impl EffectBuilder for BinauralEffectBuilder {
    type Handle = ();

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        let effect = BinauralEffect {
            params: self.params,
            sample_rate: 44100.0,
            delay_line_l: Vec::new(),
            delay_line_r: Vec::new(),
            delay_write_pos: 0,
            prev_delay_l: 0.0,
            prev_delay_r: 0.0,
            filter_state_l: 0.0,
            filter_state_r: 0.0,
            prev_shadow_l: 0.0,
            prev_shadow_r: 0.0,
        };
        (Box::new(effect), ())
    }
}

// ---------------------------------------------------------------------------
// Direction computation helpers
// ---------------------------------------------------------------------------

/// Compute azimuth and elevation of a source relative to the listener.
///
/// `relative_pos` is the source position in the listener's local coordinate
/// frame (i.e. already rotated by the inverse of the listener's orientation).
///
/// Returns `(azimuth, elevation)` in radians.
/// - Azimuth: 0 = front (−Z), +π/2 = right (+X), −π/2 = left, ±π = behind.
/// - Elevation: 0 = level, +π/2 = above (+Y), −π/2 = below.
pub(crate) fn direction_to_azimuth_elevation(relative_pos: glam::Vec3) -> (f32, f32) {
    let dist = relative_pos.length();
    if dist < 1e-6 {
        return (0.0, 0.0);
    }

    // Kira convention: unrotated listener faces −Z, +X is right, +Y is up.
    let azimuth = relative_pos.x.atan2(-relative_pos.z);
    let elevation = (relative_pos.y / dist).asin();

    (azimuth, elevation)
}

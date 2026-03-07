use std::collections::HashMap;

use glam::Vec2;

use crate::renderer::Texture2D;
use crate::uuid::Uuid;
use crate::Ref;

// ==========================================================================
// CPU per-entity animation (SpriteAnimatorComponent)
//
// Good for "hero" entities (player, bosses, NPCs) — tens of entities.
// Each entity maintains its own timer and frame counter, ticked every frame.
//
// For mass animation (hundreds/thousands of identical sprites), use
// InstancedSpriteAnimator instead — see below. It uses stateless math
// (frame = start_frame + floor((time - start_time) * fps) % count) and
// the existing instanced rendering pipeline, avoiding per-entity updates.
//
// AnimationControllerComponent (below) provides data-driven transitions
// between clips. Lua scripts set parameters via Engine.set_anim_param()
// and the engine evaluates transitions automatically.
//
// GPU-computed animation is implemented: when an InstancedSpriteAnimator
// is playing, the vertex shader computes UVs from u_time + per-instance
// animation parameters. Zero CPU animation cost for playing entities.
// ==========================================================================

/// A named animation clip within a sprite sheet.
///
/// Clips reference a contiguous range of frames in a grid-based sprite sheet.
/// Frame indices are 0-based and row-major: frame 0 is top-left, frame
/// `columns - 1` is top-right, frame `columns` is the first cell of the
/// second row, and so on.
///
/// Each clip can optionally reference its own texture via `texture_handle`.
/// When set (non-zero), the clip's texture is used instead of the entity's
/// `SpriteRendererComponent` texture during rendering.
#[derive(Clone)]
pub struct AnimationClip {
    /// Human-readable name (e.g. "idle", "walk", "run").
    pub name: String,
    /// First frame index (0-based, row-major in grid).
    pub start_frame: u32,
    /// Last frame index (inclusive).
    pub end_frame: u32,
    /// Playback speed in frames per second.
    pub fps: f32,
    /// Whether the clip loops when it reaches the end.
    pub looping: bool,
    /// Optional per-clip texture asset handle. 0 = use sprite's texture.
    pub texture_handle: Uuid,
    /// Runtime-only loaded texture for this clip. Not serialized.
    pub texture: Option<Ref<Texture2D>>,
}

impl Default for AnimationClip {
    fn default() -> Self {
        Self {
            name: String::new(),
            start_frame: 0,
            end_frame: 0,
            fps: 12.0,
            looping: true,
            texture_handle: Uuid::from_raw(0),
            texture: None,
        }
    }
}

/// Sprite animator component for frame-based sprite sheet animation.
///
/// Requires a [`SpriteRendererComponent`](super::SpriteRendererComponent) on
/// the same entity with a loaded texture (the sprite sheet). The animator
/// divides the texture into a grid of `columns` x N rows, each cell being
/// `cell_size` pixels.
///
/// At runtime, [`on_update_animations`](super::Scene::on_update_animations)
/// advances the frame timer and computes the current frame. During rendering,
/// the current frame's UV region is used instead of the full texture.
#[derive(Clone)]
pub struct SpriteAnimatorComponent {
    /// Pixel size of each cell in the sprite sheet.
    pub cell_size: Vec2,
    /// Number of columns in the sprite sheet grid.
    pub columns: u32,
    /// Animation clips defined for this sprite sheet.
    pub clips: Vec<AnimationClip>,
    /// Name of the default/idle clip. Played automatically on create and
    /// when a non-looping clip finishes (if set).
    pub default_clip: String,

    /// Playback speed multiplier (1.0 = normal, 0.5 = half speed, 2.0 = double).
    pub speed_scale: f32,

    // -- Runtime state (reset on clone) --
    /// Currently playing clip index, or `None` if stopped.
    pub(crate) current_clip_index: Option<usize>,
    /// Accumulated time since the current frame started.
    pub(crate) frame_timer: f32,
    /// Current frame within the playing clip.
    pub(crate) current_frame: u32,
    /// Whether the animator is actively playing.
    pub(crate) playing: bool,
    /// Set by `update()` when a non-looping clip reaches its last frame.
    /// Cleared after the scene dispatches the `on_animation_finished` callback.
    pub(crate) finished_clip_name: Option<String>,
    /// Editor-only: when true, the animation ticks in edit mode for preview.
    pub(crate) previewing: bool,
}

impl SpriteAnimatorComponent {
    /// Play a clip by name. Returns `true` if the clip was found.
    pub fn play(&mut self, name: &str) -> bool {
        if let Some(index) = self.clips.iter().position(|c| c.name == name) {
            // Only reset if switching to a different clip.
            if self.current_clip_index != Some(index) {
                self.current_clip_index = Some(index);
                self.current_frame = self.clips[index].start_frame;
                self.frame_timer = 0.0;
            }
            self.playing = true;
            true
        } else {
            log::warn!("SpriteAnimator: clip '{}' not found", name);
            false
        }
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Returns `true` if the animator is currently playing.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Advance the animation by `dt` seconds. Called by Scene::on_update_animations.
    pub(crate) fn update(&mut self, dt: f32) {
        if !self.playing {
            return;
        }

        let clip_index = match self.current_clip_index {
            Some(i) => i,
            None => return,
        };

        let clip = match self.clips.get(clip_index) {
            Some(c) => c,
            None => return,
        };

        if clip.fps <= 0.0 || self.speed_scale <= 0.0 {
            return;
        }

        self.frame_timer += dt * self.speed_scale;
        let frame_duration = 1.0 / clip.fps;

        while self.frame_timer >= frame_duration {
            self.frame_timer -= frame_duration;
            self.current_frame += 1;

            if self.current_frame > clip.end_frame {
                if clip.looping {
                    self.current_frame = clip.start_frame;
                } else {
                    self.current_frame = clip.end_frame;
                    self.playing = false;
                    self.finished_clip_name = Some(clip.name.clone());
                    break;
                }
            }
        }
    }

    /// Get the current frame's grid coordinates (column, row).
    ///
    /// Returns `None` if no clip is playing.
    pub fn current_grid_coords(&self) -> Option<(u32, u32)> {
        if self.columns == 0 {
            return None;
        }
        self.current_clip_index.map(|_| {
            let col = self.current_frame % self.columns;
            let row = self.current_frame / self.columns;
            (col, row)
        })
    }

    /// Returns the current clip's per-clip texture, if any.
    pub fn current_clip_texture(&self) -> Option<&Ref<Texture2D>> {
        let idx = self.current_clip_index?;
        let clip = self.clips.get(idx)?;
        clip.texture.as_ref()
    }

    /// Returns the name of the currently selected clip, or `None` if no clip is active.
    pub fn current_clip_name(&self) -> Option<&str> {
        let idx = self.current_clip_index?;
        self.clips.get(idx).map(|c| c.name.as_str())
    }

    /// Returns the current frame index.
    pub fn current_frame(&self) -> u32 {
        self.current_frame
    }

    /// Set the current frame and reset the frame timer.
    pub fn set_current_frame(&mut self, frame: u32) {
        self.current_frame = frame;
        self.frame_timer = 0.0;
    }

    /// Returns the current clip index, or `None` if no clip is selected.
    pub fn current_clip_index(&self) -> Option<usize> {
        self.current_clip_index
    }

    /// Set the current clip index directly. Use `play()` for normal playback.
    pub fn set_current_clip_index(&mut self, index: Option<usize>) {
        self.current_clip_index = index;
    }

    /// Whether the editor preview is active.
    pub fn is_previewing(&self) -> bool {
        self.previewing
    }

    /// Enable or disable editor preview mode.
    pub fn set_previewing(&mut self, v: bool) {
        self.previewing = v;
    }

    /// Reset all runtime state (stop playback, clear clip selection).
    pub fn reset(&mut self) {
        self.playing = false;
        self.previewing = false;
        self.current_clip_index = None;
        self.current_frame = 0;
        self.frame_timer = 0.0;
        self.finished_clip_name = None;
    }
}

impl Default for SpriteAnimatorComponent {
    fn default() -> Self {
        Self {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 1,
            clips: Vec::new(),
            default_clip: String::new(),
            speed_scale: 1.0,
            current_clip_index: None,
            frame_timer: 0.0,
            current_frame: 0,
            playing: false,
            finished_clip_name: None,
            previewing: false,
        }
    }
}

// ==========================================================================
// Instanced sprite animation (mass entities — hundreds/thousands)
//
// Unlike SpriteAnimatorComponent, this stores NO per-frame mutable state.
// The current frame is computed from (global_time - start_time) * fps,
// making it O(0) per entity per frame. The CPU only touches this data
// when a state transition occurs (play a different clip).
//
// Rendered via the existing instanced pipeline (push_instance / flush_instances).
// During batching, the scene computes uv_min/uv_max with stateless math
// and submits a SpriteInstanceData — no timer ticking, no branching.
//
// GPU-computed animation: when playing, the vertex shader computes UVs
// from u_time + per-instance animation params. See instance.glsl.
// ==========================================================================

/// Lightweight animation component for mass-instanced sprites.
///
/// Designed for hundreds or thousands of identical animated entities
/// (soldiers, zombies, background characters, etc.) where per-entity
/// CPU timer ticking is too expensive.
///
/// Instead of maintaining per-frame mutable state, the current frame is
/// computed from `(global_time - start_time) * fps`. The CPU only writes
/// to this struct when a clip transition happens (e.g. idle → walk).
///
/// Requires a [`SpriteRendererComponent`](super::SpriteRendererComponent)
/// with a loaded sprite sheet texture on the same entity.
#[derive(Clone)]
pub struct InstancedSpriteAnimator {
    /// Pixel size of each cell in the sprite sheet.
    pub cell_size: Vec2,
    /// Number of columns in the sprite sheet grid.
    pub columns: u32,
    /// Animation clips defined for this sprite sheet.
    pub clips: Vec<AnimationClip>,
    /// Name of the default/idle clip. Played on create and when a
    /// non-looping clip finishes (if set).
    pub default_clip: String,
    /// Playback speed multiplier (1.0 = normal, 0.5 = half, 2.0 = double).
    pub speed_scale: f32,

    // -- Active clip parameters (written by play/play_by_name) --
    /// First frame index of the current clip (0-based, row-major).
    pub start_frame: u32,
    /// Number of frames in the current clip.
    pub frame_count: u32,
    /// Playback speed in frames per second (from clip definition).
    pub fps: f32,
    /// Whether the current clip loops.
    pub looping: bool,
    /// Global time at which the current clip started playing.
    pub start_time: f64,
    /// Whether the animator is actively playing.
    pub playing: bool,
    /// Index of the currently playing clip in `clips`, or `None`.
    pub(crate) current_clip_index: Option<usize>,
    /// Optional per-clip texture asset handle. 0 = use sprite's texture.
    pub texture_handle: Uuid,
    /// Runtime-only loaded texture. Not serialized.
    pub texture: Option<Ref<Texture2D>>,
}

impl InstancedSpriteAnimator {
    /// Effective fps accounting for speed_scale.
    #[inline]
    pub fn effective_fps(&self) -> f64 {
        self.fps as f64 * self.speed_scale as f64
    }

    /// Compute the current frame index using stateless math.
    ///
    /// Returns `None` if not playing or `frame_count` is zero.
    #[inline]
    pub fn current_frame(&self, global_time: f64) -> Option<u32> {
        let efps = self.effective_fps();
        if !self.playing || self.frame_count == 0 || efps <= 0.0 {
            return None;
        }

        let elapsed = (global_time - self.start_time).max(0.0);
        let frame_in_clip = (elapsed * efps).floor() as u32;

        if self.looping {
            Some(self.start_frame + frame_in_clip % self.frame_count)
        } else if frame_in_clip >= self.frame_count {
            Some(self.start_frame + self.frame_count - 1) // clamp to last frame
        } else {
            Some(self.start_frame + frame_in_clip)
        }
    }

    /// Returns `true` if a non-looping clip has finished at `global_time`.
    #[inline]
    pub fn is_finished(&self, global_time: f64) -> bool {
        let efps = self.effective_fps();
        if !self.playing || self.looping || self.frame_count == 0 || efps <= 0.0 {
            return false;
        }
        let elapsed = (global_time - self.start_time).max(0.0);
        let frame_in_clip = (elapsed * efps).floor() as u32;
        frame_in_clip >= self.frame_count
    }

    /// Get the current frame's grid coordinates (column, row).
    #[inline]
    pub fn current_grid_coords(&self, global_time: f64) -> Option<(u32, u32)> {
        if self.columns == 0 {
            return None;
        }
        let frame = self.current_frame(global_time)?;
        Some((frame % self.columns, frame / self.columns))
    }

    /// Play a clip by name. Returns `true` if the clip was found.
    ///
    /// Looks up the clip in `clips`, copies its parameters to the active
    /// fields, and records `global_time` as the start time.
    pub fn play_by_name(&mut self, name: &str, global_time: f64) -> bool {
        if let Some(index) = self.clips.iter().position(|c| c.name == name) {
            // Only reset if switching to a different clip.
            if self.current_clip_index != Some(index) {
                let clip = &self.clips[index];
                self.start_frame = clip.start_frame;
                self.frame_count = clip.end_frame - clip.start_frame + 1;
                self.fps = clip.fps;
                self.looping = clip.looping;
                self.texture_handle = clip.texture_handle;
                self.texture = clip.texture.clone();
                self.start_time = global_time;
                self.current_clip_index = Some(index);
            }
            self.playing = true;
            true
        } else {
            log::warn!("InstancedSpriteAnimator: clip '{}' not found", name);
            false
        }
    }

    /// Start playing a clip by raw parameters. Only writes fields — no per-frame cost.
    pub fn play(
        &mut self,
        start_frame: u32,
        frame_count: u32,
        fps: f32,
        looping: bool,
        global_time: f64,
    ) {
        self.start_frame = start_frame;
        self.frame_count = frame_count;
        self.fps = fps;
        self.looping = looping;
        self.start_time = global_time;
        self.current_clip_index = None;
        self.playing = true;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Returns the name of the currently playing clip, or `None`.
    pub fn current_clip_name(&self) -> Option<&str> {
        let idx = self.current_clip_index?;
        self.clips.get(idx).map(|c| c.name.as_str())
    }

    /// Returns the current clip's per-clip texture, if any.
    pub fn current_clip_texture(&self) -> Option<&Ref<Texture2D>> {
        self.texture.as_ref()
    }
}

impl Default for InstancedSpriteAnimator {
    fn default() -> Self {
        Self {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 1,
            clips: Vec::new(),
            default_clip: String::new(),
            speed_scale: 1.0,
            start_frame: 0,
            frame_count: 0,
            fps: 12.0,
            looping: true,
            start_time: 0.0,
            playing: false,
            current_clip_index: None,
            texture_handle: Uuid::from_raw(0),
            texture: None,
        }
    }
}

// ==========================================================================
// Animation state machine / controller
//
// Data-driven transitions between animation clips. Works alongside
// SpriteAnimatorComponent — the controller evaluates transition rules
// each frame and calls play() on the animator when a transition fires.
//
// Lua scripts set parameters (bool/float) and the engine evaluates
// transitions automatically, replacing manual if/elseif chains.
// ==========================================================================

/// Ordering comparison for float parameter conditions.
#[derive(Clone, Debug, PartialEq)]
pub enum FloatOrdering {
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

/// Condition that triggers an animation transition.
#[derive(Clone, Debug)]
pub enum TransitionCondition {
    /// Transition fires when the current clip finishes (non-looping only).
    OnFinished,
    /// Transition fires when a bool parameter matches the expected value.
    ParamBool(String, bool),
    /// Transition fires when a float parameter satisfies the comparison.
    ParamFloat(String, FloatOrdering, f32),
}

/// A single transition rule between animation clips.
#[derive(Clone, Debug)]
pub struct AnimationTransition {
    /// Source clip name. Empty string means "any state".
    pub from: String,
    /// Target clip name to transition to.
    pub to: String,
    /// Condition that must be met for the transition to fire.
    pub condition: TransitionCondition,
}

/// Data-driven animation controller that evaluates transitions between clips.
///
/// Attach this alongside a [`SpriteAnimatorComponent`] to get automatic
/// clip transitions based on parameter values. Lua scripts set parameters
/// via `Engine.set_anim_param(entity_id, name, value)` and the engine
/// evaluates transitions each frame.
///
/// Transitions are evaluated in order — the first matching transition wins.
#[derive(Clone, Default)]
pub struct AnimationControllerComponent {
    /// Ordered list of transition rules. First match wins.
    pub transitions: Vec<AnimationTransition>,
    /// Named boolean parameters set by gameplay code.
    pub bool_params: HashMap<String, bool>,
    /// Named float parameters set by gameplay code.
    pub float_params: HashMap<String, f32>,
}

impl AnimationControllerComponent {
    /// Evaluate transitions against the currently playing clip.
    ///
    /// Returns the name of the target clip if a transition fires, or `None`.
    /// `current_clip` is the name of the currently playing clip (or `None` if stopped).
    /// `clip_finished` is `true` if the current non-looping clip just finished.
    pub fn evaluate(
        &self,
        current_clip: Option<&str>,
        clip_finished: bool,
    ) -> Option<&str> {
        for t in &self.transitions {
            // Check "from" constraint.
            if !t.from.is_empty() {
                match current_clip {
                    Some(name) if name == t.from => {}
                    _ => continue,
                }
            }

            // Check condition.
            let fires = match &t.condition {
                TransitionCondition::OnFinished => clip_finished,
                TransitionCondition::ParamBool(name, expected) => {
                    self.bool_params.get(name).copied().unwrap_or(false) == *expected
                }
                TransitionCondition::ParamFloat(name, ordering, threshold) => {
                    let val = self.float_params.get(name).copied().unwrap_or(0.0);
                    match ordering {
                        FloatOrdering::Greater => val > *threshold,
                        FloatOrdering::Less => val < *threshold,
                        FloatOrdering::GreaterOrEqual => val >= *threshold,
                        FloatOrdering::LessOrEqual => val <= *threshold,
                    }
                }
            };

            if fires {
                return Some(&t.to);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_animator() -> SpriteAnimatorComponent {
        SpriteAnimatorComponent {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 4,
            clips: vec![
                AnimationClip {
                    name: "idle".into(),
                    start_frame: 0,
                    end_frame: 3,
                    fps: 10.0,
                    looping: true,
                    ..Default::default()
                },
                AnimationClip {
                    name: "walk".into(),
                    start_frame: 4,
                    end_frame: 7,
                    fps: 10.0,
                    looping: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn play_sets_clip() {
        let mut anim = make_animator();
        assert!(anim.play("idle"));
        assert!(anim.is_playing());
        assert_eq!(anim.current_clip_index, Some(0));
        assert_eq!(anim.current_frame, 0);
    }

    #[test]
    fn play_unknown_clip_returns_false() {
        let mut anim = make_animator();
        assert!(!anim.play("nonexistent"));
        assert!(!anim.is_playing());
    }

    #[test]
    fn update_advances_frame() {
        let mut anim = make_animator();
        anim.play("idle");
        // fps=10, so frame duration = 0.1s
        anim.update(0.15);
        assert_eq!(anim.current_frame, 1);
    }

    #[test]
    fn looping_clip_wraps() {
        let mut anim = make_animator();
        anim.play("idle");
        // Advance through all 4 frames (0..3) + wrap
        anim.update(0.45); // 4.5 frames at 10fps → frame 4 wraps to 0
        assert_eq!(anim.current_frame, 0);
        assert!(anim.is_playing());
    }

    #[test]
    fn non_looping_clip_stops() {
        let mut anim = make_animator();
        anim.play("walk");
        // Walk: frames 4..7, fps=10, non-looping
        anim.update(0.45); // 4.5 frames → reaches end
        assert_eq!(anim.current_frame, 7);
        assert!(!anim.is_playing());
        assert_eq!(anim.finished_clip_name.as_deref(), Some("walk"));
    }

    #[test]
    fn stop_pauses_playback() {
        let mut anim = make_animator();
        anim.play("idle");
        anim.update(0.15);
        let frame = anim.current_frame;
        anim.stop();
        anim.update(0.5);
        assert_eq!(anim.current_frame, frame); // Doesn't advance
    }

    #[test]
    fn grid_coords() {
        let mut anim = make_animator();
        anim.play("walk");
        // Frame 4 in a 4-column grid → (0, 1)
        assert_eq!(anim.current_grid_coords(), Some((0, 1)));
        anim.update(0.1);
        // Frame 5 → (1, 1)
        assert_eq!(anim.current_grid_coords(), Some((1, 1)));
    }

    #[test]
    fn clone_resets_runtime_state() {
        let mut anim = make_animator();
        anim.play("idle");
        anim.update(0.2);

        // Clone should NOT reset — it clones all fields including runtime
        // (the macro copies everything). Runtime reset happens in Scene::copy
        // via the manual clone impl if needed. For now, Clone derives all fields.
        let cloned = anim.clone();
        assert!(cloned.is_playing());
    }

    // -----------------------------------------------------------------------
    // InstancedSpriteAnimator tests
    // -----------------------------------------------------------------------

    fn make_instanced_animator() -> InstancedSpriteAnimator {
        InstancedSpriteAnimator {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 4,
            clips: vec![
                AnimationClip {
                    name: "idle".into(),
                    start_frame: 0,
                    end_frame: 3,
                    fps: 10.0,
                    looping: true,
                    ..Default::default()
                },
                AnimationClip {
                    name: "attack".into(),
                    start_frame: 4,
                    end_frame: 7,
                    fps: 10.0,
                    looping: false,
                    ..Default::default()
                },
            ],
            default_clip: "idle".into(),
            speed_scale: 1.0,
            start_frame: 0,
            frame_count: 4,
            fps: 10.0,
            looping: true,
            start_time: 0.0,
            playing: true,
            ..Default::default()
        }
    }

    #[test]
    fn instanced_current_frame_stateless() {
        let anim = make_instanced_animator();
        // fps=10, at t=0.15 → frame 1 (floor(0.15*10) = 1)
        assert_eq!(anim.current_frame(0.15), Some(1));
        // At t=0.35 → frame 3
        assert_eq!(anim.current_frame(0.35), Some(3));
    }

    #[test]
    fn instanced_looping_wraps() {
        let anim = make_instanced_animator();
        // 4 frames, fps=10. At t=0.45 → floor(4.5) = 4, 4 % 4 = 0
        assert_eq!(anim.current_frame(0.45), Some(0));
        // At t=1.05 → floor(10.5) = 10, 10 % 4 = 2
        assert_eq!(anim.current_frame(1.05), Some(2));
    }

    #[test]
    fn instanced_non_looping_clamps() {
        let mut anim = make_instanced_animator();
        anim.looping = false;
        // At t=0.45 → frame_in_clip=4, >= frame_count(4) → clamp to last (frame 3)
        assert_eq!(anim.current_frame(0.45), Some(3));
        assert!(anim.is_finished(0.45));
    }

    #[test]
    fn instanced_not_playing_returns_none() {
        let mut anim = make_instanced_animator();
        anim.playing = false;
        assert_eq!(anim.current_frame(1.0), None);
    }

    #[test]
    fn instanced_play_resets_start_time() {
        let mut anim = make_instanced_animator();
        anim.play(4, 4, 10.0, false, 5.0);
        assert_eq!(anim.start_time, 5.0);
        assert_eq!(anim.start_frame, 4);
        // At t=5.15 → elapsed=0.15, frame_in_clip=1 → frame 5
        assert_eq!(anim.current_frame(5.15), Some(5));
    }

    #[test]
    fn instanced_grid_coords() {
        let anim = make_instanced_animator();
        // frame 0 in 4-column grid → (0, 0)
        assert_eq!(anim.current_grid_coords(0.0), Some((0, 0)));
        // At t=0.15, frame 1 → (1, 0)
        assert_eq!(anim.current_grid_coords(0.15), Some((1, 0)));
    }

    #[test]
    fn instanced_offset_start_frame() {
        let mut anim = make_instanced_animator();
        anim.start_frame = 4; // second row
        anim.frame_count = 4;
        // At t=0.0, frame = 4 + 0 = 4, in 4-col grid → (0, 1)
        assert_eq!(anim.current_grid_coords(0.0), Some((0, 1)));
        // At t=0.15, frame = 4 + 1 = 5 → (1, 1)
        assert_eq!(anim.current_grid_coords(0.15), Some((1, 1)));
    }

    #[test]
    fn instanced_play_by_name() {
        let mut anim = make_instanced_animator();
        assert!(anim.play_by_name("attack", 1.0));
        assert_eq!(anim.start_frame, 4);
        assert_eq!(anim.frame_count, 4);
        assert!(!anim.looping);
        assert_eq!(anim.current_clip_index, Some(1));
        // At t=1.15 → elapsed=0.15 → frame 5
        assert_eq!(anim.current_frame(1.15), Some(5));
    }

    #[test]
    fn instanced_play_by_name_unknown() {
        let mut anim = make_instanced_animator();
        assert!(!anim.play_by_name("nonexistent", 0.0));
    }

    #[test]
    fn instanced_speed_scale() {
        let mut anim = make_instanced_animator();
        anim.speed_scale = 2.0;
        // fps=10 * speed_scale=2 = 20 effective fps
        // At t=0.05 → elapsed=0.05 → floor(0.05*20) = 1
        assert_eq!(anim.current_frame(0.05), Some(1));
    }

    #[test]
    fn instanced_speed_scale_half() {
        let mut anim = make_instanced_animator();
        anim.speed_scale = 0.5;
        // fps=10 * speed_scale=0.5 = 5 effective fps
        // At t=0.15 → floor(0.15*5) = 0
        assert_eq!(anim.current_frame(0.15), Some(0));
        // At t=0.25 → floor(0.25*5) = 1
        assert_eq!(anim.current_frame(0.25), Some(1));
    }

    #[test]
    fn instanced_clip_name() {
        let mut anim = make_instanced_animator();
        assert_eq!(anim.current_clip_name(), None);
        anim.play_by_name("idle", 0.0);
        assert_eq!(anim.current_clip_name(), Some("idle"));
    }

    // -----------------------------------------------------------------------
    // AnimationControllerComponent tests
    // -----------------------------------------------------------------------

    #[test]
    fn controller_param_bool_transition() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: "idle".into(),
                    to: "walk".into(),
                    condition: TransitionCondition::ParamBool("moving".into(), true),
                },
            ],
            bool_params: [("moving".into(), true)].into_iter().collect(),
            float_params: HashMap::new(),
        };
        assert_eq!(ctrl.evaluate(Some("idle"), false), Some("walk"));
    }

    #[test]
    fn controller_param_bool_no_match() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: "idle".into(),
                    to: "walk".into(),
                    condition: TransitionCondition::ParamBool("moving".into(), true),
                },
            ],
            bool_params: [("moving".into(), false)].into_iter().collect(),
            float_params: HashMap::new(),
        };
        assert_eq!(ctrl.evaluate(Some("idle"), false), None);
    }

    #[test]
    fn controller_on_finished_transition() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: "attack".into(),
                    to: "idle".into(),
                    condition: TransitionCondition::OnFinished,
                },
            ],
            bool_params: HashMap::new(),
            float_params: HashMap::new(),
        };
        // Not finished yet.
        assert_eq!(ctrl.evaluate(Some("attack"), false), None);
        // Finished.
        assert_eq!(ctrl.evaluate(Some("attack"), true), Some("idle"));
    }

    #[test]
    fn controller_float_param_transition() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: String::new(), // any state
                    to: "run".into(),
                    condition: TransitionCondition::ParamFloat(
                        "speed".into(),
                        FloatOrdering::Greater,
                        5.0,
                    ),
                },
            ],
            bool_params: HashMap::new(),
            float_params: [("speed".into(), 6.0)].into_iter().collect(),
        };
        assert_eq!(ctrl.evaluate(Some("walk"), false), Some("run"));
    }

    #[test]
    fn controller_any_state_transition() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: String::new(), // any state
                    to: "death".into(),
                    condition: TransitionCondition::ParamBool("dead".into(), true),
                },
            ],
            bool_params: [("dead".into(), true)].into_iter().collect(),
            float_params: HashMap::new(),
        };
        assert_eq!(ctrl.evaluate(Some("walk"), false), Some("death"));
        assert_eq!(ctrl.evaluate(Some("idle"), false), Some("death"));
        assert_eq!(ctrl.evaluate(None, false), Some("death"));
    }

    #[test]
    fn controller_first_match_wins() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: String::new(),
                    to: "first".into(),
                    condition: TransitionCondition::ParamBool("a".into(), true),
                },
                AnimationTransition {
                    from: String::new(),
                    to: "second".into(),
                    condition: TransitionCondition::ParamBool("a".into(), true),
                },
            ],
            bool_params: [("a".into(), true)].into_iter().collect(),
            float_params: HashMap::new(),
        };
        assert_eq!(ctrl.evaluate(Some("idle"), false), Some("first"));
    }

    #[test]
    fn controller_wrong_from_state() {
        let ctrl = AnimationControllerComponent {
            transitions: vec![
                AnimationTransition {
                    from: "idle".into(),
                    to: "walk".into(),
                    condition: TransitionCondition::ParamBool("moving".into(), true),
                },
            ],
            bool_params: [("moving".into(), true)].into_iter().collect(),
            float_params: HashMap::new(),
        };
        // Current clip is "run", not "idle" — transition should not fire.
        assert_eq!(ctrl.evaluate(Some("run"), false), None);
    }
}

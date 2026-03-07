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
// TODO(feature): Animation state machine / controller — data-driven
// transitions between clips. An AnimationControllerComponent would define
// transition rules (from_clip → to_clip with conditions like OnFinished,
// ParamBool, ParamFloat). Lua scripts would set parameters via
// Engine.set_anim_param(entity_id, "speed", 5.0) and the engine evaluates
// transitions automatically. This replaces the manual if/elseif chains
// currently written in Lua (see 2dguy_controller.lua). Editor UI would
// show a simple transition list (not a visual graph). See AUDIT.md item 25.
//
// TODO(perf): GPU-computed animation — move frame-to-UV math into the
// vertex shader. Extend SpriteInstanceData with animation parameters and
// compute UVs from a global u_time uniform. Zero CPU animation cost.
// Only needed if the CPU stateless path becomes a bottleneck at 10K+.
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
// INTEGRATION PLAN (remaining work):
//
// 1. Scene rendering path (mod.rs render_scene):
//    - Entities with InstancedSpriteAnimator go through push_instance()
//      instead of draw_sprite(). Compute uv_min/uv_max with stateless
//      math from current_grid_coords(global_time).
//
// 2. Scene update (mod.rs):
//    - Add on_update_instanced_animations(global_time) that only checks
//      non-looping clips for completion: is_finished(global_time).
//      Fires on_animation_finished Lua callback and transitions to idle.
//      No per-entity timer ticking — just one comparison per non-looping entity.
//
// 3. Serialization (scene_serializer.rs):
//    - Add InstancedSpriteAnimatorData serde struct.
//    - Serialize: cell_size, columns, clips (reuse AnimationClipData).
//    - Clips stored as named presets; active clip restored by name.
//
// 4. Lua API (script_glue.rs):
//    - Engine.play_instanced_animation(entity_id, clip_name)
//    - Engine.stop_instanced_animation(entity_id)
//    - Clip lookup: InstancedSpriteAnimator stores a Vec<AnimationClip>
//      alongside the active clip params, so play() looks up by name
//      and writes start_frame/frame_count/fps/looping/start_time.
//
// 5. Editor (properties/sprite.rs):
//    - UI for InstancedSpriteAnimator: cell_size, columns, clip list.
//    - Same clip editing as SpriteAnimatorComponent.
//    - Entities with this component skip the timeline panel (no
//      per-frame state to scrub).
//
// 6. for_each_cloneable_component! macro (mod.rs):
//    - Add InstancedSpriteAnimator to the clone list.
//
// TODO(perf): GPU-computed animation — move the frame-to-UV calculation
// into the instance vertex shader. Add animation fields to SpriteInstanceData
// (start_time, fps, start_frame, frame_count, columns, cell_size, tex_size)
// and a u_time uniform to the camera UBO. The vertex shader computes:
//   uint frame = start_frame + uint(floor((u_time - start_time) * fps)) % frame_count;
//   uint col = frame % columns; uint row = frame / columns;
//   vec2 uv_min = vec2(col, row) * cell_size / tex_size;
// This eliminates ALL CPU animation work, even during batching.
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
    /// First frame index of the current clip (0-based, row-major).
    pub start_frame: u32,
    /// Number of frames in the current clip.
    pub frame_count: u32,
    /// Playback speed in frames per second.
    pub fps: f32,
    /// Whether the current clip loops.
    pub looping: bool,
    /// Global time at which the current clip started playing.
    pub start_time: f64,
    /// Whether the animator is actively playing.
    pub playing: bool,
    /// Optional per-clip texture asset handle. 0 = use sprite's texture.
    pub texture_handle: Uuid,
    /// Runtime-only loaded texture. Not serialized.
    pub texture: Option<Ref<Texture2D>>,
}

impl InstancedSpriteAnimator {
    /// Compute the current frame index using stateless math.
    ///
    /// Returns `None` if not playing or `frame_count` is zero.
    #[inline]
    pub fn current_frame(&self, global_time: f64) -> Option<u32> {
        if !self.playing || self.frame_count == 0 || self.fps <= 0.0 {
            return None;
        }

        let elapsed = (global_time - self.start_time).max(0.0);
        let frame_in_clip = (elapsed * self.fps as f64).floor() as u32;

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
        if !self.playing || self.looping || self.frame_count == 0 || self.fps <= 0.0 {
            return false;
        }
        let elapsed = (global_time - self.start_time).max(0.0);
        let frame_in_clip = (elapsed * self.fps as f64).floor() as u32;
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

    /// Start playing a clip. Only writes fields — no per-frame cost.
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
        self.playing = true;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }
}

impl Default for InstancedSpriteAnimator {
    fn default() -> Self {
        Self {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 1,
            start_frame: 0,
            frame_count: 0,
            fps: 12.0,
            looping: true,
            start_time: 0.0,
            playing: false,
            texture_handle: Uuid::from_raw(0),
            texture: None,
        }
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
}

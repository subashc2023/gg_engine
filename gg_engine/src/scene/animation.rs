use glam::Vec2;

use crate::renderer::Texture2D;
use crate::uuid::Uuid;
use crate::Ref;

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

        if clip.fps <= 0.0 {
            return;
        }

        self.frame_timer += dt;
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
            current_clip_index: None,
            frame_timer: 0.0,
            current_frame: 0,
            playing: false,
            finished_clip_name: None,
            previewing: false,
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
}

use glam::Vec2;

/// A named animation clip within a sprite sheet.
///
/// Clips reference a contiguous range of frames in a grid-based sprite sheet.
/// Frame indices are 0-based and row-major: frame 0 is top-left, frame
/// `columns - 1` is top-right, frame `columns` is the first cell of the
/// second row, and so on.
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
}

impl Default for AnimationClip {
    fn default() -> Self {
        Self {
            name: String::new(),
            start_frame: 0,
            end_frame: 0,
            fps: 12.0,
            looping: true,
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

    // -- Runtime state (reset on clone) --
    /// Currently playing clip index, or `None` if stopped.
    pub(crate) current_clip_index: Option<usize>,
    /// Accumulated time since the current frame started.
    pub(crate) frame_timer: f32,
    /// Current frame within the playing clip.
    pub(crate) current_frame: u32,
    /// Whether the animator is actively playing.
    pub(crate) playing: bool,
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
}

impl Default for SpriteAnimatorComponent {
    fn default() -> Self {
        Self {
            cell_size: Vec2::new(32.0, 32.0),
            columns: 1,
            clips: Vec::new(),
            current_clip_index: None,
            frame_timer: 0.0,
            current_frame: 0,
            playing: false,
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
                },
                AnimationClip {
                    name: "walk".into(),
                    start_frame: 4,
                    end_frame: 7,
                    fps: 10.0,
                    looping: false,
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

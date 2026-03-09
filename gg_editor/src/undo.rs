use std::collections::VecDeque;

use gg_engine::prelude::*;

/// A single undo/redo entry with a human-readable description.
pub(crate) struct UndoEntry {
    pub description: String,
    pub snapshot: String,
}

/// Undo/redo system with descriptions and pre-frame snapshot capture.
///
/// Supports three recording patterns:
/// 1. **Discrete**: `record(scene, desc)` — instant snapshot for one-shot operations.
/// 2. **Gesture**: `begin_edit(scene, desc)` / `end_edit()` — coalesced drag/text edits.
/// 3. **Pre-frame**: `capture_pre_frame(scene)` + `push_pre_frame(desc)` or
///    `begin_edit_from_pre_frame(desc)` — captures state before any modifications
///    so that discrete property changes (checkbox, combobox) and continuous edits
///    (drag values, text input) are both covered without per-widget undo calls.
pub(crate) struct UndoSystem {
    undo_stack: VecDeque<UndoEntry>,
    redo_stack: VecDeque<UndoEntry>,
    max_entries: usize,
    /// Pending "before" entry for gesture coalescing.
    pending_entry: Option<UndoEntry>,
    /// Whether a continuous edit gesture is currently in progress.
    editing_in_progress: bool,
    /// Pre-captured snapshot taken before any modifications in the current frame.
    pre_frame_snapshot: Option<String>,
}

impl UndoSystem {
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_entries: 100,
            pending_entry: None,
            editing_in_progress: false,
            pre_frame_snapshot: None,
        }
    }

    // -- Pre-frame snapshot API --

    /// Capture a pre-frame snapshot for potential undo recording.
    /// Called once per frame before any modifications happen.
    pub fn capture_pre_frame(&mut self, scene: &Scene) {
        self.pre_frame_snapshot = SceneSerializer::serialize_to_string(scene).ok();
    }

    /// Begin a continuous gesture using the pre-captured snapshot.
    /// Use this when a drag or text edit starts and you already captured
    /// the "before" state via `capture_pre_frame`.
    pub fn begin_edit_from_pre_frame(&mut self, description: impl Into<String>) {
        if self.editing_in_progress {
            return;
        }
        self.editing_in_progress = true;
        self.pending_entry = self.pre_frame_snapshot.take().map(|snapshot| UndoEntry {
            description: description.into(),
            snapshot,
        });
    }

    /// Push the pre-captured snapshot as a discrete undo entry.
    /// Use this for instantaneous changes detected after the fact
    /// (checkbox toggle, combobox selection).
    pub fn push_pre_frame(&mut self, description: impl Into<String>) {
        if let Some(snapshot) = self.pre_frame_snapshot.take() {
            self.push_undo(UndoEntry {
                description: description.into(),
                snapshot,
            });
        }
    }

    // -- Standard recording API --

    /// Begin a continuous edit gesture (drag, gizmo, text edit).
    ///
    /// Captures a "before" snapshot if one isn't already pending.
    /// No-op if already inside a gesture.
    pub fn begin_edit(&mut self, scene: &Scene, description: impl Into<String>) {
        if self.editing_in_progress {
            return;
        }
        self.editing_in_progress = true;
        self.pending_entry = SceneSerializer::serialize_to_string(scene)
            .ok()
            .map(|snapshot| UndoEntry {
                description: description.into(),
                snapshot,
            });
    }

    /// End a continuous edit gesture. Pushes the "before" snapshot
    /// to the undo stack and clears the redo stack.
    pub fn end_edit(&mut self) {
        if !self.editing_in_progress {
            return;
        }
        self.editing_in_progress = false;
        if let Some(entry) = self.pending_entry.take() {
            self.push_undo(entry);
        }
    }

    /// Cancel an in-progress edit gesture without pushing to the undo stack.
    /// Use when the edit should be discarded (e.g. stopping play mode).
    pub fn cancel_edit(&mut self) {
        self.editing_in_progress = false;
        self.pending_entry = None;
    }

    /// Record a discrete (instantaneous) edit.
    ///
    /// Captures the current state and pushes it to undo immediately.
    /// Clears the redo stack and any pre-frame snapshot.
    pub fn record(&mut self, scene: &Scene, description: impl Into<String>) {
        // Clear pre-frame snapshot to prevent double-recording.
        self.pre_frame_snapshot = None;
        if let Ok(snapshot) = SceneSerializer::serialize_to_string(scene) {
            self.push_undo(UndoEntry {
                description: description.into(),
                snapshot,
            });
        }
    }

    /// Pop the undo stack, push current state to redo, return restored scene.
    pub fn undo(&mut self, current_scene: &Scene) -> Option<Scene> {
        let entry = self.undo_stack.pop_back()?;
        // Push current state to redo (with cap enforcement).
        if let Ok(current_yaml) = SceneSerializer::serialize_to_string(current_scene) {
            self.redo_stack.push_back(UndoEntry {
                description: entry.description.clone(),
                snapshot: current_yaml,
            });
            if self.redo_stack.len() > self.max_entries {
                self.redo_stack.pop_front();
            }
        }
        self.restore_from_yaml(&entry.snapshot)
    }

    /// Pop the redo stack, push current state to undo, return restored scene.
    pub fn redo(&mut self, current_scene: &Scene) -> Option<Scene> {
        let entry = self.redo_stack.pop_back()?;
        // Push current state to undo (with cap enforcement).
        if let Ok(current_yaml) = SceneSerializer::serialize_to_string(current_scene) {
            self.undo_stack.push_back(UndoEntry {
                description: entry.description.clone(),
                snapshot: current_yaml,
            });
            if self.undo_stack.len() > self.max_entries {
                self.undo_stack.pop_front();
            }
        }
        self.restore_from_yaml(&entry.snapshot)
    }

    /// Clear both stacks and any pending gesture.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending_entry = None;
        self.editing_in_progress = false;
        self.pre_frame_snapshot = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Whether a continuous edit gesture is currently in progress.
    pub fn is_editing(&self) -> bool {
        self.editing_in_progress
    }

    /// Peek at the description of the next undo operation.
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.back().map(|e| e.description.as_str())
    }

    /// Peek at the description of the next redo operation.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.back().map(|e| e.description.as_str())
    }

    // -- Internal helpers --

    fn push_undo(&mut self, entry: UndoEntry) {
        self.undo_stack.push_back(entry);
        self.redo_stack.clear();
        // Enforce cap — O(1) with VecDeque.
        if self.undo_stack.len() > self.max_entries {
            self.undo_stack.pop_front();
        }
    }

    fn restore_from_yaml(&self, yaml: &str) -> Option<Scene> {
        let mut scene = Scene::new();
        if SceneSerializer::deserialize_from_string(&mut scene, yaml).is_ok() {
            Some(scene)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scene(name: &str) -> Scene {
        let mut scene = Scene::new();
        scene.create_entity_with_tag(name);
        scene
    }

    #[test]
    fn push_pop_undo() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let scene_b = make_scene("B");

        // Record state before changing from A to B.
        undo.record(&scene_a, "test");
        assert!(undo.can_undo());
        assert!(!undo.can_redo());

        // Undo: should restore scene A.
        let restored = undo.undo(&scene_b).unwrap();
        let entities = restored.each_entity_with_tag();
        assert!(entities.iter().any(|(_, n)| n == "A"));
        assert!(undo.can_redo());
    }

    #[test]
    fn redo_after_undo() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let scene_b = make_scene("B");

        undo.record(&scene_a, "test");
        let restored_a = undo.undo(&scene_b).unwrap();
        assert!(undo.can_redo());

        let restored_b = undo.redo(&restored_a).unwrap();
        let entities = restored_b.each_entity_with_tag();
        assert!(entities.iter().any(|(_, n)| n == "B"));
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let scene_b = make_scene("B");
        let scene_c = make_scene("C");

        undo.record(&scene_a, "first");
        let _restored = undo.undo(&scene_b);
        assert!(undo.can_redo());

        // New edit clears redo.
        undo.record(&scene_c, "second");
        assert!(!undo.can_redo());
    }

    #[test]
    fn max_cap_enforced() {
        let mut undo = UndoSystem::new();
        for i in 0..150 {
            let scene = make_scene(&format!("Entity{}", i));
            undo.record(&scene, "test");
        }
        assert_eq!(undo.undo_stack.len(), 100);
    }

    #[test]
    fn begin_end_coalescing() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Before");

        undo.begin_edit(&scene, "drag");
        // Multiple changes would happen here...
        undo.end_edit();

        // Only one undo entry.
        assert_eq!(undo.undo_stack.len(), 1);
        assert_eq!(undo.undo_stack[0].description, "drag");

        // Undo restores "Before" state.
        let current = make_scene("After");
        let restored = undo.undo(&current).unwrap();
        let entities = restored.each_entity_with_tag();
        assert!(entities.iter().any(|(_, n)| n == "Before"));
    }

    #[test]
    fn clear_resets_everything() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("X");
        undo.record(&scene, "test");
        assert!(undo.can_undo());
        undo.clear();
        assert!(!undo.can_undo());
        assert!(!undo.can_redo());
    }

    #[test]
    fn begin_edit_noop_when_already_editing() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Test");
        undo.begin_edit(&scene, "first");
        undo.begin_edit(&scene, "second"); // No-op.
        undo.end_edit();
        assert_eq!(undo.undo_stack.len(), 1);
        assert_eq!(undo.undo_stack[0].description, "first");
    }

    #[test]
    fn descriptions_accessible() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let scene_b = make_scene("B");

        assert!(undo.undo_description().is_none());
        assert!(undo.redo_description().is_none());

        undo.record(&scene_a, "Move entity");
        assert_eq!(undo.undo_description(), Some("Move entity"));

        let _restored = undo.undo(&scene_b);
        assert_eq!(undo.redo_description(), Some("Move entity"));
    }

    #[test]
    fn pre_frame_push() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Before");

        undo.capture_pre_frame(&scene);
        // Simulate a discrete change (checkbox toggle).
        undo.push_pre_frame("Toggle checkbox");

        assert_eq!(undo.undo_stack.len(), 1);
        assert_eq!(undo.undo_stack[0].description, "Toggle checkbox");

        // Second push is a no-op (snapshot consumed).
        undo.push_pre_frame("Should not appear");
        assert_eq!(undo.undo_stack.len(), 1);
    }

    #[test]
    fn pre_frame_begin_edit() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Before");

        undo.capture_pre_frame(&scene);
        undo.begin_edit_from_pre_frame("Drag value");
        assert!(undo.is_editing());

        undo.end_edit();
        assert_eq!(undo.undo_stack.len(), 1);
        assert_eq!(undo.undo_stack[0].description, "Drag value");
    }

    #[test]
    fn record_clears_pre_frame() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Before");

        undo.capture_pre_frame(&scene);
        // record() should clear pre-frame to prevent double-recording.
        undo.record(&scene, "Add component");
        undo.push_pre_frame("Should not appear");

        assert_eq!(undo.undo_stack.len(), 1);
        assert_eq!(undo.undo_stack[0].description, "Add component");
    }
}

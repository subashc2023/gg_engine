use std::collections::VecDeque;

use gg_engine::prelude::*;

/// Snapshot-based undo/redo system using YAML scene serialization.
///
/// Each entry is a complete YAML string snapshot of the scene state.
/// Gestures (drags, gizmo transforms) are coalesced via `begin_edit()`
/// and `end_edit()` bracketing so they produce a single undo step.
pub(crate) struct UndoSystem {
    undo_stack: VecDeque<String>,
    redo_stack: Vec<String>,
    max_entries: usize,
    /// "Before" snapshot captured at the start of a continuous gesture.
    pending_before: Option<String>,
    /// Whether a continuous edit gesture is currently in progress.
    editing_in_progress: bool,
}

impl UndoSystem {
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_entries: 100,
            pending_before: None,
            editing_in_progress: false,
        }
    }

    /// Begin a continuous edit gesture (drag, gizmo, text edit).
    ///
    /// Captures a "before" snapshot if one isn't already pending.
    /// No-op if already inside a gesture.
    pub fn begin_edit(&mut self, scene: &Scene) {
        if self.editing_in_progress {
            return;
        }
        self.editing_in_progress = true;
        self.pending_before = SceneSerializer::serialize_to_string(scene).ok();
    }

    /// End a continuous edit gesture. Pushes the "before" snapshot
    /// to the undo stack and clears the redo stack.
    pub fn end_edit(&mut self) {
        if !self.editing_in_progress {
            return;
        }
        self.editing_in_progress = false;
        if let Some(snapshot) = self.pending_before.take() {
            self.push_undo(snapshot);
        }
    }

    /// Record a discrete (instantaneous) edit.
    ///
    /// Captures the current state and pushes it to undo immediately.
    /// Clears the redo stack.
    pub fn record(&mut self, scene: &Scene) {
        if let Ok(snapshot) = SceneSerializer::serialize_to_string(scene) {
            self.push_undo(snapshot);
        }
    }

    /// Pop the undo stack, push current state to redo, return restored scene.
    pub fn undo(&mut self, current_scene: &Scene) -> Option<Scene> {
        let snapshot = self.undo_stack.pop_back()?;
        // Push current state to redo.
        if let Ok(current_yaml) = SceneSerializer::serialize_to_string(current_scene) {
            self.redo_stack.push(current_yaml);
        }
        self.restore_from_yaml(&snapshot)
    }

    /// Pop the redo stack, push current state to undo, return restored scene.
    pub fn redo(&mut self, current_scene: &Scene) -> Option<Scene> {
        let snapshot = self.redo_stack.pop()?;
        // Push current state to undo (with cap enforcement).
        if let Ok(current_yaml) = SceneSerializer::serialize_to_string(current_scene) {
            self.undo_stack.push_back(current_yaml);
            if self.undo_stack.len() > self.max_entries {
                self.undo_stack.pop_front();
            }
        }
        self.restore_from_yaml(&snapshot)
    }

    /// Clear both stacks and any pending gesture.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending_before = None;
        self.editing_in_progress = false;
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

    // -- Internal helpers --

    fn push_undo(&mut self, snapshot: String) {
        self.undo_stack.push_back(snapshot);
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
        undo.record(&scene_a);
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

        undo.record(&scene_a);
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

        undo.record(&scene_a);
        let _restored = undo.undo(&scene_b);
        assert!(undo.can_redo());

        // New edit clears redo.
        undo.record(&scene_c);
        assert!(!undo.can_redo());
    }

    #[test]
    fn max_cap_enforced() {
        let mut undo = UndoSystem::new();
        for i in 0..150 {
            let scene = make_scene(&format!("Entity{}", i));
            undo.record(&scene);
        }
        assert_eq!(undo.undo_stack.len(), 100);
    }

    #[test]
    fn begin_end_coalescing() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Before");

        undo.begin_edit(&scene);
        // Multiple changes would happen here...
        undo.end_edit();

        // Only one undo entry.
        assert_eq!(undo.undo_stack.len(), 1);

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
        undo.record(&scene);
        assert!(undo.can_undo());
        undo.clear();
        assert!(!undo.can_undo());
        assert!(!undo.can_redo());
    }

    #[test]
    fn begin_edit_noop_when_already_editing() {
        let mut undo = UndoSystem::new();
        let scene = make_scene("Test");
        undo.begin_edit(&scene);
        undo.begin_edit(&scene); // No-op.
        undo.end_edit();
        assert_eq!(undo.undo_stack.len(), 1);
    }
}

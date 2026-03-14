use std::collections::VecDeque;

use gg_engine::log;
use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// The data captured for an undo/redo entry.
pub(crate) enum UndoSnapshot {
    /// Full scene serialized as JSON (structural changes: create/delete entity,
    /// reparent, reorder, multi-entity ops).
    FullScene(String),
    /// Single entity serialized as JSON (property edits on one entity).
    /// Much smaller and faster to capture/restore than a full scene.
    Entity { uuid: u64, json: String },
}

/// A single undo/redo entry with a human-readable description.
pub(crate) struct UndoEntry {
    pub description: String,
    pub snapshot: UndoSnapshot,
}

/// Result of an undo/redo operation.
pub(crate) enum UndoAction {
    /// Entire scene was replaced — caller should swap to the returned scene.
    SceneRestored(Scene),
    /// A single entity was updated in-place — caller should refresh handles.
    EntityRestored(u64),
}

// ---------------------------------------------------------------------------
// Undo system
// ---------------------------------------------------------------------------

/// Undo/redo system with descriptions, pre-frame snapshot capture, and
/// entity-scoped snapshots for property edits.
///
/// Supports three recording patterns:
/// 1. **Discrete**: `record(scene, desc)` — full-scene JSON snapshot for
///    one-shot structural operations.
/// 2. **Gesture**: `begin_edit(scene, desc)` / `end_edit()` — coalesced
///    drag/text edits with full-scene snapshot.
/// 3. **Pre-frame**: `capture_pre_frame_entity(scene, entity)` +
///    `push_pre_frame(desc)` or `begin_edit_from_pre_frame(desc)` — captures
///    a single entity's state before any modifications so that discrete
///    property changes (checkbox, combobox) and continuous edits (drag
///    values, text input) are both covered without per-widget undo calls.
///
/// All snapshots use JSON serialization (~5-10× faster than YAML).
/// Entity-scoped snapshots avoid serializing the entire scene for property
/// edits, which is the most frequent undo operation.
pub(crate) struct UndoSystem {
    undo_stack: VecDeque<UndoEntry>,
    redo_stack: VecDeque<UndoEntry>,
    max_entries: usize,
    /// Pending "before" entry for gesture coalescing.
    pending_entry: Option<UndoEntry>,
    /// Whether a continuous edit gesture is currently in progress.
    editing_in_progress: bool,
    /// Pre-captured snapshot taken before any modifications in the current frame.
    pre_frame_snapshot: Option<UndoSnapshot>,
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

    /// Capture a pre-frame entity snapshot for potential undo recording.
    ///
    /// Called once per frame before any property modifications happen.
    /// Only serializes the single selected entity (not the entire scene).
    pub fn capture_pre_frame_entity(&mut self, scene: &Scene, entity: Entity) {
        if let Some(id) = scene.get_component::<IdComponent>(entity) {
            let uuid = id.id.raw();
            if let Ok(json) = SceneSerializer::serialize_entity_to_json(scene, entity) {
                self.pre_frame_snapshot = Some(UndoSnapshot::Entity { uuid, json });
            }
        }
    }

    /// Capture a pre-frame full-scene snapshot for potential undo recording.
    ///
    /// Use when no specific entity is targeted (e.g., multi-entity operations).
    #[allow(dead_code)]
    pub fn capture_pre_frame(&mut self, scene: &Scene) {
        self.pre_frame_snapshot = SceneSerializer::serialize_scene_to_json(scene)
            .ok()
            .map(UndoSnapshot::FullScene);
    }

    /// Begin a continuous gesture using the pre-captured snapshot.
    /// Use this when a drag or text edit starts and you already captured
    /// the "before" state via `capture_pre_frame_entity`.
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
    /// Captures a full-scene "before" snapshot if one isn't already pending.
    /// No-op if already inside a gesture.
    pub fn begin_edit(&mut self, scene: &Scene, description: impl Into<String>) {
        if self.editing_in_progress {
            return;
        }
        self.editing_in_progress = true;
        self.pending_entry = SceneSerializer::serialize_scene_to_json(scene)
            .ok()
            .map(|json| UndoEntry {
                description: description.into(),
                snapshot: UndoSnapshot::FullScene(json),
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

    /// Record a discrete (instantaneous) edit with a full-scene snapshot.
    ///
    /// Captures the current state and pushes it to undo immediately.
    /// Clears the redo stack and any pre-frame snapshot.
    pub fn record(&mut self, scene: &Scene, description: impl Into<String>) {
        // Clear pre-frame snapshot to prevent double-recording.
        self.pre_frame_snapshot = None;
        if let Ok(json) = SceneSerializer::serialize_scene_to_json(scene) {
            self.push_undo(UndoEntry {
                description: description.into(),
                snapshot: UndoSnapshot::FullScene(json),
            });
        }
    }

    /// Pop the undo stack and restore the snapshot.
    ///
    /// For full-scene snapshots, returns `SceneRestored` with a new scene.
    /// For entity snapshots, modifies the entity in-place and returns
    /// `EntityRestored`.
    pub fn undo(&mut self, current_scene: &mut Scene) -> Option<UndoAction> {
        let entry = self.undo_stack.pop_back()?;
        match entry.snapshot {
            UndoSnapshot::FullScene(ref json) => {
                // Save current scene for redo.
                if let Ok(current_json) = SceneSerializer::serialize_scene_to_json(current_scene) {
                    self.push_redo(UndoEntry {
                        description: entry.description.clone(),
                        snapshot: UndoSnapshot::FullScene(current_json),
                    });
                }
                self.restore_full_scene(json)
                    .map(UndoAction::SceneRestored)
            }
            UndoSnapshot::Entity { uuid, ref json } => {
                // Save current entity state for redo.
                if let Some(entity) = current_scene.find_entity_by_uuid(uuid) {
                    if let Ok(current_json) =
                        SceneSerializer::serialize_entity_to_json(current_scene, entity)
                    {
                        self.push_redo(UndoEntry {
                            description: entry.description.clone(),
                            snapshot: UndoSnapshot::Entity {
                                uuid,
                                json: current_json,
                            },
                        });
                    }
                }
                // Restore entity in-place.
                if SceneSerializer::restore_entity_from_json(current_scene, uuid, json).is_ok() {
                    Some(UndoAction::EntityRestored(uuid))
                } else {
                    // Entity not found or restore failed — fall back by re-pushing.
                    log::warn!("Entity undo failed for UUID {}, skipping", uuid);
                    None
                }
            }
        }
    }

    /// Pop the redo stack and restore the snapshot.
    pub fn redo(&mut self, current_scene: &mut Scene) -> Option<UndoAction> {
        let entry = self.redo_stack.pop_back()?;
        match entry.snapshot {
            UndoSnapshot::FullScene(ref json) => {
                // Save current scene for undo.
                if let Ok(current_json) = SceneSerializer::serialize_scene_to_json(current_scene) {
                    self.undo_stack.push_back(UndoEntry {
                        description: entry.description.clone(),
                        snapshot: UndoSnapshot::FullScene(current_json),
                    });
                    if self.undo_stack.len() > self.max_entries {
                        self.undo_stack.pop_front();
                    }
                }
                self.restore_full_scene(json)
                    .map(UndoAction::SceneRestored)
            }
            UndoSnapshot::Entity { uuid, ref json } => {
                // Save current entity state for undo.
                if let Some(entity) = current_scene.find_entity_by_uuid(uuid) {
                    if let Ok(current_json) =
                        SceneSerializer::serialize_entity_to_json(current_scene, entity)
                    {
                        self.undo_stack.push_back(UndoEntry {
                            description: entry.description.clone(),
                            snapshot: UndoSnapshot::Entity {
                                uuid,
                                json: current_json,
                            },
                        });
                        if self.undo_stack.len() > self.max_entries {
                            self.undo_stack.pop_front();
                        }
                    }
                }
                // Restore entity in-place.
                if SceneSerializer::restore_entity_from_json(current_scene, uuid, json).is_ok() {
                    Some(UndoAction::EntityRestored(uuid))
                } else {
                    log::warn!("Entity redo failed for UUID {}, skipping", uuid);
                    None
                }
            }
        }
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
        if self.undo_stack.len() > self.max_entries {
            self.undo_stack.pop_front();
        }
    }

    fn push_redo(&mut self, entry: UndoEntry) {
        self.redo_stack.push_back(entry);
        if self.redo_stack.len() > self.max_entries {
            self.redo_stack.pop_front();
        }
    }

    fn restore_full_scene(&self, json: &str) -> Option<Scene> {
        let mut scene = Scene::new();
        if SceneSerializer::deserialize_scene_from_json(&mut scene, json).is_ok() {
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
        let mut scene_b = make_scene("B");

        // Record state before changing from A to B.
        undo.record(&scene_a, "test");
        assert!(undo.can_undo());
        assert!(!undo.can_redo());

        // Undo: should restore scene A.
        match undo.undo(&mut scene_b).unwrap() {
            UndoAction::SceneRestored(restored) => {
                let entities = restored.each_entity_with_tag();
                assert!(entities.iter().any(|(_, n)| n == "A"));
            }
            _ => panic!("Expected SceneRestored"),
        }
        assert!(undo.can_redo());
    }

    #[test]
    fn redo_after_undo() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let mut scene_b = make_scene("B");

        undo.record(&scene_a, "test");
        let mut restored_a = match undo.undo(&mut scene_b).unwrap() {
            UndoAction::SceneRestored(s) => s,
            _ => panic!("Expected SceneRestored"),
        };
        assert!(undo.can_redo());

        match undo.redo(&mut restored_a).unwrap() {
            UndoAction::SceneRestored(restored_b) => {
                let entities = restored_b.each_entity_with_tag();
                assert!(entities.iter().any(|(_, n)| n == "B"));
            }
            _ => panic!("Expected SceneRestored"),
        }
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut undo = UndoSystem::new();
        let scene_a = make_scene("A");
        let mut scene_b = make_scene("B");
        let scene_c = make_scene("C");

        undo.record(&scene_a, "first");
        let _restored = undo.undo(&mut scene_b);
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
        let mut current = make_scene("After");
        match undo.undo(&mut current).unwrap() {
            UndoAction::SceneRestored(restored) => {
                let entities = restored.each_entity_with_tag();
                assert!(entities.iter().any(|(_, n)| n == "Before"));
            }
            _ => panic!("Expected SceneRestored"),
        }
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
        let mut scene_b = make_scene("B");

        assert!(undo.undo_description().is_none());
        assert!(undo.redo_description().is_none());

        undo.record(&scene_a, "Move entity");
        assert_eq!(undo.undo_description(), Some("Move entity"));

        let _restored = undo.undo(&mut scene_b);
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

    #[test]
    fn entity_scoped_pre_frame() {
        let mut undo = UndoSystem::new();
        let mut scene = make_scene("TestEntity");

        // Get the entity.
        let entities = scene.each_entity_with_tag();
        let (entity, _) = entities[0];

        // Capture entity-scoped pre-frame.
        undo.capture_pre_frame_entity(&scene, entity);

        // Simulate a property change: modify the entity's tag.
        if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
            tc.tag = "Modified".to_string();
        }

        // Push the pre-frame snapshot.
        undo.push_pre_frame("Change tag");
        assert_eq!(undo.undo_stack.len(), 1);

        // Undo should restore the entity in-place.
        match undo.undo(&mut scene).unwrap() {
            UndoAction::EntityRestored(uuid) => {
                // Entity should be restored with original tag.
                let entity = scene.find_entity_by_uuid(uuid).unwrap();
                let tag = scene.get_component::<TagComponent>(entity).unwrap();
                assert_eq!(tag.tag, "TestEntity");
            }
            _ => panic!("Expected EntityRestored"),
        }
    }

    #[test]
    fn entity_scoped_redo() {
        let mut undo = UndoSystem::new();
        let mut scene = make_scene("Original");

        let entities = scene.each_entity_with_tag();
        let (entity, _) = entities[0];

        // Capture before change.
        undo.capture_pre_frame_entity(&scene, entity);

        // Modify.
        if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
            tc.tag = "Changed".to_string();
        }
        undo.push_pre_frame("Rename");

        // Undo: restores "Original".
        let uuid = match undo.undo(&mut scene).unwrap() {
            UndoAction::EntityRestored(uuid) => uuid,
            _ => panic!("Expected EntityRestored"),
        };
        let entity = scene.find_entity_by_uuid(uuid).unwrap();
        assert_eq!(
            scene.get_component::<TagComponent>(entity).unwrap().tag,
            "Original"
        );

        // Redo: restores "Changed".
        match undo.redo(&mut scene).unwrap() {
            UndoAction::EntityRestored(uuid) => {
                let entity = scene.find_entity_by_uuid(uuid).unwrap();
                assert_eq!(
                    scene.get_component::<TagComponent>(entity).unwrap().tag,
                    "Changed"
                );
            }
            _ => panic!("Expected EntityRestored"),
        }
    }
}

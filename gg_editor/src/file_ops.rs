use std::path::{Path, PathBuf};

use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::WireframeMode;

use super::panels::Tab;
use super::{panels, EditorMode, GGEditor, SceneState};

impl GGEditor {
    /// Returns true if it's safe to discard the current scene (either not dirty,
    /// or the user confirmed). Shows a native dialog when the scene has unsaved changes.
    pub(super) fn confirm_discard_changes(&self) -> bool {
        if !self.scene_ctx.dirty {
            return true;
        }
        gg_engine::platform_utils::confirm_dialog(
            "Unsaved Changes",
            "The current scene has unsaved changes. Discard them?",
        )
    }

    /// Shared menu bar contents used by both the custom title bar (Windows/Linux)
    /// and the native menu bar (macOS).
    pub(super) fn menu_bar_contents(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui
                .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                .clicked()
            {
                if self.playback.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.new_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                .clicked()
            {
                if self.playback.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.open_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                .clicked()
            {
                if self.playback.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.save_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                .clicked()
            {
                if self.playback.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.save_scene_as();
                ui.close();
            }
            ui.separator();
            if ui.add(egui::Button::new("Open Project...")).clicked() {
                self.open_project();
                ui.close();
            }
        });
        let in_edit_mode = self.playback.scene_state == SceneState::Edit;
        ui.menu_button("Edit", |ui| {
            let undo_label = if let Some(desc) = self.undo_system.undo_description() {
                format!("Undo: {}", desc)
            } else {
                "Undo".to_string()
            };
            let redo_label = if let Some(desc) = self.undo_system.redo_description() {
                format!("Redo: {}", desc)
            } else {
                "Redo".to_string()
            };
            if ui
                .add_enabled(
                    in_edit_mode && self.undo_system.can_undo(),
                    egui::Button::new(undo_label).shortcut_text("Ctrl+Z"),
                )
                .clicked()
            {
                self.perform_undo();
                ui.close();
            }
            if ui
                .add_enabled(
                    in_edit_mode && self.undo_system.can_redo(),
                    egui::Button::new(redo_label).shortcut_text("Ctrl+Y"),
                )
                .clicked()
            {
                self.perform_redo();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    in_edit_mode && !self.selection.is_empty(),
                    egui::Button::new("Copy").shortcut_text("Ctrl+C"),
                )
                .clicked()
            {
                self.on_copy_entity();
                ui.close();
            }
            if ui
                .add_enabled(
                    in_edit_mode && !self.ui.clipboard_entity_uuids.is_empty(),
                    egui::Button::new("Paste").shortcut_text("Ctrl+V"),
                )
                .clicked()
            {
                self.on_paste_entity();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    in_edit_mode && !self.selection.is_empty(),
                    egui::Button::new("Duplicate").shortcut_text("Ctrl+D"),
                )
                .clicked()
            {
                self.on_duplicate_entity();
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            if ui
                .checkbox(&mut self.editor_settings.show_grid, "X-Y Grid")
                .clicked()
            {
                ui.close();
            }
            if ui
                .checkbox(&mut self.editor_settings.show_xz_grid, "X-Z Grid")
                .clicked()
            {
                ui.close();
            }
            ui.separator();
            ui.menu_button("Wireframe", |ui| {
                if ui
                    .radio_value(&mut self.ui.wireframe_mode, WireframeMode::Off, "Off")
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .radio_value(
                        &mut self.ui.wireframe_mode,
                        WireframeMode::WireOnly,
                        "Wireframe",
                    )
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .radio_value(
                        &mut self.ui.wireframe_mode,
                        WireframeMode::Overlay,
                        "Shaded Wireframe",
                    )
                    .clicked()
                {
                    ui.close();
                }
            });
            ui.separator();
            let mut game_vp = self.viewport.game_viewport_enabled;
            if ui.checkbox(&mut game_vp, "Game Viewport").clicked() {
                self.toggle_game_viewport();
                ui.close();
            }
            ui.separator();
            if ui.button("Reset Layout").clicked() {
                self.ui.dock_state = Self::default_dock_layout();
                ui.close();
            }
        });
        #[cfg(feature = "lua-scripting")]
        ui.menu_button("Script", |ui| {
            if ui
                .add(egui::Button::new("Reload Scripts").shortcut_text("Ctrl+R"))
                .clicked()
            {
                self.scene.reload_lua_scripts();
                panels::properties::clear_field_cache();
                ui.close();
            }
        });
        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard Shortcuts").clicked() {
                self.ui.show_shortcuts_dialog = true;
                ui.close();
            }
        });
    }

    pub(super) fn toggle_game_viewport(&mut self) {
        self.viewport.game_viewport_enabled = !self.viewport.game_viewport_enabled;
        if self.viewport.game_viewport_enabled {
            // Defer framebuffer creation to on_render (needs renderer access).
            if self.viewport.game_fb.is_none() {
                self.ui.create_game_fb = true;
            }
            // Add the Game tab next to the Viewport tab if not already present.
            if self.ui.dock_state.find_tab(&Tab::GameViewport).is_none() {
                if let Some((surface, node, _)) = self.ui.dock_state.find_tab(&Tab::Viewport) {
                    self.ui.dock_state[surface][node].append_tab(Tab::GameViewport);
                } else {
                    self.ui.dock_state.push_to_first_leaf(Tab::GameViewport);
                }
            }
        } else {
            // Remove the Game tab from the dock.
            if let Some((surface, node, tab)) = self.ui.dock_state.find_tab(&Tab::GameViewport) {
                self.ui.dock_state[surface][node].remove_tab(tab);
            }
        }
    }

    pub(super) fn new_scene(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if self.project_state.project.is_some() {
            // Show naming modal — scene will be created on confirm.
            self.ui.new_scene_modal = Some("New Scene".into());
        } else {
            // No project — just create an empty unnamed scene.
            self.create_empty_scene();
        }
    }

    fn create_empty_scene(&mut self) {
        let old = std::mem::replace(&mut self.scene, Scene::new());
        self.scene_ctx.pending_drop_scenes.push(old);
        self.selection.clear();
        self.scene_ctx.editor_scene_path = None;
        self.scene_ctx.dirty = false;
        self.undo_system.clear();

        let (w, h) = self.viewport.size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }
    }

    pub(super) fn new_scene_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(ref mut scene_name) = self.ui.new_scene_modal else {
            return;
        };

        let mut confirmed = false;
        let mut cancelled = false;

        // Dim background.
        let screen_rect = ctx.input(|i| i.viewport_rect());
        egui::Area::new(egui::Id::new("new_scene_modal_bg"))
            .fixed_pos(screen_rect.left_top())
            .show(ctx, |ui| {
                ui.allocate_response(screen_rect.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(128));
            });

        egui::Window::new("New Scene")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(300.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Scene name:");
                let text_edit = ui.text_edit_singleline(scene_name);

                // Auto-focus the text field on first frame.
                if text_edit.gained_focus() || !text_edit.has_focus() {
                    text_edit.request_focus();
                }

                // Enter confirms.
                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }

                // Escape cancels.
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancelled = true;
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    let name_valid = !scene_name.trim().is_empty();
                    if ui
                        .add_enabled(name_valid, egui::Button::new("Create"))
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            let name = self.ui.new_scene_modal.take().unwrap_or_default();
            let name = name.trim().to_string();
            if !name.is_empty() {
                self.create_named_scene(&name);
            }
        } else if cancelled {
            self.ui.new_scene_modal = None;
        }
    }

    pub(super) fn shortcuts_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.ui.show_shortcuts_dialog {
            return;
        }

        egui::Window::new("Keyboard Shortcuts")
            .collapsible(false)
            .resizable(false)
            .open(&mut self.ui.show_shortcuts_dialog)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("General").strong());
                egui::Grid::new("shortcuts_general")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Ctrl+N");
                        ui.label("New Scene");
                        ui.end_row();
                        ui.label("Ctrl+O");
                        ui.label("Open Scene");
                        ui.end_row();
                        ui.label("Ctrl+S");
                        ui.label("Save Scene");
                        ui.end_row();
                        ui.label("Ctrl+Shift+S");
                        ui.label("Save As");
                        ui.end_row();
                        ui.label("Ctrl+Z");
                        ui.label("Undo");
                        ui.end_row();
                        ui.label("Ctrl+Y");
                        ui.label("Redo");
                        ui.end_row();
                        ui.label("Ctrl+D");
                        ui.label("Duplicate Entity");
                        ui.end_row();
                        ui.label("Delete");
                        ui.label("Delete Entity");
                        ui.end_row();
                        ui.label("Ctrl+R");
                        ui.label("Reload Scripts");
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Gizmo").strong());
                egui::Grid::new("shortcuts_gizmo")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Q");
                        ui.label("Select (No Gizmo)");
                        ui.end_row();
                        ui.label("W");
                        ui.label("Translate");
                        ui.end_row();
                        ui.label("E");
                        ui.label("Rotate");
                        ui.end_row();
                        ui.label("R");
                        ui.label("Scale");
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Viewport").strong());
                egui::Grid::new("shortcuts_viewport")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Middle Mouse");
                        ui.label("Pan");
                        ui.end_row();
                        ui.label("Alt + Left Mouse");
                        ui.label("Orbit");
                        ui.end_row();
                        ui.label("Alt + Right Mouse");
                        ui.label("Zoom");
                        ui.end_row();
                        ui.label("Scroll");
                        ui.label("Zoom");
                        ui.end_row();
                        ui.label("Right Mouse Hold");
                        ui.label("Fly Mode (WASD + QE)");
                        ui.end_row();
                        ui.label("Shift (in Fly)");
                        ui.label("Fast Movement");
                        ui.end_row();
                        ui.label("F");
                        ui.label("Focus Selected");
                        ui.end_row();
                    });
            });
    }

    fn create_named_scene(&mut self, name: &str) {
        let scene = Scene::new();

        // Build path: assets_root/scenes/<name>.ggscene
        let scenes_dir = self.project_state.assets_root.join("scenes");
        let _ = std::fs::create_dir_all(&scenes_dir);
        let file_name = format!("{}.ggscene", name);
        let scene_path = scenes_dir.join(&file_name);
        let path_str = scene_path.to_string_lossy().to_string();

        // Serialize the empty scene to disk immediately.
        if let Err(e) = SceneSerializer::serialize(&scene, &path_str, Some(name)) {
            warn!("Failed to create scene file '{}': {}", path_str, e);
        }

        // Swap in the new scene.
        let old = std::mem::replace(&mut self.scene, scene);
        self.scene_ctx.pending_drop_scenes.push(old);
        self.selection.clear();
        self.scene_ctx.editor_scene_path = Some(path_str);
        self.scene_ctx.dirty = false;
        self.undo_system.clear();

        let (w, h) = self.viewport.size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }

        panels::project::invalidate_scene_cache();
    }

    pub(super) fn open_scene(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if let Some(path) = FileDialogs::open_file("GGScene files", &["ggscene"]) {
            self.open_scene_from_path(std::path::Path::new(&path));
        }
    }

    fn scene_name_from_path(path: &str) -> Option<&str> {
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
    }

    pub(super) fn save_scene(&mut self) {
        if let Some(ref path) = self.scene_ctx.editor_scene_path {
            match SceneSerializer::serialize(&self.scene, path, Self::scene_name_from_path(path)) {
                Ok(()) => {
                    self.scene_ctx.dirty = false;
                    self.scene_ctx.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
                    Self::remove_autosave_file(path);
                }
                Err(e) => warn!("Failed to save scene to '{}': {}", path, e),
            }
        } else {
            self.save_scene_as();
        }
    }

    pub(super) fn save_scene_as(&mut self) {
        if let Some(path) = FileDialogs::save_file("GGScene files", &["ggscene"]) {
            match SceneSerializer::serialize(&self.scene, &path, Self::scene_name_from_path(&path))
            {
                Ok(()) => {
                    self.scene_ctx.editor_scene_path = Some(path);
                    self.scene_ctx.dirty = false;
                    self.scene_ctx.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
                    panels::project::invalidate_scene_cache();
                }
                Err(e) => warn!("Failed to save scene to '{}': {}", path, e),
            }
        }
    }

    /// Auto-save the current scene to a `.autosave.ggscene` sidecar file.
    pub(super) fn perform_autosave(&self) {
        if let Some(ref path) = self.scene_ctx.editor_scene_path {
            let autosave_path = Self::autosave_path_for(path);
            match SceneSerializer::serialize(
                &self.scene,
                &autosave_path,
                Self::scene_name_from_path(path),
            ) {
                Ok(()) => info!("Auto-saved to '{}'", autosave_path),
                Err(e) => warn!("Auto-save failed for '{}': {}", autosave_path, e),
            }
        }
    }

    /// Build the auto-save sidecar path: `foo.ggscene` -> `foo.autosave.ggscene`.
    fn autosave_path_for(scene_path: &str) -> String {
        let p = std::path::Path::new(scene_path);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("scene");
        if let Some(parent) = p.parent() {
            parent
                .join(format!("{}.autosave.ggscene", stem))
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{}.autosave.ggscene", stem)
        }
    }

    /// Remove the auto-save sidecar file after a successful manual save.
    fn remove_autosave_file(scene_path: &str) {
        let autosave = Self::autosave_path_for(scene_path);
        if std::path::Path::new(&autosave).exists() {
            if let Err(e) = std::fs::remove_file(&autosave) {
                warn!("Failed to remove auto-save file '{}': {}", autosave, e);
            }
        }
    }

    /// Check for auto-save recovery when opening a scene.
    ///
    /// If a `.autosave.ggscene` sidecar exists and is newer than the original,
    /// prompts the user to recover. Returns `Some(scene)` if recovery succeeds,
    /// `None` otherwise (auto-save cleaned up if stale or declined).
    pub(super) fn check_autosave_recovery(scene_path: &str) -> Option<Scene> {
        let autosave = Self::autosave_path_for(scene_path);
        let autosave_p = std::path::Path::new(&autosave);
        if !autosave_p.exists() {
            return None;
        }

        // Only offer recovery if the auto-save is newer than the original.
        let orig_modified = std::fs::metadata(scene_path)
            .and_then(|m| m.modified())
            .ok();
        let auto_modified = std::fs::metadata(&autosave).and_then(|m| m.modified()).ok();

        let is_newer = match (orig_modified, auto_modified) {
            (Some(orig), Some(auto)) => auto > orig,
            (None, Some(_)) => true, // original missing or unreadable
            _ => false,
        };

        if !is_newer {
            let _ = std::fs::remove_file(&autosave);
            return None;
        }

        if gg_engine::platform_utils::confirm_dialog(
            "Recover Auto-Save",
            "An auto-save file was found that is newer than the last manual save.\n\n\
             Recover unsaved changes?",
        ) {
            let mut recovered = Scene::new();
            match SceneSerializer::deserialize(&mut recovered, &autosave) {
                Ok(()) => {
                    info!("Recovered scene from auto-save: {}", autosave);
                    let _ = std::fs::remove_file(&autosave);
                    return Some(recovered);
                }
                Err(e) => warn!("Failed to deserialize auto-save file '{}': {}", autosave, e),
            }
        } else {
            // User declined — clean up stale auto-save.
            let _ = std::fs::remove_file(&autosave);
        }

        None
    }

    pub(super) fn perform_undo(&mut self) {
        // Capture selected entities' UUIDs before replacing the scene,
        // since hecs entity IDs change after deserialization.
        let selected_uuids: Vec<u64> = self
            .selection
            .iter()
            .filter_map(|sel| {
                self.scene
                    .get_component::<IdComponent>(sel)
                    .map(|id| id.id.raw())
            })
            .collect();
        if let Some(restored) = self.undo_system.undo(&self.scene) {
            let old = std::mem::replace(&mut self.scene, restored);
            self.scene_ctx.pending_drop_scenes.push(old);
            // Restore selection by IdComponent UUID (stable across serialization).
            self.selection.clear();
            for uuid in selected_uuids {
                if let Some(entity) = self.scene.find_entity_by_uuid(uuid) {
                    self.selection.add(entity);
                }
            }
            let (w, h) = self.viewport.size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.scene_ctx.dirty = true;
        }
    }

    pub(super) fn perform_redo(&mut self) {
        let selected_uuids: Vec<u64> = self
            .selection
            .iter()
            .filter_map(|sel| {
                self.scene
                    .get_component::<IdComponent>(sel)
                    .map(|id| id.id.raw())
            })
            .collect();
        if let Some(restored) = self.undo_system.redo(&self.scene) {
            let old = std::mem::replace(&mut self.scene, restored);
            self.scene_ctx.pending_drop_scenes.push(old);
            self.selection.clear();
            for uuid in selected_uuids {
                if let Some(entity) = self.scene.find_entity_by_uuid(uuid) {
                    self.selection.add(entity);
                }
            }
            let (w, h) = self.viewport.size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.scene_ctx.dirty = true;
        }
    }

    pub(super) fn on_copy_entity(&mut self) {
        let uuids: Vec<u64> = self
            .selection
            .iter()
            .filter(|e| self.scene.is_alive(*e))
            .filter_map(|e| {
                self.scene
                    .get_component::<IdComponent>(e)
                    .map(|id| id.id.raw())
            })
            .collect();
        if !uuids.is_empty() {
            self.ui.clipboard_entity_uuids = uuids;
        }
    }

    pub(super) fn on_paste_entity(&mut self) {
        if self.ui.clipboard_entity_uuids.is_empty() {
            return;
        }
        let sources: Vec<Entity> = self
            .ui
            .clipboard_entity_uuids
            .iter()
            .filter_map(|&uuid| self.scene.find_entity_by_uuid(uuid))
            .collect();
        if sources.is_empty() {
            return;
        }
        self.undo_system.record(&self.scene, "Paste entity");
        self.selection.clear();
        for source in sources {
            let duplicate = self.scene.duplicate_entity(source);
            self.selection.add(duplicate);
        }
        self.scene_ctx.dirty = true;
    }

    pub(super) fn on_duplicate_entity(&mut self) {
        let entities: Vec<Entity> = self
            .selection
            .iter()
            .filter(|e| self.scene.is_alive(*e))
            .collect();
        if entities.is_empty() {
            return;
        }
        self.undo_system.record(&self.scene, "Duplicate entity");
        self.selection.clear();
        for entity in entities {
            let duplicate = self.scene.duplicate_entity(entity);
            self.selection.add(duplicate);
        }
        self.scene_ctx.dirty = true;
    }

    pub(super) fn handle_hierarchy_action(
        &mut self,
        action: Option<panels::scene_hierarchy::HierarchyExternalAction>,
    ) {
        use panels::scene_hierarchy::HierarchyExternalAction;
        let Some(action) = action else { return };
        match action {
            HierarchyExternalAction::SaveAsPrefab(entity) => {
                if !self.scene.is_alive(entity) {
                    return;
                }
                if let Some(path_str) = FileDialogs::save_file("GG Prefab", &["ggprefab"]) {
                    match SceneSerializer::serialize_prefab(&self.scene, entity, &path_str) {
                        Ok(()) => {
                            // Auto-import to asset registry if inside assets directory.
                            let path = Path::new(&path_str);
                            if let Ok(rel) = path.strip_prefix(&self.project_state.assets_root) {
                                let rel_str = rel.to_string_lossy().replace('\\', "/");
                                if let Some(ref mut am) = self.project_state.asset_manager {
                                    am.import_asset(&rel_str);
                                }
                            }
                            panels::content_browser::invalidate_dir_cache();
                        }
                        Err(e) => warn!("Failed to save prefab '{}': {}", path_str, e),
                    }
                }
            }
            HierarchyExternalAction::InstantiatePrefab { path, parent } => {
                let path_str = path.to_string_lossy().to_string();
                self.undo_system.record(&self.scene, "Instantiate prefab");
                match SceneSerializer::instantiate_prefab(&mut self.scene, &path_str) {
                    Ok(root) => {
                        if let Some(parent_entity) = parent {
                            if self.scene.is_alive(parent_entity) {
                                self.scene.set_parent(root, parent_entity, false);
                            }
                        }
                        self.selection.set(root);
                        self.scene_ctx.dirty = true;
                    }
                    Err(e) => warn!("Failed to instantiate prefab '{}': {}", path_str, e),
                }
            }
        }
    }

    pub(super) fn open_scene_from_path(&mut self, path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let mut new_scene = Scene::new();
        if SceneSerializer::deserialize(&mut new_scene, &path_str).is_ok() {
            // Check for auto-save recovery before committing the loaded scene.
            let recovered = if let Some(recovered) = Self::check_autosave_recovery(&path_str) {
                new_scene = recovered;
                true
            } else {
                false
            };

            // Only clear state after confirming the load succeeded.
            self.scene_ctx.dirty = recovered;
            self.undo_system.clear();
            let old = std::mem::replace(&mut self.scene, new_scene);
            self.scene_ctx.pending_drop_scenes.push(old);
            self.selection.clear();
            self.scene_ctx.editor_scene_path = Some(path_str);
            let (w, h) = self.viewport.size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
        }
    }

    pub(super) fn load_project_from_path(&mut self, project_path: &std::path::Path) {
        let abs_path =
            std::fs::canonicalize(project_path).unwrap_or_else(|_| project_path.to_path_buf());
        let project = match Project::load(&abs_path.to_string_lossy()) {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to load project '{}': {}", abs_path.display(), e);
                return;
            }
        };

        // Stop playback if active.
        if self.playback.scene_state != SceneState::Edit {
            self.on_scene_stop();
        }

        // Set CWD to the project directory.
        if let Err(e) = std::env::set_current_dir(project.project_directory()) {
            warn!(
                "Failed to set CWD to project directory '{}': {}",
                project.project_directory().display(),
                e
            );
        }

        // Update assets root.
        self.project_state.assets_root = project.asset_directory_path();
        self.project_state.current_directory = self.project_state.assets_root.clone();

        // Create and load asset manager for the new project.
        let mut am = EditorAssetManager::new(&self.project_state.assets_root);
        am.load_registry();
        self.project_state.asset_manager = Some(am);

        // Load start scene.
        let start_path = project.start_scene_path();
        if start_path.exists() {
            let path_str = start_path.to_string_lossy().to_string();
            let mut new_scene = Scene::new();
            if SceneSerializer::deserialize(&mut new_scene, &path_str).is_ok() {
                // Check for auto-save recovery.
                let recovered = if let Some(recovered) = Self::check_autosave_recovery(&path_str) {
                    new_scene = recovered;
                    true
                } else {
                    false
                };
                let old = std::mem::replace(&mut self.scene, new_scene);
                self.scene_ctx.pending_drop_scenes.push(old);
                self.scene_ctx.editor_scene_path = Some(path_str);
                self.scene_ctx.dirty = recovered;
            }
        } else {
            let old = std::mem::replace(&mut self.scene, Scene::new());
            self.scene_ctx.pending_drop_scenes.push(old);
            self.scene_ctx.editor_scene_path = None;
        }

        // Restart script watcher for the new project's scripts directory.
        #[cfg(feature = "lua-scripting")]
        {
            self._script_watcher = super::create_script_watcher(
                &self.project_state.assets_root.join("scripts"),
                &self.script_reload_pending,
            );
        }

        // Resize viewport for the new scene.
        let (w, h) = self.viewport.size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }

        // Update recent projects and editor state.
        self.editor_settings
            .add_recent_project(project.name(), &abs_path.to_string_lossy());
        self.project_state.project = Some(project);
        self.selection.clear();
        // Don't overwrite dirty=true set by autosave recovery above.
        if !self.scene_ctx.dirty {
            self.scene_ctx.dirty = false;
        }
        self.undo_system.clear();
        self.editor_mode = EditorMode::Editor;

        // Reset all thread-local panel caches/dialogs for the new project.
        panels::reset_all_panel_state();
    }

    pub(super) fn open_project(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if let Some(path) = FileDialogs::open_file("GGProject files", &["ggproject"]) {
            self.load_project_from_path(&PathBuf::from(&path));
        }
    }

    pub(super) fn handle_new_project_from_hub(&mut self, path: &Path) {
        // Ensure the parent directory exists (project_name/ subfolder).
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    error!(
                        "Failed to create project directory {}: {}",
                        parent.display(),
                        e
                    );
                    return;
                }
            }
        }

        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());

        match Project::new(&path.to_string_lossy(), &name) {
            Ok(_) => self.load_project_from_path(path),
            Err(e) => warn!("Failed to create project '{}': {}", name, e),
        }
    }
}

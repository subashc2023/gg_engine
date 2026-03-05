pub(crate) mod content_browser;
pub(crate) mod project;
pub(crate) mod properties;
mod scene_hierarchy;
mod settings;
pub(crate) mod viewport;

use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::EditorTheme;
use transform_gizmo_egui::Gizmo;

use crate::TilemapPaintState;
use crate::gizmo::GizmoOperation;
use crate::undo::UndoSystem;

/// Reset all thread-local panel state (caches, rename/delete dialogs, etc.).
/// Call this when switching projects or performing a full editor reset.
pub(crate) fn reset_all_panel_state() {
    content_browser::invalidate_dir_cache();
    content_browser::reset_dialog_state();
    project::invalidate_scene_cache();
    #[cfg(feature = "lua-scripting")]
    properties::clear_field_cache();
}

// ---------------------------------------------------------------------------
// Tileset preview info (for viewport overlay)
// ---------------------------------------------------------------------------

pub(crate) struct TilesetPreviewInfo {
    pub egui_tex: egui::TextureId,
    pub tex_w: f32,
    pub tex_h: f32,
    pub tileset_columns: u32,
    pub cell_size: Vec2,
    pub spacing: Vec2,
    pub margin: Vec2,
}

/// Compute the UV min corner for a tile in the tileset image.
pub(crate) fn tile_uv_min(
    ts_col: usize, ts_row: usize,
    cell_size: Vec2, spacing: Vec2, margin: Vec2,
    tex_w: f32, tex_h: f32,
) -> (f32, f32) {
    let px_x = margin.x + ts_col as f32 * (cell_size.x + spacing.x);
    let px_y = margin.y + ts_row as f32 * (cell_size.y + spacing.y);
    (px_x / tex_w, px_y / tex_h)
}

/// Compute the UV max corner for a tile in the tileset image.
pub(crate) fn tile_uv_max(
    ts_col: usize, ts_row: usize,
    cell_size: Vec2, spacing: Vec2, margin: Vec2,
    tex_w: f32, tex_h: f32,
) -> (f32, f32) {
    let px_x = margin.x + ts_col as f32 * (cell_size.x + spacing.x) + cell_size.x;
    let px_y = margin.y + ts_row as f32 * (cell_size.y + spacing.y) + cell_size.y;
    (px_x / tex_w, px_y / tex_h)
}

// ---------------------------------------------------------------------------
// Tab identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Tab {
    SceneHierarchy,
    Viewport,
    Properties,
    ContentBrowser,
    Settings,
    Project,
}

// ---------------------------------------------------------------------------
// TabViewer sub-structs
// ---------------------------------------------------------------------------

/// Viewport-specific state (gizmos, camera, framebuffer, picking).
pub(crate) struct ViewportState<'a> {
    pub(crate) size: &'a mut (u32, u32),
    pub(crate) focused: &'a mut bool,
    pub(crate) hovered: &'a mut bool,
    pub(crate) fb_tex_id: Option<egui::TextureId>,
    pub(crate) gizmo: &'a mut Gizmo,
    pub(crate) gizmo_operation: &'a mut GizmoOperation,
    pub(crate) gizmo_editing: &'a mut bool,
    pub(crate) editor_camera: &'a EditorCamera,
    pub(crate) scene_fb: &'a mut Option<Framebuffer>,
    pub(crate) hovered_entity: i32,
    pub(crate) mouse_pos: &'a mut Option<(f32, f32)>,
    pub(crate) tileset_preview: Option<TilesetPreviewInfo>,
    pub(crate) snap_to_grid: bool,
    pub(crate) grid_size: f32,
    pub(crate) gizmo_local: &'a mut bool,
}

/// Project and asset context shared across content browser, properties, project panels.
pub(crate) struct ProjectContext<'a> {
    pub(crate) assets_root: &'a std::path::Path,
    pub(crate) current_directory: &'a mut std::path::PathBuf,
    pub(crate) asset_manager: &'a mut Option<EditorAssetManager>,
    pub(crate) project_name: Option<&'a str>,
    pub(crate) editor_scene_path: Option<&'a str>,
    pub(crate) egui_texture_map: &'a std::collections::HashMap<u64, egui::TextureId>,
}

// ---------------------------------------------------------------------------
// TabViewer
// ---------------------------------------------------------------------------

pub(crate) struct EditorTabViewer<'a> {
    pub(crate) scene: &'a mut Scene,
    pub(crate) selection_context: &'a mut Option<Entity>,
    pub(crate) pending_open_path: &'a mut Option<std::path::PathBuf>,
    pub(crate) is_playing: bool,
    pub(crate) scene_dirty: &'a mut bool,
    pub(crate) undo_system: &'a mut UndoSystem,
    pub(crate) hierarchy_filter: &'a mut String,
    pub(crate) scene_warnings: &'a [String],
    pub(crate) tilemap_paint: &'a mut TilemapPaintState,
    pub(crate) vsync: &'a mut bool,
    pub(crate) frame_time_ms: f32,
    pub(crate) render_stats: Renderer2DStats,
    pub(crate) show_physics_colliders: &'a mut bool,
    pub(crate) show_grid: &'a mut bool,
    pub(crate) snap_to_grid: &'a mut bool,
    pub(crate) grid_size: &'a mut f32,
    pub(crate) theme: &'a mut EditorTheme,
    pub(crate) viewport: ViewportState<'a>,
    pub(crate) project: ProjectContext<'a>,
}

impl EditorTabViewer<'_> {
    fn unfocus_viewport_on_click(&mut self, ui: &egui::Ui) {
        let clicked = ui.input(|i| i.pointer.any_pressed());
        if clicked && ui.ui_contains_pointer() {
            *self.viewport.focused = false;
        }
    }
}

impl egui_dock::TabViewer for EditorTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::SceneHierarchy => "Scene Hierarchy".into(),
            Tab::Viewport => "Viewport".into(),
            Tab::Properties => "Properties".into(),
            Tab::ContentBrowser => "Content Browser".into(),
            Tab::Settings => "Settings".into(),
            Tab::Project => "Project".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::SceneHierarchy => {
                self.unfocus_viewport_on_click(ui);
                scene_hierarchy::scene_hierarchy_ui(ui, self.scene, self.selection_context, self.scene_dirty, self.undo_system, self.hierarchy_filter);
            }

            Tab::Viewport => {
                viewport::viewport_ui(
                    ui,
                    self.scene,
                    self.selection_context,
                    self.viewport.size,
                    self.viewport.focused,
                    self.viewport.hovered,
                    self.viewport.fb_tex_id,
                    self.viewport.gizmo,
                    self.viewport.gizmo_operation,
                    self.viewport.editor_camera,
                    self.viewport.scene_fb,
                    self.viewport.hovered_entity,
                    self.pending_open_path,
                    self.is_playing,
                    self.scene_dirty,
                    self.undo_system,
                    self.viewport.gizmo_editing,
                    self.tilemap_paint,
                    self.viewport.mouse_pos,
                    &self.viewport.tileset_preview,
                    self.viewport.snap_to_grid,
                    self.viewport.grid_size,
                    self.viewport.gizmo_local,
                );
            }

            Tab::Properties => {
                self.unfocus_viewport_on_click(ui);
                properties::properties_ui(
                    ui,
                    self.scene,
                    self.selection_context,
                    self.project.asset_manager,
                    self.is_playing,
                    self.project.assets_root,
                    self.scene_dirty,
                    self.undo_system,
                    self.tilemap_paint,
                    self.project.egui_texture_map,
                );
            }

            Tab::ContentBrowser => {
                self.unfocus_viewport_on_click(ui);
                content_browser::content_browser_ui(ui, self.project.current_directory, self.project.assets_root, self.project.asset_manager, self.scene);
            }

            Tab::Settings => {
                self.unfocus_viewport_on_click(ui);
                settings::settings_ui(
                    ui,
                    self.scene,
                    self.frame_time_ms,
                    self.render_stats,
                    self.vsync,
                    self.show_physics_colliders,
                    self.viewport.hovered_entity,
                    self.show_grid,
                    self.snap_to_grid,
                    self.grid_size,
                    self.scene_warnings,
                    self.theme,
                );
            }

            Tab::Project => {
                self.unfocus_viewport_on_click(ui);
                project::project_ui(
                    ui,
                    self.project.project_name,
                    self.project.assets_root,
                    self.project.editor_scene_path,
                    self.pending_open_path,
                );
            }
        }
    }

    fn is_closeable(&self, _tab: &Tab) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Viewport)
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::SceneHierarchy | Tab::Properties | Tab::Settings | Tab::Project => [false, true],
            _ => [false, false],
        }
    }
}

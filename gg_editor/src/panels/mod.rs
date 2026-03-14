pub(crate) mod animation_timeline;
mod console;
pub(crate) mod content_browser;
pub(crate) mod game_viewport;
pub(crate) mod project;
pub(crate) mod properties;
pub(crate) mod scene_hierarchy;
mod settings;
pub(crate) mod viewport;

use std::path::Path;

use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::EditorTheme;

use crate::{GpuTimingSnapshot, PostProcessSettings};

/// Strip the `\\?\` UNC prefix that Windows canonicalization adds,
/// so that `strip_prefix` works when comparing paths from different sources
/// (e.g. file dialogs return `C:\...` but canonicalized paths have `\\?\C:\...`).
fn strip_unc_prefix(p: &Path) -> &Path {
    p.to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .map(Path::new)
        .unwrap_or(p)
}

/// Compute a relative asset path from an absolute file path and the asset directory.
/// Handles UNC prefix mismatches on Windows.
pub(crate) fn relative_asset_path(abs_path: &Path, asset_dir: &Path) -> String {
    let clean_abs = strip_unc_prefix(abs_path);
    let clean_dir = strip_unc_prefix(asset_dir);
    clean_abs
        .strip_prefix(clean_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| abs_path.to_string_lossy().to_string())
}
use transform_gizmo_egui::Gizmo;

use crate::gizmo::GizmoOperation;
use crate::selection::Selection;
use crate::undo::UndoSystem;
use crate::TilemapPaintState;

/// Reset all thread-local panel state (caches, rename/delete dialogs, etc.).
/// Call this when switching projects or performing a full editor reset.
pub(crate) fn reset_all_panel_state() {
    content_browser::invalidate_dir_cache();
    content_browser::reset_dialog_state();
    scene_hierarchy::reset_hierarchy_state();
    project::invalidate_scene_cache();
    animation_timeline::reset_animation_timeline_state();
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
    ts_col: usize,
    ts_row: usize,
    cell_size: Vec2,
    spacing: Vec2,
    margin: Vec2,
    tex_w: f32,
    tex_h: f32,
) -> (f32, f32) {
    let px_x = margin.x + ts_col as f32 * (cell_size.x + spacing.x);
    let px_y = margin.y + ts_row as f32 * (cell_size.y + spacing.y);
    (px_x / tex_w, px_y / tex_h)
}

/// Compute the UV max corner for a tile in the tileset image.
pub(crate) fn tile_uv_max(
    ts_col: usize,
    ts_row: usize,
    cell_size: Vec2,
    spacing: Vec2,
    margin: Vec2,
    tex_w: f32,
    tex_h: f32,
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
    GameViewport,
    Properties,
    ContentBrowser,
    Settings,
    Project,
    Console,
    AnimationTimeline,
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

/// Game viewport state (game camera preview).
pub(crate) struct GameViewportState<'a> {
    pub(crate) size: &'a mut (u32, u32),
    pub(crate) hovered: &'a mut bool,
    pub(crate) fb_tex_id: Option<egui::TextureId>,
}

/// Project and asset context shared across content browser, properties, project panels.
pub(crate) struct ProjectContext<'a> {
    pub(crate) assets_root: &'a std::path::Path,
    pub(crate) current_directory: &'a mut std::path::PathBuf,
    pub(crate) asset_manager: &'a mut Option<EditorAssetManager>,
    pub(crate) project_name: Option<&'a str>,
    pub(crate) editor_scene_path: Option<&'a str>,
    pub(crate) egui_texture_map: &'a std::collections::HashMap<u64, egui::TextureId>,
    pub(crate) input_actions: &'a mut gg_engine::InputActionMap,
    pub(crate) project: &'a mut Option<gg_engine::Project>,
}

// ---------------------------------------------------------------------------
// TabViewer
// ---------------------------------------------------------------------------

pub(crate) struct EditorTabViewer<'a> {
    pub(crate) scene: &'a mut Scene,
    pub(crate) selection: &'a mut Selection,
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
    pub(crate) theme: &'a mut EditorTheme,
    pub(crate) reload_shaders_requested: &'a mut bool,
    pub(crate) msaa_samples: &'a mut gg_engine::MsaaSamples,
    pub(crate) max_msaa_samples: MsaaSamples,
    pub(crate) msaa_changed: &'a mut bool,
    pub(crate) viewport: ViewportState<'a>,
    pub(crate) game_viewport: GameViewportState<'a>,
    pub(crate) project: ProjectContext<'a>,
    pub(crate) hierarchy_action: &'a mut Option<scene_hierarchy::HierarchyExternalAction>,
    pub(crate) postprocess_settings: &'a mut PostProcessSettings,
    pub(crate) gpu_timing: &'a mut GpuTimingSnapshot,
    pub(crate) show_msaa_test: &'a mut bool,
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
            Tab::GameViewport => "Game".into(),
            Tab::Properties => "Properties".into(),
            Tab::ContentBrowser => "Content Browser".into(),
            Tab::Settings => "Settings".into(),
            Tab::Project => "Project".into(),
            Tab::Console => "Console".into(),
            Tab::AnimationTimeline => "Animation".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::SceneHierarchy => {
                self.unfocus_viewport_on_click(ui);
                if let Some(action) = scene_hierarchy::scene_hierarchy_ui(
                    ui,
                    self.scene,
                    self.selection,
                    self.scene_dirty,
                    self.undo_system,
                    self.hierarchy_filter,
                ) {
                    *self.hierarchy_action = Some(action);
                }
            }

            Tab::Viewport => {
                viewport::viewport_ui(
                    ui,
                    self.scene,
                    self.selection,
                    &mut self.viewport,
                    self.pending_open_path,
                    self.is_playing,
                    self.scene_dirty,
                    self.undo_system,
                    self.tilemap_paint,
                );
            }

            Tab::GameViewport => {
                game_viewport::game_viewport_ui(
                    ui,
                    self.game_viewport.size,
                    self.game_viewport.hovered,
                    self.game_viewport.fb_tex_id,
                );
            }

            Tab::Properties => {
                self.unfocus_viewport_on_click(ui);
                properties::properties_ui(
                    ui,
                    self.scene,
                    self.selection,
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
                content_browser::content_browser_ui(
                    ui,
                    self.project.current_directory,
                    self.project.assets_root,
                    self.project.asset_manager,
                    self.scene,
                    self.project.egui_texture_map,
                );
            }

            Tab::Settings => {
                self.unfocus_viewport_on_click(ui);
                let mut state = settings::SettingsState {
                    frame_time_ms: self.frame_time_ms,
                    render_stats: self.render_stats,
                    vsync: self.vsync,
                    scene_warnings: self.scene_warnings,
                    theme: self.theme,
                    reload_shaders_requested: self.reload_shaders_requested,
                    msaa_samples: self.msaa_samples,
                    max_msaa_samples: self.max_msaa_samples,
                    msaa_changed: self.msaa_changed,
                    pp_settings: self.postprocess_settings,
                    gpu_timing: self.gpu_timing,
                    show_msaa_test: self.show_msaa_test,
                };
                settings::settings_ui(ui, self.scene, &mut state);
            }

            Tab::Project => {
                self.unfocus_viewport_on_click(ui);
                project::project_ui(
                    ui,
                    self.project.project_name,
                    self.project.assets_root,
                    self.project.editor_scene_path,
                    self.pending_open_path,
                    self.project.input_actions,
                    self.project.project,
                );
            }

            Tab::Console => {
                self.unfocus_viewport_on_click(ui);
                console::console_ui(ui);
            }

            Tab::AnimationTimeline => {
                self.unfocus_viewport_on_click(ui);
                animation_timeline::animation_timeline_ui(
                    ui,
                    self.scene,
                    self.selection,
                    self.project.asset_manager,
                    self.project.egui_texture_map,
                    self.scene_dirty,
                    self.undo_system,
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
        !matches!(tab, Tab::Viewport | Tab::GameViewport)
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::SceneHierarchy | Tab::Properties | Tab::Settings | Tab::Project => [false, true],
            Tab::Console | Tab::AnimationTimeline => [false, false],
            _ => [false, false],
        }
    }
}

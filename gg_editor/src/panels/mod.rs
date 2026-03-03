pub(crate) mod content_browser;
mod properties;
mod scene_hierarchy;
mod settings;
pub(crate) mod viewport;

use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::Gizmo;

use crate::gizmo::GizmoOperation;

// ---------------------------------------------------------------------------
// Tab identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub(crate) enum Tab {
    SceneHierarchy,
    Viewport,
    Properties,
    ContentBrowser,
    Settings,
}

// ---------------------------------------------------------------------------
// TabViewer
// ---------------------------------------------------------------------------

pub(crate) struct EditorTabViewer<'a> {
    pub(crate) scene: &'a mut Scene,
    pub(crate) selection_context: &'a mut Option<Entity>,
    pub(crate) viewport_size: &'a mut (u32, u32),
    pub(crate) viewport_focused: &'a mut bool,
    pub(crate) viewport_hovered: &'a mut bool,
    pub(crate) fb_tex_id: Option<egui::TextureId>,
    pub(crate) vsync: &'a mut bool,
    pub(crate) frame_time_ms: f32,
    pub(crate) gizmo: &'a mut Gizmo,
    pub(crate) gizmo_operation: GizmoOperation,
    pub(crate) editor_camera: &'a EditorCamera,
    pub(crate) scene_fb: &'a mut Option<Framebuffer>,
    pub(crate) hovered_entity: i32,
    pub(crate) current_directory: &'a mut std::path::PathBuf,
    pub(crate) pending_open_path: &'a mut Option<std::path::PathBuf>,
    pub(crate) pending_texture_loads: &'a mut Vec<(Entity, std::path::PathBuf)>,
}

impl EditorTabViewer<'_> {
    fn unfocus_viewport_on_click(&mut self, ui: &egui::Ui) {
        let clicked = ui.input(|i| i.pointer.any_pressed());
        if clicked && ui.ui_contains_pointer() {
            *self.viewport_focused = false;
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
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::SceneHierarchy => {
                self.unfocus_viewport_on_click(ui);
                scene_hierarchy::scene_hierarchy_ui(ui, self.scene, self.selection_context);
            }

            Tab::Viewport => {
                viewport::viewport_ui(
                    ui,
                    self.scene,
                    self.selection_context,
                    self.viewport_size,
                    self.viewport_focused,
                    self.viewport_hovered,
                    self.fb_tex_id,
                    self.gizmo,
                    self.gizmo_operation,
                    self.editor_camera,
                    self.scene_fb,
                    self.hovered_entity,
                    self.pending_open_path,
                );
            }

            Tab::Properties => {
                self.unfocus_viewport_on_click(ui);
                properties::properties_ui(
                    ui,
                    self.scene,
                    self.selection_context,
                    self.pending_texture_loads,
                );
            }

            Tab::ContentBrowser => {
                self.unfocus_viewport_on_click(ui);
                content_browser::content_browser_ui(ui, self.current_directory);
            }

            Tab::Settings => {
                self.unfocus_viewport_on_click(ui);
                settings::settings_ui(
                    ui,
                    self.scene,
                    self.frame_time_ms,
                    self.vsync,
                    self.hovered_entity,
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

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

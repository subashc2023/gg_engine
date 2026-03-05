use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;

#[cfg(feature = "lua-scripting")]
use std::cell::RefCell;
#[cfg(feature = "lua-scripting")]
use std::collections::HashMap;

#[cfg(feature = "lua-scripting")]
thread_local! {
    static FIELD_CACHE: RefCell<HashMap<String, Vec<(String, ScriptFieldValue)>>> =
        RefCell::new(HashMap::new());
}

/// Clear the cached script field definitions. Call this after hot-reloading
/// Lua scripts so that updated `fields` tables are re-discovered.
#[cfg(feature = "lua-scripting")]
pub(crate) fn clear_field_cache() {
    FIELD_CACHE.with(|c| c.borrow_mut().clear());
}

#[cfg(feature = "lua-scripting")]
fn get_cached_fields(script_path: &str) -> Vec<(String, ScriptFieldValue)> {
    if script_path.is_empty() {
        return Vec::new();
    }
    FIELD_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(fields) = cache.get(script_path) {
            return fields.clone();
        }
        let fields = ScriptEngine::discover_fields(script_path);
        cache.insert(script_path.to_string(), fields.clone());
        fields
    })
}

pub(crate) fn draw_native_script_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    _scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<NativeScriptComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Native Script")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("native_script", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Script");
                ui.label(
                    egui::RichText::new("(bound in code)")
                        .color(egui::Color32::from_rgb(0x96, 0x96, 0x96))
                        .italics(),
                );
            });
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove = true;
                ui.close();
            }
        });
    }

    remove
}

#[cfg(feature = "lua-scripting")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_lua_script_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    assets_root: &std::path::Path,
    is_playing: bool,
    _scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<LuaScriptComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Lua Script")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("lua_script", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (script_path, field_overrides) = scene
                .get_component::<LuaScriptComponent>(entity)
                .map(|lsc| (lsc.script_path.clone(), lsc.field_overrides.clone()))
                .unwrap_or_default();

            let mut new_script_path = None;

            ui.horizontal(|ui| {
                ui.label("Script");

                let display = if script_path.is_empty() {
                    "None".to_string()
                } else {
                    std::path::Path::new(&script_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| script_path.clone())
                };
                let btn_resp = ui.add_sized(
                    [ui.available_width(), 0.0],
                    egui::Button::new(display),
                );

                if btn_resp.clicked() {
                    let scripts_dir = assets_root.join("scripts");
                    let scripts_dir_str = scripts_dir.to_string_lossy();
                    if let Some(path) =
                        FileDialogs::open_file_in("Lua scripts", &["lua"], &scripts_dir_str)
                    {
                        new_script_path = Some(path);
                    }
                }

                if let Some(payload) =
                    btn_resp.dnd_release_payload::<ContentBrowserPayload>()
                {
                    if !payload.is_directory {
                        let ext = payload
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if ext == "lua" {
                            new_script_path =
                                Some(payload.path.to_string_lossy().to_string());
                        }
                    }
                }

                if btn_resp
                    .dnd_hover_payload::<ContentBrowserPayload>()
                    .is_some()
                {
                    ui.painter().rect_stroke(
                        btn_resp.rect,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(
                            2.0,
                            egui::Color32::from_rgb(0x56, 0x9C, 0xD6),
                        ),
                        egui::StrokeKind::Inside,
                    );
                }
            });

            // Apply script path change.
            if let Some(path) = new_script_path {
                FIELD_CACHE.with(|c| c.borrow_mut().remove(&path));
                if let Some(mut lsc) =
                    scene.get_component_mut::<LuaScriptComponent>(entity)
                {
                    if lsc.script_path != path {
                        lsc.field_overrides.clear();
                    }
                    lsc.script_path = path;
                }
            }

            // ----- Script Fields -----
            if !script_path.is_empty() {
                let entity_uuid = scene
                    .get_component::<IdComponent>(entity)
                    .map(|id| id.id.raw())
                    .unwrap_or(0);

                let fields: Vec<(String, ScriptFieldValue)> = if is_playing {
                    scene
                        .script_engine()
                        .and_then(|eng| eng.get_entity_fields(entity_uuid))
                        .unwrap_or_else(|| get_cached_fields(&script_path))
                } else {
                    get_cached_fields(&script_path)
                };

                if !fields.is_empty() {
                    ui.separator();
                }

                for (name, default_value) in &fields {
                    let current = if is_playing {
                        default_value.clone()
                    } else {
                        field_overrides
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| default_value.clone())
                    };

                    ui.horizontal(|ui| {
                        ui.label(name);

                        match current {
                            ScriptFieldValue::Float(mut v) => {
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut v)
                                            .speed(0.1),
                                    )
                                    .changed()
                                {
                                    let new_val = ScriptFieldValue::Float(v);
                                    if is_playing {
                                        if let Some(eng) = scene.script_engine() {
                                            eng.set_entity_field(
                                                entity_uuid,
                                                name,
                                                &new_val,
                                            );
                                        }
                                    }
                                    if let Some(mut lsc) = scene
                                        .get_component_mut::<LuaScriptComponent>(entity)
                                    {
                                        lsc.field_overrides
                                            .insert(name.clone(), new_val);
                                    }
                                }
                            }
                            ScriptFieldValue::Bool(mut v) => {
                                if ui.checkbox(&mut v, "").changed() {
                                    let new_val = ScriptFieldValue::Bool(v);
                                    if is_playing {
                                        if let Some(eng) = scene.script_engine() {
                                            eng.set_entity_field(
                                                entity_uuid,
                                                name,
                                                &new_val,
                                            );
                                        }
                                    }
                                    if let Some(mut lsc) = scene
                                        .get_component_mut::<LuaScriptComponent>(entity)
                                    {
                                        lsc.field_overrides
                                            .insert(name.clone(), new_val);
                                    }
                                }
                            }
                            ScriptFieldValue::String(mut v) => {
                                if ui
                                    .text_edit_singleline(&mut v)
                                    .changed()
                                {
                                    let new_val = ScriptFieldValue::String(v);
                                    if is_playing {
                                        if let Some(eng) = scene.script_engine() {
                                            eng.set_entity_field(
                                                entity_uuid,
                                                name,
                                                &new_val,
                                            );
                                        }
                                    }
                                    if let Some(mut lsc) = scene
                                        .get_component_mut::<LuaScriptComponent>(entity)
                                    {
                                        lsc.field_overrides
                                            .insert(name.clone(), new_val);
                                    }
                                }
                            }
                        }
                    });
                }
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove = true;
                ui.close();
            }
        });
    }

    remove
}

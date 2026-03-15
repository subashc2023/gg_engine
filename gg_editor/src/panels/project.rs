use gg_engine::egui;
use gg_engine::events::gamepad::{GamepadAxis, GamepadButton};
use gg_engine::events::{KeyCode, MouseButton};
use gg_engine::input_action::{ActionType, InputAction, InputActionMap, InputBinding};
use gg_engine::log;
use gg_engine::DeadZoneConfig;
use gg_engine::Project;
use std::path::{Path, PathBuf};

thread_local! {
    /// Cached scene list: (assets_root, scene_paths).
    static SCENE_CACHE: std::cell::RefCell<Option<(PathBuf, Vec<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Invalidate the cached scene list (call when project changes or scenes are saved/created).
pub(crate) fn invalidate_scene_cache() {
    SCENE_CACHE.with(|c| *c.borrow_mut() = None);
}

/// Recursively collect all `.ggscene` files under `dir`, returning paths
/// relative to `assets_root`.
fn collect_scenes(dir: &Path, assets_root: &Path) -> Vec<PathBuf> {
    let mut scenes = Vec::new();
    collect_scenes_recursive(dir, assets_root, &mut scenes);
    scenes.sort();
    scenes
}

fn collect_scenes_recursive(dir: &Path, assets_root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_scenes_recursive(&path, assets_root, out);
        } else if path.extension().is_some_and(|ext| ext == "ggscene") {
            if let Ok(relative) = path.strip_prefix(assets_root) {
                out.push(relative.to_path_buf());
            } else {
                out.push(path);
            }
        }
    }
}

pub(crate) fn project_ui(
    ui: &mut egui::Ui,
    project_name: Option<&str>,
    assets_root: &Path,
    editor_scene_path: Option<&str>,
    pending_open_path: &mut Option<PathBuf>,
    input_actions: &mut InputActionMap,
    dead_zones: &mut [f32; GamepadAxis::COUNT],
    project: &mut Option<Project>,
) {
    let Some(name) = project_name else {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(
                egui::RichText::new("No project loaded")
                    .color(egui::Color32::from_rgb(0x88, 0x88, 0x88))
                    .italics(),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("File > Open Project...")
                    .color(egui::Color32::from_rgb(0x66, 0x66, 0x66))
                    .small(),
            );
        });
        return;
    };

    ui.heading(name);
    ui.separator();

    let scenes = SCENE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let needs_refresh = match &*cache {
            Some((cached_root, _)) => *cached_root != assets_root,
            None => true,
        };
        if needs_refresh {
            let collected = collect_scenes(assets_root, assets_root);
            *cache = Some((assets_root.to_path_buf(), collected));
        }
        cache.as_ref().unwrap().1.clone()
    });

    if scenes.is_empty() {
        ui.label(
            egui::RichText::new("No scenes found").color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
        );
    } else {
        // Resolve current scene path once, not per-scene.
        let current_relative: Option<PathBuf> = editor_scene_path.and_then(|p| {
            let p = PathBuf::from(p);
            p.strip_prefix(assets_root).ok().map(|r| r.to_path_buf())
        });

        ui.label(
            egui::RichText::new("Scenes")
                .small()
                .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
        );
        ui.add_space(2.0);

        for relative in &scenes {
            let abs_path = assets_root.join(relative);
            let is_current = current_relative
                .as_ref()
                .is_some_and(|current| current == relative);

            let display_name = relative.to_string_lossy();
            let response = ui.selectable_label(is_current, display_name.as_ref());

            if response.clicked() && !is_current {
                *pending_open_path = Some(abs_path);
            }
        }
    }

    ui.add_space(8.0);
    ui.separator();

    // --- Input Actions ---
    input_actions_ui(ui, input_actions, project);

    ui.add_space(8.0);
    ui.separator();

    // --- Dead Zones ---
    dead_zones_ui(ui, dead_zones, project);
}

// ---------------------------------------------------------------------------
// Input Actions UI
// ---------------------------------------------------------------------------

fn input_actions_ui(
    ui: &mut egui::Ui,
    actions: &mut InputActionMap,
    project: &mut Option<Project>,
) {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Input Actions")
                .small()
                .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Add Action").clicked() {
                actions.actions.push(InputAction {
                    name: format!("action_{}", actions.actions.len()),
                    action_type: ActionType::Button,
                    bindings: Vec::new(),
                });
                changed = true;
            }
        });
    });
    ui.add_space(2.0);

    if actions.actions.is_empty() {
        ui.label(
            egui::RichText::new("No actions defined")
                .color(egui::Color32::from_rgb(0x66, 0x66, 0x66))
                .italics(),
        );
    }

    let mut remove_idx = None;

    for (i, action) in actions.actions.iter_mut().enumerate() {
        let id = ui.make_persistent_id(format!("input_action_{i}"));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                let type_label = match action.action_type {
                    ActionType::Button => "Btn",
                    ActionType::Axis => "Axis",
                };
                ui.label(
                    egui::RichText::new(format!("[{}]", type_label))
                        .small()
                        .color(egui::Color32::from_rgb(0x88, 0xAA, 0xCC)),
                );
                ui.label(&action.name);
            })
            .body(|ui| {
                // Name field.
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    if ui
                        .text_edit_singleline(&mut action.name)
                        .on_hover_text(
                            "Action name used in Lua (e.g. Engine.is_action_pressed(\"jump\"))",
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });

                // Type selector.
                ui.horizontal(|ui| {
                    ui.label("Type:");
                    let mut type_idx = match action.action_type {
                        ActionType::Button => 0,
                        ActionType::Axis => 1,
                    };
                    if egui::ComboBox::from_id_salt(format!("action_type_{i}"))
                        .width(80.0)
                        .show_index(ui, &mut type_idx, 2, |i| ["Button", "Axis"][i].to_string())
                        .changed()
                    {
                        action.action_type = match type_idx {
                            0 => ActionType::Button,
                            _ => ActionType::Axis,
                        };
                        changed = true;
                    }
                });

                // Bindings list.
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Bindings")
                        .small()
                        .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
                );

                let mut remove_binding = None;
                for (bi, binding) in action.bindings.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("x")
                            .on_hover_text("Remove binding")
                            .clicked()
                        {
                            remove_binding = Some(bi);
                        }
                        if binding_ui(ui, binding, i, bi) {
                            changed = true;
                        }
                    });
                }
                if let Some(bi) = remove_binding {
                    action.bindings.remove(bi);
                    changed = true;
                }

                // Add binding button.
                ui.horizontal(|ui| {
                    if ui.small_button("+ Binding").clicked() {
                        action.bindings.push(InputBinding::Key(KeyCode::Space));
                        changed = true;
                    }
                    ui.add_space(8.0);
                    if ui
                        .small_button("Delete Action")
                        .on_hover_text("Remove this action")
                        .clicked()
                    {
                        remove_idx = Some(i);
                    }
                });
            });
    }

    if let Some(idx) = remove_idx {
        actions.actions.remove(idx);
        changed = true;
    }

    // Persist changes to the project file.
    if changed {
        if let Some(proj) = project.as_mut() {
            proj.config_mut().input_actions = actions.clone();
            if let Err(e) = proj.save() {
                log::error!("Failed to save project after input action change: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dead Zones UI
// ---------------------------------------------------------------------------

const DEAD_ZONE_AXIS_LABELS: &[(&str, usize)] = &[
    ("Left Stick X", 0),
    ("Left Stick Y", 1),
    ("Right Stick X", 2),
    ("Right Stick Y", 3),
    ("Left Trigger", 4),
    ("Right Trigger", 5),
];

fn dead_zones_ui(
    ui: &mut egui::Ui,
    dead_zones: &mut [f32; GamepadAxis::COUNT],
    project: &mut Option<Project>,
) {
    let mut changed = false;

    ui.label(
        egui::RichText::new("Gamepad Dead Zones")
            .small()
            .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
    );
    ui.add_space(2.0);

    for &(label, idx) in DEAD_ZONE_AXIS_LABELS {
        ui.horizontal(|ui| {
            ui.label(label);
            if ui
                .add(
                    egui::DragValue::new(&mut dead_zones[idx])
                        .range(0.0..=0.9)
                        .speed(0.005)
                        .fixed_decimals(2),
                )
                .changed()
            {
                changed = true;
            }
        });
    }

    if changed {
        if let Some(proj) = project.as_mut() {
            proj.config_mut().dead_zones = DeadZoneConfig::from_array(*dead_zones);
            if let Err(e) = proj.save() {
                log::error!("Failed to save project after dead zone change: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Single binding UI
// ---------------------------------------------------------------------------

/// Binding type index for the ComboBox selector.
const BINDING_TYPES: [&str; 6] = [
    "Key",
    "Mouse",
    "GamepadButton",
    "GamepadAxisAsBtn",
    "GamepadAxis",
    "KeyComposite",
];

fn binding_type_index(binding: &InputBinding) -> usize {
    match binding {
        InputBinding::Key(_) => 0,
        InputBinding::Mouse(_) => 1,
        InputBinding::GamepadButton { .. } => 2,
        InputBinding::GamepadAxisAsButton { .. } => 3,
        InputBinding::GamepadAxis { .. } => 4,
        InputBinding::KeyComposite { .. } => 5,
    }
}

fn binding_ui(
    ui: &mut egui::Ui,
    binding: &mut InputBinding,
    action_idx: usize,
    binding_idx: usize,
) -> bool {
    let mut changed = false;
    let id_salt = format!("binding_{action_idx}_{binding_idx}");

    // Binding type selector.
    let mut type_idx = binding_type_index(binding);
    if egui::ComboBox::from_id_salt(format!("{id_salt}_type"))
        .width(110.0)
        .show_index(ui, &mut type_idx, BINDING_TYPES.len(), |i| {
            BINDING_TYPES[i].to_string()
        })
        .changed()
    {
        // Convert to new binding type with defaults.
        *binding = match type_idx {
            0 => InputBinding::Key(KeyCode::Space),
            1 => InputBinding::Mouse(MouseButton::Left),
            2 => InputBinding::GamepadButton {
                button: GamepadButton::South,
                gamepad_id: None,
            },
            3 => InputBinding::GamepadAxisAsButton {
                axis: GamepadAxis::RightTrigger,
                threshold: 0.5,
                gamepad_id: None,
            },
            4 => InputBinding::GamepadAxis {
                axis: GamepadAxis::LeftStickX,
                dead_zone: 0.15,
                scale: 1.0,
                gamepad_id: None,
            },
            _ => InputBinding::KeyComposite {
                negative: KeyCode::A,
                positive: KeyCode::D,
            },
        };
        changed = true;
    }

    // Type-specific fields.
    match binding {
        InputBinding::Key(key) => {
            changed |= key_combo(ui, key, &format!("{id_salt}_key"));
        }
        InputBinding::Mouse(btn) => {
            changed |= mouse_combo(ui, btn, &format!("{id_salt}_mouse"));
        }
        InputBinding::GamepadButton {
            button,
            gamepad_id: _,
        } => {
            changed |= gamepad_button_combo(ui, button, &format!("{id_salt}_gpbtn"));
        }
        InputBinding::GamepadAxisAsButton {
            axis,
            threshold,
            gamepad_id: _,
        } => {
            changed |= gamepad_axis_combo(ui, axis, &format!("{id_salt}_gpaxis"));
            if ui
                .add(
                    egui::DragValue::new(threshold)
                        .range(-1.0..=1.0)
                        .speed(0.01)
                        .prefix("thr: "),
                )
                .changed()
            {
                changed = true;
            }
        }
        InputBinding::GamepadAxis {
            axis,
            dead_zone,
            scale,
            gamepad_id: _,
        } => {
            changed |= gamepad_axis_combo(ui, axis, &format!("{id_salt}_gpaxis"));
            if ui
                .add(
                    egui::DragValue::new(dead_zone)
                        .range(0.0..=0.9)
                        .speed(0.01)
                        .prefix("dz: "),
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .add(
                    egui::DragValue::new(scale)
                        .range(-2.0..=2.0)
                        .speed(0.01)
                        .prefix("s: "),
                )
                .changed()
            {
                changed = true;
            }
        }
        InputBinding::KeyComposite { negative, positive } => {
            ui.label("-");
            changed |= key_combo(ui, negative, &format!("{id_salt}_neg"));
            ui.label("+");
            changed |= key_combo(ui, positive, &format!("{id_salt}_pos"));
        }
    }

    changed
}

// ---------------------------------------------------------------------------
// ComboBox helpers for enums
// ---------------------------------------------------------------------------

const KEY_NAMES: &[(&str, KeyCode)] = &[
    ("A", KeyCode::A),
    ("B", KeyCode::B),
    ("C", KeyCode::C),
    ("D", KeyCode::D),
    ("E", KeyCode::E),
    ("F", KeyCode::F),
    ("G", KeyCode::G),
    ("H", KeyCode::H),
    ("I", KeyCode::I),
    ("J", KeyCode::J),
    ("K", KeyCode::K),
    ("L", KeyCode::L),
    ("M", KeyCode::M),
    ("N", KeyCode::N),
    ("O", KeyCode::O),
    ("P", KeyCode::P),
    ("Q", KeyCode::Q),
    ("R", KeyCode::R),
    ("S", KeyCode::S),
    ("T", KeyCode::T),
    ("U", KeyCode::U),
    ("V", KeyCode::V),
    ("W", KeyCode::W),
    ("X", KeyCode::X),
    ("Y", KeyCode::Y),
    ("Z", KeyCode::Z),
    ("0", KeyCode::Num0),
    ("1", KeyCode::Num1),
    ("2", KeyCode::Num2),
    ("3", KeyCode::Num3),
    ("4", KeyCode::Num4),
    ("5", KeyCode::Num5),
    ("6", KeyCode::Num6),
    ("7", KeyCode::Num7),
    ("8", KeyCode::Num8),
    ("9", KeyCode::Num9),
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("LShift", KeyCode::LeftShift),
    ("RShift", KeyCode::RightShift),
    ("LCtrl", KeyCode::LeftCtrl),
    ("RCtrl", KeyCode::RightCtrl),
    ("LAlt", KeyCode::LeftAlt),
    ("RAlt", KeyCode::RightAlt),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Space", KeyCode::Space),
    ("Enter", KeyCode::Enter),
    ("Escape", KeyCode::Escape),
    ("Tab", KeyCode::Tab),
    ("Backspace", KeyCode::Backspace),
    ("Delete", KeyCode::Delete),
    ("Insert", KeyCode::Insert),
];

fn key_combo(ui: &mut egui::Ui, key: &mut KeyCode, id: &str) -> bool {
    let mut idx = KEY_NAMES.iter().position(|(_, k)| k == key).unwrap_or(0);
    let resp = egui::ComboBox::from_id_salt(id).width(70.0).show_index(
        ui,
        &mut idx,
        KEY_NAMES.len(),
        |i| KEY_NAMES[i].0.to_string(),
    );
    if resp.changed() {
        *key = KEY_NAMES[idx].1;
    }
    resp.changed()
}

const MOUSE_NAMES: &[(&str, MouseButton)] = &[
    ("Left", MouseButton::Left),
    ("Right", MouseButton::Right),
    ("Middle", MouseButton::Middle),
    ("Back", MouseButton::Back),
    ("Forward", MouseButton::Forward),
];

fn mouse_combo(ui: &mut egui::Ui, btn: &mut MouseButton, id: &str) -> bool {
    let mut idx = MOUSE_NAMES.iter().position(|(_, b)| b == btn).unwrap_or(0);
    let resp = egui::ComboBox::from_id_salt(id).width(70.0).show_index(
        ui,
        &mut idx,
        MOUSE_NAMES.len(),
        |i| MOUSE_NAMES[i].0.to_string(),
    );
    if resp.changed() {
        *btn = MOUSE_NAMES[idx].1;
    }
    resp.changed()
}

const GAMEPAD_BUTTON_NAMES: &[(&str, GamepadButton)] = &[
    ("South/A", GamepadButton::South),
    ("East/B", GamepadButton::East),
    ("West/X", GamepadButton::West),
    ("North/Y", GamepadButton::North),
    ("LBumper", GamepadButton::LeftBumper),
    ("RBumper", GamepadButton::RightBumper),
    ("LTrigger", GamepadButton::LeftTrigger),
    ("RTrigger", GamepadButton::RightTrigger),
    ("Select", GamepadButton::Select),
    ("Start", GamepadButton::Start),
    ("Guide", GamepadButton::Guide),
    ("LStick", GamepadButton::LeftStick),
    ("RStick", GamepadButton::RightStick),
    ("DPadUp", GamepadButton::DPadUp),
    ("DPadDown", GamepadButton::DPadDown),
    ("DPadLeft", GamepadButton::DPadLeft),
    ("DPadRight", GamepadButton::DPadRight),
];

fn gamepad_button_combo(ui: &mut egui::Ui, btn: &mut GamepadButton, id: &str) -> bool {
    let mut idx = GAMEPAD_BUTTON_NAMES
        .iter()
        .position(|(_, b)| b == btn)
        .unwrap_or(0);
    let resp = egui::ComboBox::from_id_salt(id).width(80.0).show_index(
        ui,
        &mut idx,
        GAMEPAD_BUTTON_NAMES.len(),
        |i| GAMEPAD_BUTTON_NAMES[i].0.to_string(),
    );
    if resp.changed() {
        *btn = GAMEPAD_BUTTON_NAMES[idx].1;
    }
    resp.changed()
}

const GAMEPAD_AXIS_NAMES: &[(&str, GamepadAxis)] = &[
    ("LStickX", GamepadAxis::LeftStickX),
    ("LStickY", GamepadAxis::LeftStickY),
    ("RStickX", GamepadAxis::RightStickX),
    ("RStickY", GamepadAxis::RightStickY),
    ("LTrigger", GamepadAxis::LeftTrigger),
    ("RTrigger", GamepadAxis::RightTrigger),
];

fn gamepad_axis_combo(ui: &mut egui::Ui, axis: &mut GamepadAxis, id: &str) -> bool {
    let mut idx = GAMEPAD_AXIS_NAMES
        .iter()
        .position(|(_, a)| a == axis)
        .unwrap_or(0);
    let resp = egui::ComboBox::from_id_salt(id).width(80.0).show_index(
        ui,
        &mut idx,
        GAMEPAD_AXIS_NAMES.len(),
        |i| GAMEPAD_AXIS_NAMES[i].0.to_string(),
    );
    if resp.changed() {
        *axis = GAMEPAD_AXIS_NAMES[idx].1;
    }
    resp.changed()
}

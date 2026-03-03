use gg_engine::egui;
use std::path::{Path, PathBuf};

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

    let scenes = collect_scenes(assets_root, assets_root);

    if scenes.is_empty() {
        ui.label(
            egui::RichText::new("No scenes found")
                .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
        );
        return;
    }

    let current_abs: Option<PathBuf> = editor_scene_path
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)));

    ui.label(
        egui::RichText::new("Scenes")
            .small()
            .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
    );
    ui.add_space(2.0);

    for relative in &scenes {
        let abs_path = assets_root.join(relative);
        let is_current = current_abs.as_ref().is_some_and(|current| {
            let canon = std::fs::canonicalize(&abs_path).unwrap_or_else(|_| abs_path.clone());
            *current == canon
        });

        let display_name = relative.to_string_lossy();
        let response = ui.selectable_label(is_current, display_name.as_ref());

        if response.clicked() && !is_current {
            *pending_open_path = Some(abs_path);
        }
    }
}

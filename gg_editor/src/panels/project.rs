use gg_engine::egui;
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
            egui::RichText::new("No scenes found")
                .color(egui::Color32::from_rgb(0x88, 0x88, 0x88)),
        );
        return;
    }

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
        let is_current = current_relative.as_ref().is_some_and(|current| current == relative);

        let display_name = relative.to_string_lossy();
        let response = ui.selectable_label(is_current, display_name.as_ref());

        if response.clicked() && !is_current {
            *pending_open_path = Some(abs_path);
        }
    }
}

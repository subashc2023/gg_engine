use std::path::{Path, PathBuf};

use gg_engine::egui;
use gg_engine::log;
use gg_engine::prelude::*;
use gg_engine::{cook_assets, BuildManifest};

use super::GGEditor;

// ---------------------------------------------------------------------------
// Build configuration & result
// ---------------------------------------------------------------------------

pub(crate) struct BuildConfig {
    pub output_directory: String,
    pub build_name: String,
    pub strip_unused: bool,
}

pub(crate) enum BuildResult {
    Success {
        output_path: PathBuf,
        bytes_copied: u64,
        manifest: Option<BuildManifest>,
    },
    Error(String),
}

pub(crate) struct BuildModal {
    pub output_directory: String,
    pub build_name: String,
    pub strip_unused: bool,
    pub result: Option<BuildResult>,
}

// ---------------------------------------------------------------------------
// Player binary discovery
// ---------------------------------------------------------------------------

fn player_exe_name() -> &'static str {
    if cfg!(windows) {
        "gg_player.exe"
    } else {
        "gg_player"
    }
}

/// Locate the `gg_player` binary by searching common locations.
fn find_player_binary() -> Option<PathBuf> {
    let exe_name = player_exe_name();

    // 1. Search in target/ directories relative to CWD (workspace root during dev).
    for profile in &["dist", "release", "debug"] {
        let path = PathBuf::from(format!("target/{}/{}", profile, exe_name));
        if path.exists() {
            return Some(path);
        }
    }

    // 2. Next to the editor executable (distributed build).
    if let Ok(editor_exe) = std::env::current_exe() {
        if let Some(dir) = editor_exe.parent() {
            let path = dir.join(exe_name);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Recursive directory copy (legacy — used when strip_unused is off)
// ---------------------------------------------------------------------------

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let mut bytes_copied = 0u64;
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        // Skip autosave files.
        if let Some(name) = src_path.file_name().and_then(|n| n.to_str()) {
            if name.contains(".autosave.") {
                continue;
            }
        }

        if file_type.is_dir() {
            bytes_copied += copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            bytes_copied += std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(bytes_copied)
}

// ---------------------------------------------------------------------------
// Manifest-based copy (used when strip_unused is on)
// ---------------------------------------------------------------------------

/// Copy only the files listed in the manifest from `asset_dir` to `dest_assets`.
fn copy_from_manifest(
    asset_dir: &Path,
    dest_assets: &Path,
    manifest: &BuildManifest,
) -> std::io::Result<u64> {
    let mut bytes_copied = 0u64;

    for entry in &manifest.entries {
        let src = asset_dir.join(&entry.path);
        let dst = dest_assets.join(&entry.path);

        // Ensure parent directory exists.
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }

        if src.exists() {
            bytes_copied += std::fs::copy(&src, &dst)?;
        }
    }

    Ok(bytes_copied)
}

// ---------------------------------------------------------------------------
// Build execution
// ---------------------------------------------------------------------------

pub(crate) fn execute_build(config: &BuildConfig, project: &Project) -> BuildResult {
    let output_dir = PathBuf::from(&config.output_directory);

    // Create the output directory.
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        return BuildResult::Error(format!("Failed to create output directory: {}", e));
    }

    // Find and copy the player binary.
    let player_binary = match find_player_binary() {
        Some(path) => path,
        None => {
            return BuildResult::Error(
                "Could not find gg_player binary.\n\
                 Build it first with:\n  cargo build --release -p gg_player"
                    .to_string(),
            );
        }
    };

    let exe_extension = if cfg!(windows) { ".exe" } else { "" };
    let dest_exe = output_dir.join(format!("{}{}", config.build_name, exe_extension));
    if let Err(e) = std::fs::copy(&player_binary, &dest_exe) {
        return BuildResult::Error(format!("Failed to copy player binary: {}", e));
    }

    let mut total_bytes = dest_exe.metadata().map(|m| m.len()).unwrap_or(0);

    // Copy the .ggproject file.
    let project_file = PathBuf::from(project.project_file_path());
    let project_file_name = project_file
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("game.ggproject"));
    let dest_project = output_dir.join(project_file_name);
    if let Err(e) = std::fs::copy(&project_file, &dest_project) {
        return BuildResult::Error(format!("Failed to copy project file: {}", e));
    }
    total_bytes += dest_project.metadata().map(|m| m.len()).unwrap_or(0);

    // Copy the assets directory.
    let asset_dir = project.asset_directory_path();
    let asset_dir_name = project.config().asset_directory.clone();
    let dest_assets = output_dir.join(&asset_dir_name);

    if !asset_dir.exists() {
        return BuildResult::Success {
            output_path: output_dir,
            bytes_copied: total_bytes,
            manifest: None,
        };
    }

    if config.strip_unused {
        // Compute script module subdir relative to asset dir.
        let script_module_path = project.script_module_path();
        let script_module_subdir = script_module_path
            .strip_prefix(&asset_dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| "scripts".to_string());

        let manifest = cook_assets(&asset_dir, &script_module_subdir);

        for warning in &manifest.warnings {
            log::warn!("Build warning: {}", warning);
        }

        match copy_from_manifest(&asset_dir, &dest_assets, &manifest) {
            Ok(bytes) => total_bytes += bytes,
            Err(e) => {
                return BuildResult::Error(format!("Failed to copy assets: {}", e));
            }
        }

        log::info!(
            "Build complete: {} ({:.1} MB, {} files included, {} excluded)",
            output_dir.display(),
            total_bytes as f64 / (1024.0 * 1024.0),
            manifest.entries.len(),
            manifest.excluded.len(),
        );

        BuildResult::Success {
            output_path: output_dir,
            bytes_copied: total_bytes,
            manifest: Some(manifest),
        }
    } else {
        match copy_dir_recursive(&asset_dir, &dest_assets) {
            Ok(bytes) => total_bytes += bytes,
            Err(e) => {
                return BuildResult::Error(format!("Failed to copy assets: {}", e));
            }
        }

        log::info!(
            "Build complete: {} ({:.1} MB)",
            output_dir.display(),
            total_bytes as f64 / (1024.0 * 1024.0)
        );

        BuildResult::Success {
            output_path: output_dir,
            bytes_copied: total_bytes,
            manifest: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Build modal UI
// ---------------------------------------------------------------------------

/// Format bytes as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

impl GGEditor {
    pub(super) fn build_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(ref mut modal) = self.ui.build_modal else {
            return;
        };

        let mut should_close = false;
        let mut should_build = false;

        // Dim background.
        let screen_rect = ctx.input(|i| i.viewport_rect());
        egui::Area::new(egui::Id::new("build_modal_bg"))
            .fixed_pos(screen_rect.left_top())
            .show(ctx, |ui| {
                ui.allocate_response(screen_rect.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(128));
            });

        egui::Window::new("Build Project")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(450.0, 0.0))
            .show(ctx, |ui| {
                if let Some(ref result) = modal.result {
                    // -- Result display --
                    match result {
                        BuildResult::Success {
                            output_path,
                            bytes_copied,
                            manifest,
                        } => {
                            ui.colored_label(
                                egui::Color32::from_rgb(0x4E, 0xC9, 0xB0),
                                format!("Build successful! ({})", format_size(*bytes_copied)),
                            );
                            ui.add_space(4.0);
                            ui.label(output_path.display().to_string());

                            // Show manifest details if asset stripping was used.
                            if let Some(manifest) = manifest {
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);

                                ui.label(
                                    egui::RichText::new("Asset Summary").strong(),
                                );
                                ui.add_space(2.0);

                                // Size breakdown by category.
                                let sizes = manifest.size_by_category();
                                let mut size_entries: Vec<_> = sizes.iter().collect();
                                size_entries.sort_by(|a, b| b.1.cmp(a.1));

                                egui::Grid::new("build_size_grid")
                                    .num_columns(2)
                                    .spacing([20.0, 2.0])
                                    .show(ui, |ui| {
                                        for (category, bytes) in &size_entries {
                                            ui.label(category.label());
                                            ui.label(format_size(**bytes));
                                            ui.end_row();
                                        }
                                    });

                                ui.add_space(4.0);
                                ui.label(format!(
                                    "{} files included, {} excluded (saved {})",
                                    manifest.entries.len(),
                                    manifest.excluded.len(),
                                    format_size(manifest.total_excluded_bytes),
                                ));

                                // Show warnings if any.
                                if !manifest.warnings.is_empty() {
                                    ui.add_space(4.0);
                                    for warning in &manifest.warnings {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(0xFF, 0xCC, 0x00),
                                            format!("Warning: {}", warning),
                                        );
                                    }
                                }
                            }

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button("Open Folder").clicked() {
                                    open_folder(output_path);
                                }
                                if ui.button("Close").clicked() {
                                    should_close = true;
                                }
                            });
                        }
                        BuildResult::Error(msg) => {
                            ui.colored_label(
                                egui::Color32::from_rgb(0xF4, 0x80, 0x71),
                                format!("Build failed:\n{}", msg),
                            );
                            ui.add_space(8.0);
                            if ui.button("Close").clicked() {
                                should_close = true;
                            }
                        }
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        should_close = true;
                    }
                } else {
                    // -- Configuration form --
                    ui.label("Output Directory:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut modal.output_directory)
                                .desired_width(ui.available_width() - 80.0)
                                .hint_text("Select output folder..."),
                        );
                        if ui.button("Browse...").clicked() {
                            if let Some(folder) = FileDialogs::pick_folder() {
                                modal.output_directory = folder;
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.label("Build Name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut modal.build_name)
                            .desired_width(ui.available_width()),
                    );
                    ui.label(
                        egui::RichText::new("(Used as the executable name)")
                            .small()
                            .weak(),
                    );

                    ui.add_space(8.0);
                    ui.checkbox(&mut modal.strip_unused, "Strip unused assets");
                    ui.label(
                        egui::RichText::new(
                            "Only include assets referenced by scenes and scripts",
                        )
                        .small()
                        .weak(),
                    );

                    ui.add_space(12.0);

                    let can_build =
                        !modal.output_directory.is_empty() && !modal.build_name.trim().is_empty();

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(can_build, egui::Button::new("Build"))
                            .clicked()
                        {
                            should_build = true;
                        }
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }
                    });

                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        should_close = true;
                    }
                }
            });

        if should_build {
            let config = BuildConfig {
                output_directory: modal.output_directory.clone(),
                build_name: modal.build_name.trim().to_string(),
                strip_unused: modal.strip_unused,
            };
            let result = execute_build(&config, self.project_state.project.as_ref().unwrap());
            modal.result = Some(result);
        }

        if should_close {
            self.ui.build_modal = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific folder open
// ---------------------------------------------------------------------------

fn open_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) const ASSETS_DIR: &str = "assets";

// ---------------------------------------------------------------------------
// Content browser mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum ContentBrowserMode {
    FileSystem,
    Asset,
}

// Thread-local state for the content browser mode toggle and directory cache.
// (Stored here because the panel function is stateless otherwise.)
thread_local! {
    static BROWSER_MODE: std::cell::Cell<ContentBrowserMode> =
        const { std::cell::Cell::new(ContentBrowserMode::FileSystem) };
    /// Cached directory listing: (cached_path, directories, files).
    static DIR_CACHE: std::cell::RefCell<Option<(
        std::path::PathBuf,
        Vec<(String, std::path::PathBuf)>,
        Vec<(String, std::path::PathBuf)>,
    )>> = const { std::cell::RefCell::new(None) };
}

/// Invalidate the cached directory listing (call on file changes).
#[allow(dead_code)]
pub(crate) fn invalidate_dir_cache() {
    DIR_CACHE.with(|c| *c.borrow_mut() = None);
}

// ---------------------------------------------------------------------------
// Content browser drag-and-drop payload
// ---------------------------------------------------------------------------

pub(crate) struct ContentBrowserPayload {
    pub(crate) path: std::path::PathBuf,
    pub(crate) name: String,
    pub(crate) is_directory: bool,
}

// ---------------------------------------------------------------------------
// Content browser panel
// ---------------------------------------------------------------------------

pub(crate) fn content_browser_ui(
    ui: &mut egui::Ui,
    current_directory: &mut std::path::PathBuf,
    assets_root: &std::path::Path,
    asset_manager: &mut Option<EditorAssetManager>,
) {
    let assets_root = assets_root.to_path_buf();

    // Mode toggle (File / Asset).
    let mut mode = BROWSER_MODE.with(|m| m.get());
    ui.horizontal(|ui| {
        if ui.selectable_label(mode == ContentBrowserMode::FileSystem, "File").clicked() {
            mode = ContentBrowserMode::FileSystem;
        }
        if ui.selectable_label(mode == ContentBrowserMode::Asset, "Asset").clicked() {
            mode = ContentBrowserMode::Asset;
        }
    });
    BROWSER_MODE.with(|m| m.set(mode));
    ui.separator();

    match mode {
        ContentBrowserMode::FileSystem => {
            file_browser_ui(ui, current_directory, &assets_root, asset_manager);
        }
        ContentBrowserMode::Asset => {
            asset_browser_ui(ui, &assets_root, asset_manager);
        }
    }
}

// ---------------------------------------------------------------------------
// File browser (original behavior + right-click import)
// ---------------------------------------------------------------------------

fn file_browser_ui(
    ui: &mut egui::Ui,
    current_directory: &mut std::path::PathBuf,
    assets_root: &std::path::Path,
    asset_manager: &mut Option<EditorAssetManager>,
) {
    let assets_root = assets_root.to_path_buf();

    // Back button — only when deeper than the assets root.
    if *current_directory != assets_root {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let hovered = response.hovered();
            let color = if hovered {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(0xCC, 0xCC, 0xCC)
            };
            paint_back_arrow(ui.painter(), rect, color);
        }
        if response.clicked() {
            if let Some(parent) = current_directory.parent() {
                *current_directory = parent.to_path_buf();
            }
        }
        ui.add_space(2.0);
    }

    // Collect and sort directory entries (cached, invalidated on directory change).
    let (directories, files) = DIR_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let needs_refresh = match &*cache {
            Some((cached_path, _, _)) => *cached_path != *current_directory,
            None => true,
        };
        if needs_refresh {
            let mut dirs = Vec::new();
            let mut fls = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&*current_directory) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if path.is_dir() {
                        dirs.push((name, path));
                    } else {
                        fls.push((name, path));
                    }
                }
            }
            dirs.sort_by(|a, b| a.0.cmp(&b.0));
            fls.sort_by(|a, b| a.0.cmp(&b.0));
            *cache = Some((current_directory.clone(), dirs, fls));
        }
        let (_, dirs, fls) = cache.as_ref().unwrap();
        (dirs.clone(), fls.clone())
    });

    let padding = 16.0;
    let button_size = 64.0;
    let cell_size = button_size + padding;

    ui.add_space(4.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        let available_width = ui.available_width();
        let columns = ((available_width / cell_size) as usize).max(1);

        let label_font = egui::FontId::new(11.0, egui::FontFamily::Proportional);

        let mut col = 0;
        let mut navigate_to: Option<std::path::PathBuf> = None;

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(padding * 0.5, padding * 0.5);

            // -- Directories --
            for (name, path) in &directories {
                let response = ui.allocate_ui_with_layout(
                    egui::vec2(cell_size, cell_size + 14.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let btn = icon_button(ui, button_size, |painter, rect| {
                            paint_folder_icon(painter, rect);
                        });
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name).font(label_font.clone()),
                            )
                            .truncate(),
                        );
                        btn
                    },
                );
                if response.inner.double_clicked() {
                    navigate_to = Some(path.clone());
                }

                // Drag source — set payload only while dragging.
                if response.inner.drag_started() || response.inner.dragged() {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        ContentBrowserPayload {
                            path: path.clone(),
                            name: name.clone(),
                            is_directory: true,
                        },
                    );
                }

                col += 1;
                if col >= columns {
                    col = 0;
                }
            }

            // -- Files --
            for (name, path) in &files {
                let response = ui.allocate_ui_with_layout(
                    egui::vec2(cell_size, cell_size + 14.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let btn = icon_button(ui, button_size, |painter, rect| {
                            paint_file_icon(painter, rect);
                        });
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name).font(label_font.clone()),
                            )
                            .truncate(),
                        );
                        btn
                    },
                );

                // Drag source — set payload only while dragging.
                if response.inner.drag_started() || response.inner.dragged() {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        ContentBrowserPayload {
                            path: path.clone(),
                            name: name.clone(),
                            is_directory: false,
                        },
                    );
                }

                // Right-click context menu: Import into asset registry.
                response.inner.context_menu(|ui| {
                    if let Some(am) = asset_manager.as_mut() {
                        let rel_path = path
                            .strip_prefix(am.asset_directory())
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| path.to_string_lossy().to_string());
                        let already_imported = am.is_imported(&rel_path);
                        if already_imported {
                            ui.label("Already imported");
                        } else if ui.button("Import").clicked() {
                            am.import_asset(&rel_path);
                            am.save_registry();
                            ui.close();
                        }
                    } else {
                        ui.label("No project loaded");
                    }
                });

                col += 1;
                if col >= columns {
                    col = 0;
                }
            }
        });

        if let Some(path) = navigate_to {
            *current_directory = path;
        }
    });
}

// ---------------------------------------------------------------------------
// Asset browser (shows imported assets from registry)
// ---------------------------------------------------------------------------

fn asset_browser_ui(
    ui: &mut egui::Ui,
    _assets_root: &std::path::Path,
    asset_manager: &mut Option<EditorAssetManager>,
) {
    let Some(am) = asset_manager.as_mut() else {
        ui.label("No project loaded");
        return;
    };

    if am.registry().is_empty() {
        ui.label("No assets imported. Use File mode to import assets.");
        return;
    }

    // Collect and sort assets by path.
    let mut entries: Vec<(Uuid, String, AssetType)> = am
        .registry()
        .iter()
        .map(|(handle, meta)| (*handle, meta.file_path.clone(), meta.asset_type))
        .collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));

    // Track which handle to remove (if any) after iteration.
    let mut remove_handle: Option<Uuid> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (handle, file_path, asset_type) in &entries {
            let label = format!("[{:?}] {}", asset_type, file_path);

            let response = ui.selectable_label(false, &label);

            // Drag source for asset entries.
            if response.drag_started() || response.dragged() {
                let abs_path = am.asset_directory().join(file_path);
                let name = std::path::Path::new(file_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());
                egui::DragAndDrop::set_payload(
                    ui.ctx(),
                    ContentBrowserPayload {
                        path: abs_path,
                        name,
                        is_directory: false,
                    },
                );
            }

            // Right-click context menu: Remove from registry.
            response.context_menu(|ui| {
                if ui.button("Remove from registry").clicked() {
                    remove_handle = Some(*handle);
                    ui.close();
                }
            });
        }
    });

    // Process deferred removal.
    if let Some(handle) = remove_handle {
        am.registry_mut().remove(&handle);
        am.save_registry();
    }
}

// ---------------------------------------------------------------------------
// Icon painting helpers
// ---------------------------------------------------------------------------

/// Allocate a square button and paint a custom icon into it via `paint_fn`.
fn icon_button(
    ui: &mut egui::Ui,
    size: f32,
    paint_fn: impl FnOnce(&egui::Painter, egui::Rect),
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        // Button background.
        let bg = if response.hovered() {
            egui::Color32::from_rgb(0x3C, 0x3C, 0x3C)
        } else {
            egui::Color32::from_rgb(0x2A, 0x2A, 0x2A)
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), bg);
        paint_fn(ui.painter(), rect);
    }
    response
}

/// Paint a folder icon inside `rect`.
fn paint_folder_icon(painter: &egui::Painter, rect: egui::Rect) {
    let color = egui::Color32::from_rgb(0xDC, 0xDC, 0xAA);
    let cx = rect.center().x;
    let cy = rect.center().y;
    let w = rect.width() * 0.5;
    let h = rect.height() * 0.4;

    // Folder tab (small rectangle on top-left).
    let tab = egui::Rect::from_min_max(
        egui::pos2(cx - w, cy - h - 2.0),
        egui::pos2(cx - w * 0.3, cy - h + 4.0),
    );
    painter.rect_filled(tab, egui::CornerRadius::same(2), color);

    // Folder body.
    let body =
        egui::Rect::from_min_max(egui::pos2(cx - w, cy - h + 2.0), egui::pos2(cx + w, cy + h));
    painter.rect_filled(body, egui::CornerRadius::same(3), color);
}

/// Paint a file/document icon inside `rect`.
fn paint_file_icon(painter: &egui::Painter, rect: egui::Rect) {
    let color = egui::Color32::from_rgb(0x96, 0x96, 0x96);
    let cx = rect.center().x;
    let cy = rect.center().y;
    let w = rect.width() * 0.3;
    let h = rect.height() * 0.4;
    let fold = w * 0.4;

    // Page body.
    let body = egui::Rect::from_min_max(egui::pos2(cx - w, cy - h), egui::pos2(cx + w, cy + h));
    painter.rect_filled(body, egui::CornerRadius::same(2), color);

    // Corner fold (dark triangle in top-right).
    let fold_color = egui::Color32::from_rgb(0x60, 0x60, 0x60);
    let top_right = egui::pos2(cx + w, cy - h);
    let fold_points = vec![
        top_right,
        egui::pos2(cx + w - fold, cy - h),
        egui::pos2(cx + w, cy - h + fold),
    ];
    painter.add(egui::Shape::convex_polygon(
        fold_points,
        fold_color,
        egui::Stroke::NONE,
    ));

    // Two "text lines" inside the page.
    let line_color = egui::Color32::from_rgb(0x70, 0x70, 0x70);
    let lx = cx - w * 0.6;
    let rx = cx + w * 0.5;
    for i in 0..2 {
        let ly = cy - h * 0.1 + i as f32 * h * 0.45;
        painter.line_segment(
            [egui::pos2(lx, ly), egui::pos2(rx, ly)],
            egui::Stroke::new(1.5, line_color),
        );
    }
}

/// Paint a left-pointing back arrow inside `rect`.
fn paint_back_arrow(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let cx = rect.center().x;
    let cy = rect.center().y;
    let s = rect.height() * 0.3;

    // Arrow head (left-pointing chevron).
    let points = vec![
        egui::pos2(cx - s, cy),
        egui::pos2(cx + s * 0.3, cy - s),
        egui::pos2(cx + s * 0.3, cy + s),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));

    // Arrow shaft.
    painter.line_segment(
        [egui::pos2(cx - s * 0.6, cy), egui::pos2(cx + s, cy)],
        egui::Stroke::new(2.0, color),
    );
}

// ---------------------------------------------------------------------------
// DnD ghost overlay
// ---------------------------------------------------------------------------

pub(crate) fn render_dnd_ghost(ctx: &egui::Context) {
    let Some(payload) = egui::DragAndDrop::payload::<ContentBrowserPayload>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_latest_pos() else {
        return;
    };

    egui::Area::new(egui::Id::new("content_browser_dnd_ghost"))
        .order(egui::Order::Tooltip)
        .current_pos(pos + egui::vec2(14.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(egui::Color32::from_rgba_unmultiplied(0x2A, 0x2A, 0x2A, 0xE0))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_max_width(200.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon_rect, _) = ui.allocate_exact_size(
                            egui::vec2(16.0, 16.0),
                            egui::Sense::hover(),
                        );
                        if payload.is_directory {
                            paint_folder_icon(ui.painter(), icon_rect);
                        } else {
                            paint_file_icon(ui.painter(), icon_rect);
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&payload.name)
                                    .font(egui::FontId::new(
                                        11.0,
                                        egui::FontFamily::Proportional,
                                    ))
                                    .color(egui::Color32::from_rgb(0xCC, 0xCC, 0xCC)),
                            )
                            .truncate(),
                        );
                    });
                });
        });
}

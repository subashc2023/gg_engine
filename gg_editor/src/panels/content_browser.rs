use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::scene::SceneSerializer;

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
    // Cached directory listing: (cached_path, directories, files).
    #[allow(clippy::type_complexity)]
    static DIR_CACHE: std::cell::RefCell<Option<(
        std::path::PathBuf,
        Vec<(String, std::path::PathBuf)>,
        Vec<(String, std::path::PathBuf)>,
    )>> = const { std::cell::RefCell::new(None) };
}

// Search filter for content browser.
thread_local! {
    static SEARCH_FILTER: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

// Rename state: (original_path, current_edit_string, whether we just entered rename mode).
thread_local! {
    static RENAME_STATE: std::cell::RefCell<Option<(std::path::PathBuf, String, bool)>> =
        const { std::cell::RefCell::new(None) };
    static DELETE_CONFIRM: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
    /// Pending asset removal: (handle, list of referencing entity descriptions).
    static ASSET_REMOVE_CONFIRM: std::cell::RefCell<Option<(Uuid, Vec<(String, &'static str)>)>> =
        std::cell::RefCell::new(None);
}

/// Clear the search filter string.
pub(crate) fn reset_search_filter() {
    SEARCH_FILTER.with(|f| f.borrow_mut().clear());
}

/// Invalidate the cached directory listing (call on file changes).
pub(crate) fn invalidate_dir_cache() {
    DIR_CACHE.with(|c| *c.borrow_mut() = None);
}

/// Clear rename/delete dialog state (call on project switch or editor reset).
pub(crate) fn reset_dialog_state() {
    BROWSER_MODE.with(|m| m.set(ContentBrowserMode::FileSystem));
    RENAME_STATE.with(|s| *s.borrow_mut() = None);
    DELETE_CONFIRM.with(|d| *d.borrow_mut() = None);
    ASSET_REMOVE_CONFIRM.with(|d| *d.borrow_mut() = None);
    reset_search_filter();
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
    scene: &Scene,
) {
    let assets_root = assets_root.to_path_buf();

    // Mode toggle (File / Asset).
    let mut mode = BROWSER_MODE.with(|m| m.get());
    ui.horizontal(|ui| {
        if ui
            .selectable_label(mode == ContentBrowserMode::FileSystem, "File")
            .clicked()
        {
            mode = ContentBrowserMode::FileSystem;
        }
        if ui
            .selectable_label(mode == ContentBrowserMode::Asset, "Asset")
            .clicked()
        {
            mode = ContentBrowserMode::Asset;
        }
    });
    BROWSER_MODE.with(|m| m.set(mode));

    // Search / filter field.
    SEARCH_FILTER.with(|f| {
        let mut filter = f.borrow_mut();
        ui.horizontal(|ui| {
            let te = egui::TextEdit::singleline(&mut *filter)
                .desired_width(ui.available_width() - 22.0)
                .hint_text("Search...");
            ui.add(te);
            if ui
                .add_enabled(!filter.is_empty(), egui::Button::new("\u{2715}").small())
                .clicked()
            {
                filter.clear();
            }
        });
    });

    ui.separator();

    match mode {
        ContentBrowserMode::FileSystem => {
            file_browser_ui(ui, current_directory, &assets_root, asset_manager);
        }
        ContentBrowserMode::Asset => {
            asset_browser_ui(ui, &assets_root, asset_manager, scene);
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
        let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
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

    // Apply search filter (case-insensitive substring match on name).
    let filter = SEARCH_FILTER.with(|f| f.borrow().clone());
    let (directories, files) = if filter.is_empty() {
        (directories, files)
    } else {
        let filter_lower = filter.to_lowercase();
        let dirs = directories
            .into_iter()
            .filter(|(name, _)| name.to_lowercase().contains(&filter_lower))
            .collect();
        let fls = files
            .into_iter()
            .filter(|(name, _)| name.to_lowercase().contains(&filter_lower))
            .collect();
        (dirs, fls)
    };

    let padding = 16.0;
    let button_size = 64.0;
    let cell_size = button_size + padding;

    ui.add_space(4.0);

    // Deferred file operations (collected during iteration, applied after).
    let mut deferred_rename: Option<(std::path::PathBuf, String)> = None;
    let mut deferred_create: Option<(String, &str)> = None; // (filename, template content)
    let mut deferred_mkdir: Option<String> = None;

    // Check if we're currently in rename mode for any item.
    let rename_path = RENAME_STATE.with(|s| s.borrow().as_ref().map(|(p, _, _)| p.clone()));

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
                let is_renaming = rename_path.as_ref() == Some(path);

                let response = ui.allocate_ui_with_layout(
                    egui::vec2(cell_size, cell_size + 14.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let btn = icon_button(ui, button_size, |painter, rect| {
                            paint_folder_icon(painter, rect);
                        });
                        if is_renaming {
                            render_rename_field(ui, path, &mut deferred_rename);
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name).font(label_font.clone()),
                                )
                                .truncate(),
                            );
                        }
                        btn
                    },
                );
                if !is_renaming && response.inner.double_clicked() {
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

                // Right-click context menu for directories.
                if !is_renaming {
                    response.inner.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            navigate_to = Some(path.clone());
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Rename").clicked() {
                            RENAME_STATE.with(|s| {
                                *s.borrow_mut() = Some((path.clone(), name.clone(), true));
                            });
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            DELETE_CONFIRM.with(|d| {
                                *d.borrow_mut() = Some(path.clone());
                            });
                            ui.close();
                        }
                    });
                }

                col += 1;
                if col >= columns {
                    col = 0;
                }
            }

            // -- Files --
            for (name, path) in &files {
                let is_renaming = rename_path.as_ref() == Some(path);

                let response = ui.allocate_ui_with_layout(
                    egui::vec2(cell_size, cell_size + 14.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let btn = icon_button(ui, button_size, |painter, rect| {
                            paint_file_icon(painter, rect);
                        });
                        if is_renaming {
                            render_rename_field(ui, path, &mut deferred_rename);
                        } else {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name).font(label_font.clone()),
                                )
                                .truncate(),
                            );
                        }
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

                // Right-click context menu for files.
                if !is_renaming {
                    response.inner.context_menu(|ui| {
                        if let Some(am) = asset_manager.as_mut() {
                            let rel_path = super::relative_asset_path(&path, am.asset_directory());
                            let already_imported = am.is_imported(&rel_path);
                            if already_imported {
                                ui.label("Already imported");
                            } else if ui.button("Import").clicked() {
                                am.import_asset(&rel_path);
                                am.save_registry();
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button("Rename").clicked() {
                            RENAME_STATE.with(|s| {
                                *s.borrow_mut() = Some((path.clone(), name.clone(), true));
                            });
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            DELETE_CONFIRM.with(|d| {
                                *d.borrow_mut() = Some(path.clone());
                            });
                            ui.close();
                        }
                    });
                }

                col += 1;
                if col >= columns {
                    col = 0;
                }
            }
        });

        // Blank space context menu — create new files/folders.
        let remaining = ui.available_rect_before_wrap();
        let bg_response = ui.allocate_rect(remaining, egui::Sense::click());
        bg_response.context_menu(|ui| {
            if ui.button("New Folder").clicked() {
                deferred_mkdir = Some("New Folder".to_string());
                ui.close();
            }
            if ui.button("New Lua Script").clicked() {
                deferred_create = Some((
                    "new_script.lua".to_string(),
                    "function on_create()\nend\n\nfunction on_update(dt)\nend\n",
                ));
                ui.close();
            }
            if ui.button("New Scene").clicked() {
                deferred_create = Some(("New Scene.ggscene".to_string(), ""));
                ui.close();
            }
        });

        if let Some(path) = navigate_to {
            *current_directory = path;
        }
    });

    // -- Apply deferred file operations --

    if let Some((old_path, new_name)) = deferred_rename {
        let new_path = old_path.parent().unwrap_or(&old_path).join(&new_name);
        if new_path != old_path {
            let _ = std::fs::rename(&old_path, &new_path);
        }
        RENAME_STATE.with(|s| *s.borrow_mut() = None);
        invalidate_dir_cache();
    }

    if let Some(name) = deferred_mkdir {
        let new_dir = current_directory.join(&name);
        let _ = std::fs::create_dir_all(&new_dir);
        RENAME_STATE.with(|s| {
            *s.borrow_mut() = Some((new_dir, name, true));
        });
        invalidate_dir_cache();
    }

    if let Some((filename, template)) = deferred_create {
        let new_path = current_directory.join(&filename);
        if filename.ends_with(".ggscene") {
            let scene = gg_engine::scene::Scene::new();
            let name = std::path::Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled");
            SceneSerializer::serialize(&scene, &new_path.to_string_lossy(), Some(name));
        } else {
            let _ = std::fs::write(&new_path, template);
        }
        RENAME_STATE.with(|s| {
            *s.borrow_mut() = Some((new_path, filename, true));
        });
        invalidate_dir_cache();
    }

    // Delete confirmation window.
    let delete_path = DELETE_CONFIRM.with(|d| d.borrow().clone());
    if let Some(ref path) = delete_path {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut open = true;
        egui::Window::new("Confirm Delete")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(format!("Delete \"{}\"?", name));
                if path.is_dir() {
                    ui.label("This will delete the folder and all its contents.");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        if path.is_dir() {
                            let _ = std::fs::remove_dir_all(path);
                        } else {
                            let _ = std::fs::remove_file(path);
                        }
                        DELETE_CONFIRM.with(|d| *d.borrow_mut() = None);
                        invalidate_dir_cache();
                    }
                    if ui.button("Cancel").clicked() {
                        DELETE_CONFIRM.with(|d| *d.borrow_mut() = None);
                    }
                });
            });
        if !open {
            DELETE_CONFIRM.with(|d| *d.borrow_mut() = None);
        }
    }
}

// ---------------------------------------------------------------------------
// Asset browser (shows imported assets from registry)
// ---------------------------------------------------------------------------

fn asset_browser_ui(
    ui: &mut egui::Ui,
    _assets_root: &std::path::Path,
    asset_manager: &mut Option<EditorAssetManager>,
    scene: &Scene,
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

    // Apply search filter (case-insensitive substring match on file_path).
    let filter = SEARCH_FILTER.with(|f| f.borrow().clone());
    if !filter.is_empty() {
        let filter_lower = filter.to_lowercase();
        entries.retain(|(_, file_path, _)| file_path.to_lowercase().contains(&filter_lower));
    }

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

    // Process deferred removal — check for references first.
    if let Some(handle) = remove_handle {
        let refs = scene.find_asset_references(handle);
        if refs.is_empty() {
            am.registry_mut().remove(&handle);
            am.save_registry();
        } else {
            ASSET_REMOVE_CONFIRM.with(|d| *d.borrow_mut() = Some((handle, refs)));
        }
    }

    // Asset removal confirmation dialog (shown when references exist).
    let pending = ASSET_REMOVE_CONFIRM.with(|d| d.borrow().clone());
    if let Some((handle, refs)) = pending {
        let mut open = true;
        egui::Window::new("Remove Asset")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label("This asset is referenced by entities in the current scene:");
                ui.add_space(4.0);
                for (name, kind) in &refs {
                    ui.label(format!("  \u{2022} {} ({})", name, kind));
                }
                ui.add_space(4.0);
                ui.label("Removing it will leave broken references.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove Anyway").clicked() {
                        am.registry_mut().remove(&handle);
                        am.save_registry();
                        ASSET_REMOVE_CONFIRM.with(|d| *d.borrow_mut() = None);
                    }
                    if ui.button("Cancel").clicked() {
                        ASSET_REMOVE_CONFIRM.with(|d| *d.borrow_mut() = None);
                    }
                });
            });
        if !open {
            ASSET_REMOVE_CONFIRM.with(|d| *d.borrow_mut() = None);
        }
    }
}

// ---------------------------------------------------------------------------
// Inline rename text field
// ---------------------------------------------------------------------------

fn render_rename_field(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    deferred_rename: &mut Option<(std::path::PathBuf, String)>,
) {
    RENAME_STATE.with(|state| {
        let mut s = state.borrow_mut();
        let Some((ref rename_path, ref mut edit_text, ref mut first_frame)) = *s else {
            return;
        };
        if rename_path != path {
            return;
        }

        let te = egui::TextEdit::singleline(edit_text)
            .desired_width(60.0)
            .font(egui::FontId::new(11.0, egui::FontFamily::Proportional));
        let response = ui.add(te);

        // Auto-focus and select-all on the first frame.
        if *first_frame {
            response.request_focus();
            *first_frame = false;
        }

        // Commit only on Enter; cancel on Escape or any other focus loss.
        if response.lost_focus() {
            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter_pressed && !edit_text.is_empty() {
                *deferred_rename = Some((path.to_path_buf(), edit_text.clone()));
            } else {
                // Cancelled (Escape, clicked away, etc.) — clear rename state.
                *deferred_rename = None;
                drop(s);
                state.borrow_mut().take();
            }
        }
    });
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
                .fill(egui::Color32::from_rgba_unmultiplied(
                    0x2A, 0x2A, 0x2A, 0xE0,
                ))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_max_width(200.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon_rect, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        if payload.is_directory {
                            paint_folder_icon(ui.painter(), icon_rect);
                        } else {
                            paint_file_icon(ui.painter(), icon_rect);
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&payload.name)
                                    .font(egui::FontId::new(11.0, egui::FontFamily::Proportional))
                                    .color(egui::Color32::from_rgb(0xCC, 0xCC, 0xCC)),
                            )
                            .truncate(),
                        );
                    });
                });
        });
}

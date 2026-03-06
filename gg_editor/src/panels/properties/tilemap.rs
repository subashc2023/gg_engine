use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;
use crate::panels::{tile_uv_max, tile_uv_min};
use crate::TilemapPaintState;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_tilemap_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
    tilemap_paint: &mut TilemapPaintState,
    egui_texture_map: &std::collections::HashMap<u64, egui::TextureId>,
) -> bool {
    let mut remove = false;

    if scene.has_component::<TilemapComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Tilemap").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("tilemap", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (
                mut width,
                mut height,
                tile_size,
                mut tileset_cols,
                cell_size,
                spacing,
                margin,
                tex_handle,
                mut sorting_layer,
                mut order_in_layer,
            ) = {
                let tm = scene.get_component::<TilemapComponent>(entity).unwrap();
                (
                    tm.width,
                    tm.height,
                    tm.tile_size,
                    tm.tileset_columns,
                    tm.cell_size,
                    tm.spacing,
                    tm.margin,
                    tm.texture_handle,
                    tm.sorting_layer,
                    tm.order_in_layer,
                )
            };

            // Grid size.
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Width");
                if ui
                    .add(egui::DragValue::new(&mut width).speed(1).range(1..=1000u32))
                    .changed()
                {
                    changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Height");
                if ui
                    .add(
                        egui::DragValue::new(&mut height)
                            .speed(1)
                            .range(1..=1000u32),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
            if changed {
                if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                    tm.resize(width, height);
                    *scene_dirty = true;
                }
            }

            // Tile size.
            ui.horizontal(|ui| {
                ui.label("Tile Size");
                let mut ts = [tile_size.x, tile_size.y];
                let r1 = ui.add(egui::DragValue::new(&mut ts[0]).speed(0.01).prefix("X: "));
                let r2 = ui.add(egui::DragValue::new(&mut ts[1]).speed(0.01).prefix("Y: "));
                if r1.changed() || r2.changed() {
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.tile_size = Vec2::new(ts[0], ts[1]);
                        *scene_dirty = true;
                    }
                }
            });

            // Tileset columns.
            ui.horizontal(|ui| {
                ui.label("Tileset Columns");
                if ui
                    .add(
                        egui::DragValue::new(&mut tileset_cols)
                            .speed(1)
                            .range(1..=256u32),
                    )
                    .changed()
                {
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.tileset_columns = tileset_cols;
                        *scene_dirty = true;
                    }
                }
            });

            // Cell size.
            ui.horizontal(|ui| {
                ui.label("Cell Size");
                let mut cs = [cell_size.x, cell_size.y];
                let r1 = ui.add(egui::DragValue::new(&mut cs[0]).speed(1.0).prefix("X: "));
                let r2 = ui.add(egui::DragValue::new(&mut cs[1]).speed(1.0).prefix("Y: "));
                if r1.changed() || r2.changed() {
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.cell_size = Vec2::new(cs[0], cs[1]);
                        *scene_dirty = true;
                    }
                }
            });

            // Spacing.
            ui.horizontal(|ui| {
                ui.label("Spacing");
                let mut sp = [spacing.x, spacing.y];
                let r1 = ui.add(
                    egui::DragValue::new(&mut sp[0])
                        .speed(1.0)
                        .range(0.0..=256.0)
                        .prefix("X: "),
                );
                let r2 = ui.add(
                    egui::DragValue::new(&mut sp[1])
                        .speed(1.0)
                        .range(0.0..=256.0)
                        .prefix("Y: "),
                );
                if r1.changed() || r2.changed() {
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.spacing = Vec2::new(sp[0], sp[1]);
                        *scene_dirty = true;
                    }
                }
            });

            // Margin.
            ui.horizontal(|ui| {
                ui.label("Margin");
                let mut mg = [margin.x, margin.y];
                let r1 = ui.add(
                    egui::DragValue::new(&mut mg[0])
                        .speed(1.0)
                        .range(0.0..=256.0)
                        .prefix("X: "),
                );
                let r2 = ui.add(
                    egui::DragValue::new(&mut mg[1])
                        .speed(1.0)
                        .range(0.0..=256.0)
                        .prefix("Y: "),
                );
                if r1.changed() || r2.changed() {
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.margin = Vec2::new(mg[0], mg[1]);
                        *scene_dirty = true;
                    }
                }
            });

            // Sorting layer & order.
            let mut sort_changed = false;
            ui.horizontal(|ui| {
                ui.label("Sorting Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut sorting_layer).speed(0.1))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Order in Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut order_in_layer).speed(0.1))
                    .changed();
            });
            if sort_changed {
                if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                    tm.sorting_layer = sorting_layer;
                    tm.order_in_layer = order_in_layer;
                }
                *scene_dirty = true;
            }

            // Texture handle (drag-drop from content browser).
            ui.horizontal(|ui| {
                ui.label("Tileset");
                let handle_label = if tex_handle.raw() == 0 {
                    "None".to_string()
                } else if let Some(ref am) = asset_manager {
                    am.get_metadata(&tex_handle)
                        .map(|m| m.file_path.clone())
                        .unwrap_or_else(|| format!("{}", tex_handle))
                } else {
                    format!("{}", tex_handle)
                };
                let resp = ui.button(&handle_label);
                if let Some(payload) = resp.dnd_release_payload::<ContentBrowserPayload>() {
                    if !payload.is_directory {
                        let ext = payload
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        if matches!(ext, "png" | "jpg" | "jpeg") {
                            if let Some(ref mut am) = asset_manager {
                                let rel_path = payload
                                    .path
                                    .strip_prefix(assets_root)
                                    .unwrap_or(&payload.path)
                                    .to_string_lossy()
                                    .replace('\\', "/");
                                let new_handle = am.import_asset(&rel_path);
                                am.save_registry();
                                if let Some(mut tm) =
                                    scene.get_component_mut::<TilemapComponent>(entity)
                                {
                                    tm.texture_handle = new_handle;
                                    tm.texture = None;
                                    *scene_dirty = true;
                                }
                            }
                        }
                    }
                }
            });

            // -- Tile Palette (paint brush) --
            ui.separator();
            ui.label(egui::RichText::new("Tile Palette").strong());

            // Tools row.
            ui.horizontal(|ui| {
                let eraser_active = tilemap_paint.brush_tile_id == -1;
                if ui.selectable_label(eraser_active, "Eraser (X)").clicked() {
                    if eraser_active {
                        tilemap_paint.clear_brush();
                    } else {
                        tilemap_paint.brush_tile_id = -1;
                    }
                }
                if tilemap_paint.is_active() && ui.button("Clear Brush (Esc)").clicked() {
                    tilemap_paint.clear_brush();
                }
            });

            // Flip toggles.
            if tilemap_paint.brush_tile_id >= 0 {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut tilemap_paint.brush_flip_h, "Flip H");
                    ui.checkbox(&mut tilemap_paint.brush_flip_v, "Flip V");
                });
            }

            // Calculate how many tiles exist in the tileset.
            let tileset_col_count = tileset_cols.max(1) as usize;
            let (tex_dims, tileset_egui_tex) = {
                let tm = scene.get_component::<TilemapComponent>(entity).unwrap();
                let dims = tm.texture.as_ref().map(|t| (t.width(), t.height()));
                let egui_tex = tm
                    .texture
                    .as_ref()
                    .and_then(|t| egui_texture_map.get(&t.egui_handle()).copied());
                (dims, egui_tex)
            };
            let max_tiles = if let Some((tex_w, tex_h)) = tex_dims {
                let effective_cell_w = cell_size.x + spacing.x;
                let effective_cell_h = cell_size.y + spacing.y;
                let tileset_rows = ((tex_h as f32 - margin.y * 2.0 + spacing.y) / effective_cell_h)
                    .floor()
                    .max(1.0) as usize;
                let tileset_cols_actual = ((tex_w as f32 - margin.x * 2.0 + spacing.x)
                    / effective_cell_w)
                    .floor()
                    .max(1.0) as usize;
                (tileset_rows * tileset_cols_actual).min(1024)
            } else {
                (tileset_col_count * 4).min(64)
            };

            // Tile coordinate picker.
            if max_tiles > 0 {
                let max_row = (max_tiles - 1) / tileset_col_count;
                let mut picker_col = if tilemap_paint.brush_tile_id >= 0 {
                    tilemap_paint.brush_tile_id as usize % tileset_col_count
                } else {
                    0
                };
                let mut picker_row = if tilemap_paint.brush_tile_id >= 0 {
                    tilemap_paint.brush_tile_id as usize / tileset_col_count
                } else {
                    0
                };
                let mut picker_col_i32 = picker_col as i32;
                let mut picker_row_i32 = picker_row as i32;

                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Tile:");
                    ui.label("Col");
                    if ui
                        .add(
                            egui::DragValue::new(&mut picker_col_i32)
                                .range(0..=(tileset_col_count as i32 - 1))
                                .speed(0.1),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.label("Row");
                    if ui
                        .add(
                            egui::DragValue::new(&mut picker_row_i32)
                                .range(0..=max_row as i32)
                                .speed(0.1),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });
                if changed {
                    picker_col = (picker_col_i32 as usize).min(tileset_col_count - 1);
                    picker_row = (picker_row_i32 as usize).min(max_row);
                    let new_id = (picker_row * tileset_col_count + picker_col) as i32;
                    if (new_id as usize) < max_tiles {
                        tilemap_paint.brush_tile_id = new_id;
                    }
                }
            }

            if tilemap_paint.brush_tile_id == -1 {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x3A, 0x20, 0x20))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Eraser active").strong());
                        ui.label("Click on tilemap to clear tiles");
                    });
            }

            ui.add_space(4.0);

            // Palette grid.
            let avail_w = ui.available_width();
            let cell_side: f32 = 32.0;
            let grid_spacing = 2.0;
            let palette_cols = ((avail_w + grid_spacing) / (cell_side + grid_spacing))
                .floor()
                .max(1.0) as usize;
            let palette_rows = max_tiles.div_ceil(palette_cols);
            let cell_btn_size = egui::vec2(cell_side, cell_side);

            egui::ScrollArea::vertical()
                .max_height(250.0)
                .id_salt(("tile_palette_scroll", entity.id()))
                .show(ui, |ui| {
                    for row in 0..palette_rows {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(grid_spacing, grid_spacing);
                            for col in 0..palette_cols {
                                let tile_id = (row * palette_cols + col) as i32;
                                if tile_id >= max_tiles as i32 {
                                    break;
                                }
                                let is_selected = tilemap_paint.brush_tile_id == tile_id;

                                let (rect, resp) =
                                    ui.allocate_exact_size(cell_btn_size, egui::Sense::click());

                                if let (Some((tw, th)), Some(egui_tex)) =
                                    (tex_dims, tileset_egui_tex)
                                {
                                    let ts_col = tile_id as usize % tileset_col_count;
                                    let ts_row = tile_id as usize / tileset_col_count;
                                    let uv_min = tile_uv_min(
                                        ts_col, ts_row, cell_size, spacing, margin, tw as f32,
                                        th as f32,
                                    );
                                    let uv_max = tile_uv_max(
                                        ts_col, ts_row, cell_size, spacing, margin, tw as f32,
                                        th as f32,
                                    );
                                    let mut mesh = egui::Mesh::with_texture(egui_tex);
                                    mesh.add_rect_with_uv(
                                        rect,
                                        egui::Rect::from_min_max(
                                            egui::pos2(uv_min.0, uv_min.1),
                                            egui::pos2(uv_max.0, uv_max.1),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                    ui.painter().add(egui::Shape::mesh(mesh));
                                } else {
                                    let hue = ((tile_id as f32) * 0.618034) % 1.0;
                                    let bg: egui::Color32 =
                                        egui::ecolor::Hsva::new(hue, 0.5, 0.7, 1.0).into();
                                    ui.painter()
                                        .rect_filled(rect, egui::CornerRadius::same(2), bg);
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        format!("{}", tile_id),
                                        egui::FontId::new(10.0, egui::FontFamily::Monospace),
                                        egui::Color32::from_rgb(0xEE, 0xEE, 0xEE),
                                    );
                                }

                                if is_selected {
                                    ui.painter().rect_stroke(
                                        rect,
                                        egui::CornerRadius::same(2),
                                        egui::Stroke::new(
                                            2.0,
                                            egui::Color32::from_rgb(0x00, 0x7A, 0xCC),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }

                                if resp.clicked() {
                                    if is_selected {
                                        tilemap_paint.clear_brush();
                                    } else {
                                        tilemap_paint.brush_tile_id = tile_id;
                                    }
                                }

                                let ts_row = tile_id as usize / tileset_col_count;
                                let ts_col = tile_id as usize % tileset_col_count;
                                resp.on_hover_text(format!(
                                    "Tile {} (col {}, row {})",
                                    tile_id, ts_col, ts_row
                                ));
                            }
                        });
                    }
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

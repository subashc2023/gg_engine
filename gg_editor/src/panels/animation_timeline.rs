use gg_engine::egui;
use gg_engine::prelude::*;

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TOOLBAR_HEIGHT: f32 = 28.0;
const RULER_HEIGHT: f32 = 22.0;
const CLIP_BAR_HEIGHT: f32 = 22.0;
const CLIP_BAR_SPACING: f32 = 3.0;
const CLIP_LABEL_LEFT_PAD: f32 = 80.0;
const MIN_ZOOM: f32 = 8.0;
const MAX_ZOOM: f32 = 64.0;
const EDGE_GRAB_PX: f32 = 5.0;
const DEFAULT_GRID_CELL_DISPLAY: f32 = 48.0;
const GRID_SPACING: f32 = 2.0;
const GRID_MIN_WIDTH: f32 = 160.0;

const COLOR_PLAYHEAD: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x44, 0x44);
const COLOR_CLIP: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x6E, 0x9E);
const COLOR_CLIP_SELECTED: egui::Color32 = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);

const COLOR_FRAME_IN_CLIP: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x7A, 0xCC, 0x40);
const COLOR_GRID_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xCC, 0x00);
const COLOR_RULER_BG: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2A, 0x2A);
const COLOR_TIMELINE_BG: egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1A, 0x1A);
const COLOR_TICK: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x55);
const COLOR_TICK_MAJOR: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);

// ---------------------------------------------------------------------------
// Thread-local panel state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum TimelineDrag {
    Playhead,
    ClipStart {
        clip_index: usize,
    },
    ClipEnd {
        clip_index: usize,
    },
    ClipBody {
        clip_index: usize,
        grab_frame_offset: i32,
    },
}

thread_local! {
    static SELECTED_CLIP: Cell<Option<usize>> = const { Cell::new(None) };
    static ZOOM: Cell<f32> = const { Cell::new(20.0) };
    static SCROLL_X: Cell<f32> = const { Cell::new(0.0) };
    static GRID_CELL_SIZE: Cell<f32> = const { Cell::new(DEFAULT_GRID_CELL_DISPLAY) };
    static TRACKED_ENTITY: Cell<u64> = const { Cell::new(0) };
    static ACTIVE_DRAG: RefCell<Option<TimelineDrag>> = const { RefCell::new(None) };
    static HOVERED_FRAME: Cell<Option<u32>> = const { Cell::new(None) };
    /// Pick mode: when Some, next grid click sets start or end frame of selected clip.
    static PICK_MODE: Cell<Option<PickTarget>> = const { Cell::new(None) };
}

#[derive(Clone, Copy, PartialEq)]
enum PickTarget {
    Start,
    End,
}

pub(crate) fn reset_animation_timeline_state() {
    SELECTED_CLIP.set(None);
    SCROLL_X.set(0.0);
    ACTIVE_DRAG.with(|d| *d.borrow_mut() = None);
    TRACKED_ENTITY.set(0);
    HOVERED_FRAME.set(None);
    PICK_MODE.set(None);
}

// ---------------------------------------------------------------------------
// Main panel entry
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn animation_timeline_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    _asset_manager: &mut Option<EditorAssetManager>,
    egui_texture_map: &HashMap<u64, egui::TextureId>,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) {
    let entity = match *selection_context {
        Some(e) if scene.has_component::<SpriteAnimatorComponent>(e) => e,
        _ => {
            ui.centered_and_justified(|ui| {
                ui.label("Select an entity with a Sprite Animator component");
            });
            return;
        }
    };

    // Detect entity change → reset state.
    let uuid = scene
        .get_component::<IdComponent>(entity)
        .map(|id| id.id.raw())
        .unwrap_or(0);
    if TRACKED_ENTITY.get() != uuid {
        TRACKED_ENTITY.set(uuid);
        SELECTED_CLIP.set(None);
        SCROLL_X.set(0.0);
        ACTIVE_DRAG.with(|d| *d.borrow_mut() = None);
        HOVERED_FRAME.set(None);
        PICK_MODE.set(None);
    }

    // Toolbar.
    draw_toolbar(ui, scene, entity, scene_dirty);

    ui.separator();

    // Split: sprite sheet grid (left) | timeline (right).
    let avail = ui.available_rect_before_wrap();
    let grid_width = GRID_MIN_WIDTH
        .max(avail.width() * 0.25)
        .min(avail.width() * 0.4);

    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(grid_width, avail.height()),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                draw_sprite_sheet_grid(ui, scene, entity, egui_texture_map, scene_dirty);
            },
        );

        ui.separator();

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), avail.height()),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                draw_timeline(ui, scene, entity, scene_dirty);
            },
        );
    });
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

fn draw_toolbar(ui: &mut egui::Ui, scene: &mut Scene, entity: Entity, scene_dirty: &mut bool) {
    let (
        clip_count,
        clip_names,
        is_playing,
        is_previewing,
        current_frame,
        selected_fps,
        selected_looping,
    ) = {
        let sa = scene
            .get_component::<SpriteAnimatorComponent>(entity)
            .unwrap();
        let sel = SELECTED_CLIP.get();
        let fps = sel
            .and_then(|i| sa.clips.get(i))
            .map(|c| c.fps)
            .unwrap_or(12.0);
        let looping = sel
            .and_then(|i| sa.clips.get(i))
            .map(|c| c.looping)
            .unwrap_or(true);
        (
            sa.clips.len(),
            sa.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
            sa.is_playing(),
            sa.is_previewing(),
            sa.current_frame(),
            fps,
            looping,
        )
    };

    ui.horizontal(|ui| {
        ui.set_height(TOOLBAR_HEIGHT);

        // Play / Pause / Stop.
        if is_playing && is_previewing {
            if ui.button("Pause").clicked() {
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    sa.stop();
                }
            }
        } else if ui.button("Play").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                sa.set_previewing(true);
                let sel = SELECTED_CLIP.get();
                if let Some(idx) = sel {
                    if let Some(clip) = sa.clips.get(idx) {
                        let name = clip.name.clone();
                        sa.play(&name);
                    }
                } else if !sa.default_clip.is_empty() {
                    let name = sa.default_clip.clone();
                    sa.play(&name);
                } else if !sa.clips.is_empty() {
                    let name = sa.clips[0].name.clone();
                    sa.play(&name);
                    SELECTED_CLIP.set(Some(0));
                }
            }
        }

        if ui.button("Stop").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                sa.reset();
            }
        }

        // Step buttons.
        if ui.button("|<").on_hover_text("Previous frame").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                if let Some(idx) = sa.current_clip_index() {
                    let start = sa.clips[idx].start_frame;
                    let cur = sa.current_frame();
                    if cur > start {
                        sa.set_current_frame(cur - 1);
                    }
                }
            }
        }
        if ui.button(">|").on_hover_text("Next frame").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                if let Some(idx) = sa.current_clip_index() {
                    let end = sa.clips[idx].end_frame;
                    let cur = sa.current_frame();
                    if cur < end {
                        sa.set_current_frame(cur + 1);
                    }
                }
            }
        }

        ui.separator();

        // Clip selector.
        let mut sel_idx = SELECTED_CLIP.get().unwrap_or(0);
        let prev = sel_idx;
        egui::ComboBox::from_id_salt(("anim_tl_clip_sel", entity.id()))
            .width(100.0)
            .selected_text(
                clip_names
                    .get(sel_idx)
                    .cloned()
                    .unwrap_or_else(|| String::from("(none)")),
            )
            .show_ui(ui, |ui| {
                for (i, name) in clip_names.iter().enumerate() {
                    ui.selectable_value(&mut sel_idx, i, name);
                }
            });
        if sel_idx != prev || SELECTED_CLIP.get().is_none() {
            SELECTED_CLIP.set(Some(sel_idx));
        }

        // Add / remove clip.
        if ui.button("+").on_hover_text("Add clip").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                let idx = sa.clips.len();
                sa.clips.push(AnimationClip {
                    name: format!("clip_{}", idx),
                    ..Default::default()
                });
                SELECTED_CLIP.set(Some(idx));
                *scene_dirty = true;
            }
        }
        if ui
            .button("-")
            .on_hover_text("Remove selected clip")
            .clicked()
        {
            if let Some(sel) = SELECTED_CLIP.get() {
                if sel < clip_count {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.clips.remove(sel);
                        *scene_dirty = true;
                    }
                    if sel > 0 {
                        SELECTED_CLIP.set(Some(sel - 1));
                    } else {
                        SELECTED_CLIP.set(None);
                    }
                }
            }
        }

        ui.separator();

        // FPS / Loop for selected clip.
        if let Some(sel) = SELECTED_CLIP.get() {
            let mut fps = selected_fps;
            let mut looping = selected_looping;
            ui.label("FPS:");
            if ui
                .add(egui::DragValue::new(&mut fps).range(0.1..=120.0).speed(0.1))
                .changed()
            {
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    if let Some(c) = sa.clips.get_mut(sel) {
                        c.fps = fps;
                        *scene_dirty = true;
                    }
                }
            }
            if ui.checkbox(&mut looping, "Loop").changed() {
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    if let Some(c) = sa.clips.get_mut(sel) {
                        c.looping = looping;
                        *scene_dirty = true;
                    }
                }
            }
        }

        ui.separator();

        // Frame display.
        ui.label(format!("Frame: {}", current_frame));

        // Zoom slider.
        ui.separator();
        let mut zoom = ZOOM.get();
        ui.label("Zoom:");
        if ui
            .add(egui::Slider::new(&mut zoom, MIN_ZOOM..=MAX_ZOOM).show_value(false))
            .changed()
        {
            ZOOM.set(zoom);
        }
    });
}

// ---------------------------------------------------------------------------
// Sprite Sheet Grid
// ---------------------------------------------------------------------------

fn draw_sprite_sheet_grid(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    egui_texture_map: &HashMap<u64, egui::TextureId>,
    scene_dirty: &mut bool,
) {
    let (columns, cell_size, tex_info, clip_range) = {
        let sa = scene
            .get_component::<SpriteAnimatorComponent>(entity)
            .unwrap();
        let sel = SELECTED_CLIP.get();
        let range = sel
            .and_then(|i| sa.clips.get(i))
            .map(|c| (c.start_frame, c.end_frame));

        // Try per-clip texture first, then sprite texture.
        let clip_tex = sel
            .and_then(|i| sa.clips.get(i))
            .and_then(|c| c.texture.as_ref());

        let sprite_tex = scene
            .get_component::<SpriteRendererComponent>(entity)
            .and_then(|s| s.texture.clone());

        let tex = clip_tex.cloned().or(sprite_tex);
        let tex_info = tex.as_ref().map(|t| {
            let egui_tex = egui_texture_map.get(&t.egui_handle()).copied();
            (t.width() as f32, t.height() as f32, egui_tex)
        });

        (sa.columns.max(1), sa.cell_size, tex_info, range)
    };

    let cell_display = GRID_CELL_SIZE.get();

    // Compute total grid rows from texture.
    let total_frames = if let Some((tw, th, _)) = &tex_info {
        if cell_size.x > 0.0 && cell_size.y > 0.0 {
            let cols_in_tex = (*tw / cell_size.x).floor() as u32;
            let rows_in_tex = (*th / cell_size.y).floor() as u32;
            rows_in_tex * cols_in_tex
        } else {
            columns * 4
        }
    } else {
        columns * 4
    };

    let grid_rows = (total_frames as f32 / columns as f32).ceil() as u32;

    // Pick mode indicator.
    let pick = PICK_MODE.get();
    if let Some(target) = pick {
        let label = match target {
            PickTarget::Start => "Click a cell to set START frame",
            PickTarget::End => "Click a cell to set END frame",
        };
        ui.colored_label(egui::Color32::YELLOW, label);
        if ui.button("Cancel").clicked() {
            PICK_MODE.set(None);
        }
    }

    // Grid zoom with scroll wheel.
    let grid_area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(grid_area) {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_delta != 0.0 && ui.input(|i| i.modifiers.ctrl) {
            let new_size = (GRID_CELL_SIZE.get() + scroll_delta * 0.5).clamp(16.0, 128.0);
            GRID_CELL_SIZE.set(new_size);
        }
    }

    egui::ScrollArea::both()
        .max_height(ui.available_height())
        .id_salt(("anim_grid_scroll", entity.id()))
        .show(ui, |ui| {
            for row in 0..grid_rows {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(GRID_SPACING, GRID_SPACING);
                    for col in 0..columns {
                        let frame = row * columns + col;
                        if frame >= total_frames {
                            break;
                        }

                        let cell_btn_size = egui::vec2(cell_display, cell_display);
                        let (rect, resp) =
                            ui.allocate_exact_size(cell_btn_size, egui::Sense::click());

                        // Draw cell contents.
                        if let Some((tw, th, Some(egui_tex))) = &tex_info {
                            let uv_col = frame % columns;
                            let uv_row = frame / columns;
                            let uv_min_x = uv_col as f32 * cell_size.x / tw;
                            let uv_min_y = uv_row as f32 * cell_size.y / th;
                            let uv_max_x = (uv_col as f32 + 1.0) * cell_size.x / tw;
                            let uv_max_y = (uv_row as f32 + 1.0) * cell_size.y / th;

                            let mut mesh = egui::Mesh::with_texture(*egui_tex);
                            mesh.add_rect_with_uv(
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(uv_min_x, uv_min_y),
                                    egui::pos2(uv_max_x, uv_max_y),
                                ),
                                egui::Color32::WHITE,
                            );
                            ui.painter().add(egui::Shape::mesh(mesh));
                        } else {
                            // Fallback: colored cell with frame number.
                            let hue = (frame as f32 * 0.618034) % 1.0;
                            let bg: egui::Color32 =
                                egui::ecolor::Hsva::new(hue, 0.3, 0.4, 1.0).into();
                            ui.painter()
                                .rect_filled(rect, egui::CornerRadius::same(1), bg);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("{}", frame),
                                egui::FontId::new(9.0, egui::FontFamily::Monospace),
                                egui::Color32::from_rgb(0xCC, 0xCC, 0xCC),
                            );
                        }

                        // Highlight frames within selected clip range.
                        if let Some((start, end)) = clip_range {
                            if frame >= start && frame <= end {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::ZERO,
                                    COLOR_FRAME_IN_CLIP,
                                );
                            }
                        }

                        // Current frame highlight.
                        let current_frame = scene
                            .get_component::<SpriteAnimatorComponent>(entity)
                            .map(|sa| sa.current_frame())
                            .unwrap_or(u32::MAX);
                        if frame == current_frame {
                            ui.painter().rect_stroke(
                                rect,
                                egui::CornerRadius::same(1),
                                egui::Stroke::new(2.0, COLOR_GRID_HIGHLIGHT),
                                egui::StrokeKind::Inside,
                            );
                        }

                        // Hover highlight (coordinated with timeline).
                        if resp.hovered() {
                            HOVERED_FRAME.set(Some(frame));
                            ui.painter().rect_stroke(
                                rect,
                                egui::CornerRadius::same(1),
                                egui::Stroke::new(1.0, egui::Color32::WHITE),
                                egui::StrokeKind::Inside,
                            );
                        }

                        // Click handling.
                        if resp.clicked() {
                            if let Some(target) = PICK_MODE.get() {
                                if let Some(sel) = SELECTED_CLIP.get() {
                                    if let Some(mut sa) =
                                        scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                                    {
                                        if let Some(c) = sa.clips.get_mut(sel) {
                                            match target {
                                                PickTarget::Start => {
                                                    c.start_frame = frame;
                                                    if c.end_frame < frame {
                                                        c.end_frame = frame;
                                                    }
                                                }
                                                PickTarget::End => {
                                                    c.end_frame = frame;
                                                    if c.start_frame > frame {
                                                        c.start_frame = frame;
                                                    }
                                                }
                                            }
                                            *scene_dirty = true;
                                        }
                                    }
                                }
                                PICK_MODE.set(None);
                            } else {
                                // Set playhead to this frame.
                                if let Some(mut sa) =
                                    scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                                {
                                    sa.set_current_frame(frame);
                                    // If no clip selected, try to find one containing this frame.
                                    if SELECTED_CLIP.get().is_none() {
                                        if let Some(idx) = sa.clips.iter().position(|c| {
                                            frame >= c.start_frame && frame <= c.end_frame
                                        }) {
                                            SELECTED_CLIP.set(Some(idx));
                                            sa.set_current_clip_index(Some(idx));
                                        }
                                    }
                                }
                            }
                        }

                        resp.on_hover_text(format!("Frame {} (col {}, row {})", frame, col, row));
                    }
                });
            }
        });
}

// ---------------------------------------------------------------------------
// Timeline / Dopesheet
// ---------------------------------------------------------------------------

fn draw_timeline(ui: &mut egui::Ui, scene: &mut Scene, entity: Entity, scene_dirty: &mut bool) {
    let zoom = ZOOM.get();
    let scroll_x = SCROLL_X.get();

    let (clip_count, clips_data, current_frame, max_frame): (
        usize,
        Vec<(String, u32, u32)>,
        u32,
        u32,
    ) = {
        let sa = scene
            .get_component::<SpriteAnimatorComponent>(entity)
            .unwrap();
        let clips: Vec<(String, u32, u32)> = sa
            .clips
            .iter()
            .map(|c| (c.name.clone(), c.start_frame, c.end_frame))
            .collect();
        let max = clips.iter().map(|(_, _, e)| *e).max().unwrap_or(0) + 10;
        (sa.clips.len(), clips, sa.current_frame(), max)
    };

    let avail = ui.available_rect_before_wrap();
    let timeline_width = avail.width();

    // Handle horizontal scroll with mouse wheel.
    if ui.rect_contains_pointer(avail) {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_delta != 0.0 && !ui.input(|i| i.modifiers.ctrl) {
            let new_scroll = (scroll_x - scroll_delta * 0.5).max(0.0);
            SCROLL_X.set(new_scroll);
        }
        // Ctrl+scroll = zoom.
        if scroll_delta != 0.0 && ui.input(|i| i.modifiers.ctrl) {
            let new_zoom = (zoom + scroll_delta * 0.3).clamp(MIN_ZOOM, MAX_ZOOM);
            ZOOM.set(new_zoom);
        }
    }

    // Allocate the full timeline rect.
    let total_height =
        RULER_HEIGHT + (clip_count as f32) * (CLIP_BAR_HEIGHT + CLIP_BAR_SPACING) + 40.0;
    let timeline_rect = egui::Rect::from_min_size(
        avail.min,
        egui::vec2(timeline_width, total_height.max(avail.height())),
    );

    // Background.
    let painter = ui.painter_at(timeline_rect);
    painter.rect_filled(timeline_rect, egui::CornerRadius::ZERO, COLOR_TIMELINE_BG);

    let origin_x = timeline_rect.left() + CLIP_LABEL_LEFT_PAD - scroll_x;

    // Label column background.
    let label_col_rect = egui::Rect::from_min_size(
        timeline_rect.min,
        egui::vec2(CLIP_LABEL_LEFT_PAD, timeline_rect.height()),
    );
    painter.rect_filled(label_col_rect, egui::CornerRadius::ZERO, COLOR_RULER_BG);

    // Frame ruler.
    let ruler_rect =
        egui::Rect::from_min_size(timeline_rect.min, egui::vec2(timeline_width, RULER_HEIGHT));
    painter.rect_filled(ruler_rect, egui::CornerRadius::ZERO, COLOR_RULER_BG);

    // Determine tick interval based on zoom.
    let tick_interval = if zoom >= 40.0 {
        1
    } else if zoom >= 20.0 {
        2
    } else if zoom >= 12.0 {
        5
    } else {
        10
    };
    let major_interval = tick_interval * 5;

    let first_frame = (scroll_x / zoom).floor() as i32;
    let last_frame = ((scroll_x + timeline_width - CLIP_LABEL_LEFT_PAD) / zoom).ceil() as i32;

    for f in first_frame..=last_frame.min(max_frame as i32 + 20) {
        if f < 0 {
            continue;
        }
        let x = origin_x + f as f32 * zoom;
        if x < timeline_rect.left() + CLIP_LABEL_LEFT_PAD || x > timeline_rect.right() {
            continue;
        }

        let is_major = f % major_interval == 0;
        let is_tick = f % tick_interval == 0;

        if is_major {
            painter.line_segment(
                [
                    egui::pos2(x, ruler_rect.top() + 2.0),
                    egui::pos2(x, ruler_rect.bottom()),
                ],
                egui::Stroke::new(1.0, COLOR_TICK_MAJOR),
            );
            painter.text(
                egui::pos2(x + 2.0, ruler_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{}", f),
                egui::FontId::new(10.0, egui::FontFamily::Monospace),
                egui::Color32::from_rgb(0xCC, 0xCC, 0xCC),
            );
        } else if is_tick {
            painter.line_segment(
                [
                    egui::pos2(x, ruler_rect.bottom() - 6.0),
                    egui::pos2(x, ruler_rect.bottom()),
                ],
                egui::Stroke::new(1.0, COLOR_TICK),
            );
        }
    }

    // Clip bars area.
    let clips_top = ruler_rect.bottom() + 4.0;
    let selected_clip = SELECTED_CLIP.get();

    for (i, (name, start, end)) in clips_data.iter().enumerate() {
        let y = clips_top + i as f32 * (CLIP_BAR_HEIGHT + CLIP_BAR_SPACING);
        let x_start = origin_x + *start as f32 * zoom;
        let x_end = origin_x + (*end as f32 + 1.0) * zoom;

        // Clip label (left side, fixed position).
        let label_rect = egui::Rect::from_min_size(
            egui::pos2(timeline_rect.left() + 2.0, y),
            egui::vec2(CLIP_LABEL_LEFT_PAD - 4.0, CLIP_BAR_HEIGHT),
        );
        painter.text(
            egui::pos2(label_rect.left() + 2.0, label_rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
            if selected_clip == Some(i) {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(0xBB, 0xBB, 0xBB)
            },
        );

        // Clip bar.
        let bar_x_start = x_start.max(timeline_rect.left() + CLIP_LABEL_LEFT_PAD);
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(bar_x_start, y),
            egui::pos2(x_end.min(timeline_rect.right()), y + CLIP_BAR_HEIGHT),
        );

        if bar_rect.width() > 0.0 {
            let fill = if selected_clip == Some(i) {
                COLOR_CLIP_SELECTED
            } else {
                COLOR_CLIP
            };
            painter.rect_filled(bar_rect, egui::CornerRadius::same(3), fill);

            // Frame ticks inside bar.
            if zoom >= 12.0 {
                for f in *start..=*end {
                    let fx = origin_x + f as f32 * zoom + zoom * 0.5;
                    if fx > bar_rect.left() && fx < bar_rect.right() {
                        painter.line_segment(
                            [
                                egui::pos2(fx, y + CLIP_BAR_HEIGHT - 4.0),
                                egui::pos2(fx, y + CLIP_BAR_HEIGHT),
                            ],
                            egui::Stroke::new(
                                1.0,
                                egui::Color32::from_rgba_premultiplied(255, 255, 255, 60),
                            ),
                        );
                    }
                }
            }

            // Frame range text inside bar.
            if bar_rect.width() > 50.0 {
                painter.text(
                    bar_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}-{}", start, end),
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 180),
                );
            }

            // Edge handles visual (thicker on hover).
            let start_handle = egui::Rect::from_min_max(
                egui::pos2(x_start - EDGE_GRAB_PX, y),
                egui::pos2(x_start + EDGE_GRAB_PX, y + CLIP_BAR_HEIGHT),
            );
            let end_handle = egui::Rect::from_min_max(
                egui::pos2(x_end - EDGE_GRAB_PX, y),
                egui::pos2(x_end + EDGE_GRAB_PX, y + CLIP_BAR_HEIGHT),
            );

            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if start_handle.contains(pos) || end_handle.contains(pos) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
    }

    // Playhead.
    let playhead_x = origin_x + current_frame as f32 * zoom + zoom * 0.5;
    if playhead_x >= timeline_rect.left() && playhead_x <= timeline_rect.right() {
        painter.line_segment(
            [
                egui::pos2(playhead_x, ruler_rect.top()),
                egui::pos2(playhead_x, timeline_rect.bottom()),
            ],
            egui::Stroke::new(2.0, COLOR_PLAYHEAD),
        );
        // Playhead triangle.
        let tri = vec![
            egui::pos2(playhead_x - 5.0, ruler_rect.top()),
            egui::pos2(playhead_x + 5.0, ruler_rect.top()),
            egui::pos2(playhead_x, ruler_rect.top() + 8.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            tri,
            COLOR_PLAYHEAD,
            egui::Stroke::NONE,
        ));
    }

    // Hovered frame indicator (from grid).
    if let Some(hf) = HOVERED_FRAME.get() {
        let hx = origin_x + hf as f32 * zoom + zoom * 0.5;
        if hx >= timeline_rect.left() && hx <= timeline_rect.right() {
            painter.line_segment(
                [
                    egui::pos2(hx, ruler_rect.bottom()),
                    egui::pos2(hx, timeline_rect.bottom()),
                ],
                egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_premultiplied(255, 204, 0, 100),
                ),
            );
        }
    }

    // Interactive overlay for the entire timeline.
    let interact_rect = timeline_rect;
    let resp = ui.interact(
        interact_rect,
        ui.id().with("timeline_interact"),
        egui::Sense::click_and_drag(),
    );

    // Click to select clip / set playhead.
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            // Check if clicked on a clip bar.
            let mut clicked_clip = false;
            for (i, (_name, start, end)) in clips_data.iter().enumerate() {
                let y = clips_top + i as f32 * (CLIP_BAR_HEIGHT + CLIP_BAR_SPACING);
                let x_start = origin_x + *start as f32 * zoom;
                let x_end = origin_x + (*end as f32 + 1.0) * zoom;
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(x_start, y),
                    egui::pos2(x_end, y + CLIP_BAR_HEIGHT),
                );
                if bar_rect.contains(pos) {
                    SELECTED_CLIP.set(Some(i));
                    clicked_clip = true;
                    break;
                }
            }

            // Click on ruler = set playhead.
            if !clicked_clip && pos.y <= ruler_rect.bottom() {
                let frame = ((pos.x - origin_x) / zoom).round().max(0.0) as u32;
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    sa.set_current_frame(frame);
                }
            }
        }
    }

    // Drag handling.
    if resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let mut drag = None;

            // Check clip edges first (highest priority).
            for (i, (_name, start, end)) in clips_data.iter().enumerate() {
                let y = clips_top + i as f32 * (CLIP_BAR_HEIGHT + CLIP_BAR_SPACING);
                let x_start = origin_x + *start as f32 * zoom;
                let x_end = origin_x + (*end as f32 + 1.0) * zoom;

                let start_handle = egui::Rect::from_min_max(
                    egui::pos2(x_start - EDGE_GRAB_PX, y),
                    egui::pos2(x_start + EDGE_GRAB_PX, y + CLIP_BAR_HEIGHT),
                );
                let end_handle = egui::Rect::from_min_max(
                    egui::pos2(x_end - EDGE_GRAB_PX, y),
                    egui::pos2(x_end + EDGE_GRAB_PX, y + CLIP_BAR_HEIGHT),
                );

                if start_handle.contains(pos) {
                    drag = Some(TimelineDrag::ClipStart { clip_index: i });
                    SELECTED_CLIP.set(Some(i));
                    break;
                }
                if end_handle.contains(pos) {
                    drag = Some(TimelineDrag::ClipEnd { clip_index: i });
                    SELECTED_CLIP.set(Some(i));
                    break;
                }

                // Clip body drag.
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(x_start, y),
                    egui::pos2(x_end, y + CLIP_BAR_HEIGHT),
                );
                if bar_rect.contains(pos) {
                    let grab_frame = ((pos.x - origin_x) / zoom).round() as i32;
                    drag = Some(TimelineDrag::ClipBody {
                        clip_index: i,
                        grab_frame_offset: grab_frame - *start as i32,
                    });
                    SELECTED_CLIP.set(Some(i));
                    break;
                }
            }

            // Ruler drag = playhead scrub.
            if drag.is_none() && pos.y <= ruler_rect.bottom() + 4.0 {
                drag = Some(TimelineDrag::Playhead);
            }

            ACTIVE_DRAG.with(|d| *d.borrow_mut() = drag);
        }
    }

    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let drag = ACTIVE_DRAG.with(|d| *d.borrow());
            match drag {
                Some(TimelineDrag::Playhead) => {
                    let frame = ((pos.x - origin_x) / zoom).round().max(0.0) as u32;
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.set_current_frame(frame);
                    }
                }
                Some(TimelineDrag::ClipStart { clip_index }) => {
                    let frame = ((pos.x - origin_x) / zoom).round().max(0.0) as u32;
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        if let Some(c) = sa.clips.get_mut(clip_index) {
                            c.start_frame = frame.min(c.end_frame);
                            *scene_dirty = true;
                        }
                    }
                }
                Some(TimelineDrag::ClipEnd { clip_index }) => {
                    let frame = ((pos.x - origin_x) / zoom).round().max(0.0) as u32;
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        if let Some(c) = sa.clips.get_mut(clip_index) {
                            c.end_frame = frame.max(c.start_frame);
                            *scene_dirty = true;
                        }
                    }
                }
                Some(TimelineDrag::ClipBody {
                    clip_index,
                    grab_frame_offset,
                }) => {
                    let mouse_frame = ((pos.x - origin_x) / zoom).round().max(0.0) as i32;
                    let new_start = (mouse_frame - grab_frame_offset).max(0) as u32;
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        if let Some(c) = sa.clips.get_mut(clip_index) {
                            let len = c.end_frame - c.start_frame;
                            c.start_frame = new_start;
                            c.end_frame = new_start + len;
                            *scene_dirty = true;
                        }
                    }
                }
                None => {}
            }
        }
    }

    if resp.drag_stopped() {
        ACTIVE_DRAG.with(|d| *d.borrow_mut() = None);
    }

    // Right-click context menu.
    resp.context_menu(|ui| {
        if let Some(sel) = SELECTED_CLIP.get() {
            if sel < clip_count {
                // Rename inline.
                let mut name = clips_data.get(sel).map(|c| c.0.clone()).unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    if ui.text_edit_singleline(&mut name).changed() {
                        if let Some(mut sa) =
                            scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                        {
                            if let Some(c) = sa.clips.get_mut(sel) {
                                c.name = name;
                                *scene_dirty = true;
                            }
                        }
                    }
                });
                ui.separator();

                if ui.button("Pick Start Frame (from grid)").clicked() {
                    PICK_MODE.set(Some(PickTarget::Start));
                    ui.close();
                }
                if ui.button("Pick End Frame (from grid)").clicked() {
                    PICK_MODE.set(Some(PickTarget::End));
                    ui.close();
                }
                ui.separator();

                if ui.button("Set as Default Clip").clicked() {
                    let clip_name = clips_data[sel].0.clone();
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.default_clip = clip_name;
                        *scene_dirty = true;
                    }
                    ui.close();
                }

                if ui.button("Duplicate Clip").clicked() {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        if let Some(clip) = sa.clips.get(sel).cloned() {
                            let mut new_clip = clip;
                            new_clip.name = format!("{}_copy", new_clip.name);
                            let new_idx = sa.clips.len();
                            sa.clips.push(new_clip);
                            SELECTED_CLIP.set(Some(new_idx));
                            *scene_dirty = true;
                        }
                    }
                    ui.close();
                }

                ui.separator();
                if ui.button("Delete Clip").clicked() {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.clips.remove(sel);
                        *scene_dirty = true;
                    }
                    if sel > 0 {
                        SELECTED_CLIP.set(Some(sel - 1));
                    } else {
                        SELECTED_CLIP.set(None);
                    }
                    ui.close();
                }
            }
        } else if ui.button("Add Clip").clicked() {
            if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                let idx = sa.clips.len();
                sa.clips.push(AnimationClip {
                    name: format!("clip_{}", idx),
                    ..Default::default()
                });
                SELECTED_CLIP.set(Some(idx));
                *scene_dirty = true;
            }
            ui.close();
        }
    });

    // Consume the allocated space.
    ui.allocate_exact_size(
        egui::vec2(timeline_width, total_height.max(avail.height())),
        egui::Sense::hover(),
    );

    // Clear hovered frame at end of frame.
    // (Will be re-set next frame if still hovering.)
    HOVERED_FRAME.set(None);
}

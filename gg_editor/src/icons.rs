use gg_engine::egui;

pub fn paint_play_triangle(painter: &egui::Painter, center: egui::Pos2, half: f32) {
    let points = vec![
        egui::pos2(center.x - half * 0.7, center.y - half),
        egui::pos2(center.x + half, center.y),
        egui::pos2(center.x - half * 0.7, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::from_rgb(0x4E, 0xC9, 0x4E),
        egui::Stroke::NONE,
    ));
}

pub fn paint_stop_square(painter: &egui::Painter, center: egui::Pos2, half: f32) {
    let stop_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(
        stop_rect,
        egui::CornerRadius::same(2),
        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
    );
}

pub fn paint_pause_icon(painter: &egui::Painter, center: egui::Pos2, bar_h: f32) {
    let bar_w = 3.0;
    let gap = 2.5;
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x - gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x + gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
}

pub fn paint_step_icon(painter: &egui::Painter, center: egui::Pos2, half: f32) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let offset_x = -2.0;
    let points = vec![
        egui::pos2(center.x + offset_x - half * 0.6, center.y - half),
        egui::pos2(center.x + offset_x + half * 0.7, center.y),
        egui::pos2(center.x + offset_x - half * 0.6, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    let bar_x = center.x + half * 0.7;
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(bar_x, center.y), egui::vec2(2.5, half * 2.0)),
        0.0,
        color,
    );
}

pub fn paint_gear_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    hole_bg: egui::Color32,
) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let teeth = 6;
    let inner_r = radius * 0.55;
    let outer_r = radius;
    let tooth_width = std::f32::consts::PI / (teeth as f32 * 2.0);

    let mut points = Vec::new();
    for i in 0..teeth {
        let angle = (i as f32 / teeth as f32) * std::f32::consts::TAU;
        let a1 = angle - tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a1.cos(),
            center.y + inner_r * a1.sin(),
        ));
        let a2 = angle - tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a2.cos(),
            center.y + outer_r * a2.sin(),
        ));
        let a3 = angle + tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a3.cos(),
            center.y + outer_r * a3.sin(),
        ));
        let a4 = angle + tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a4.cos(),
            center.y + inner_r * a4.sin(),
        ));
    }

    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    painter.circle_filled(center, radius * 0.25, hole_bg);
}

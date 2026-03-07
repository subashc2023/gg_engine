use gg_engine::egui;

/// Simplified viewport panel that shows the game camera's framebuffer.
/// No gizmos, no entity picking, no tilemap painting — purely a preview.
pub(crate) fn game_viewport_ui(
    ui: &mut egui::Ui,
    viewport_size: &mut (u32, u32),
    viewport_hovered: &mut bool,
    fb_tex_id: Option<egui::TextureId>,
) {
    let available = ui.available_size();
    if available.x > 0.0 && available.y > 0.0 {
        let ppp = ui.ctx().pixels_per_point();
        *viewport_size = (
            (available.x * ppp) as u32,
            (available.y * ppp) as u32,
        );
    }
    *viewport_hovered = ui.ui_contains_pointer();

    if let Some(tex_id) = fb_tex_id {
        let size = egui::vec2(available.x, available.y);
        ui.image(egui::load::SizedTexture::new(tex_id, size));
    } else {
        ui.centered_and_justified(|ui| {
            ui.label("No camera available");
        });
    }
}

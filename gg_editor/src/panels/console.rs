use gg_engine::egui;

pub(crate) fn console_ui(ui: &mut egui::Ui) {
    // Filter buttons.
    thread_local! {
        static SHOW_INFO: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        static SHOW_WARN: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        static SHOW_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
        static SHOW_DEBUG: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static SHOW_TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        static AUTO_SCROLL: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    }

    let mut show_info = SHOW_INFO.get();
    let mut show_warn = SHOW_WARN.get();
    let mut show_error = SHOW_ERROR.get();
    let mut show_debug = SHOW_DEBUG.get();
    let mut show_trace = SHOW_TRACE.get();
    let mut auto_scroll = AUTO_SCROLL.get();

    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            gg_engine::clear_log_buffer();
        }
        ui.separator();
        ui.checkbox(&mut show_error, "Error");
        ui.checkbox(&mut show_warn, "Warn");
        ui.checkbox(&mut show_info, "Info");
        ui.checkbox(&mut show_debug, "Debug");
        ui.checkbox(&mut show_trace, "Trace");
        ui.separator();
        ui.checkbox(&mut auto_scroll, "Auto-scroll");
    });

    SHOW_INFO.set(show_info);
    SHOW_WARN.set(show_warn);
    SHOW_ERROR.set(show_error);
    SHOW_DEBUG.set(show_debug);
    SHOW_TRACE.set(show_trace);
    AUTO_SCROLL.set(auto_scroll);

    ui.separator();

    // Log entries.
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);

    gg_engine::with_log_buffer(|entries| {
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| match e.level {
                gg_engine::log::Level::Error => show_error,
                gg_engine::log::Level::Warn => show_warn,
                gg_engine::log::Level::Info => show_info,
                gg_engine::log::Level::Debug => show_debug,
                gg_engine::log::Level::Trace => show_trace,
            })
            .collect();

        let total_rows = filtered.len();
        let scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(auto_scroll);

        scroll.show_rows(ui, row_height, total_rows, |ui, row_range| {
            for &entry in &filtered[row_range] {
                let color = match entry.level {
                    gg_engine::log::Level::Error => egui::Color32::from_rgb(0xE0, 0x40, 0x40),
                    gg_engine::log::Level::Warn => egui::Color32::from_rgb(0xE0, 0xC0, 0x40),
                    gg_engine::log::Level::Info => egui::Color32::from_rgb(0xA0, 0xD0, 0xA0),
                    gg_engine::log::Level::Debug => egui::Color32::from_rgb(0x80, 0xB0, 0xE0),
                    gg_engine::log::Level::Trace => egui::Color32::from_rgb(0x80, 0x80, 0x80),
                };
                let text = format!(
                    "[{} {} {}] {}",
                    entry.timestamp, entry.level, entry.tag, entry.message
                );
                ui.label(egui::RichText::new(text).monospace().color(color));
            }
        });
    });
}

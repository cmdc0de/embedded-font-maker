mod app;
mod font;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Embedded Font Maker")
            .with_inner_size([1100.0, 750.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Embedded Font Maker",
        options,
        Box::new(|cc| Ok(Box::new(app::FontMakerApp::new(cc)))),
    )
}

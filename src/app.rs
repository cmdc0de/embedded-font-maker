//! egui/eframe application for the embedded font maker.

use std::path::PathBuf;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::font::Font;

// ─── Settings state (used in the New Font dialog) ─────────────────────────────

/// Editable settings used to configure a new font before it is created.
struct NewFontSettings {
    width: i32,
    height: i32,
    glyphs_per_row: i32,
    first_glyph_str: String,
    total_glyphs: i32,
    column_major: bool,
}

impl Default for NewFontSettings {
    fn default() -> Self {
        Self {
            width: 7,
            height: 10,
            glyphs_per_row: 16,
            first_glyph_str: " ".to_string(),
            total_glyphs: 95,
            column_major: false,
        }
    }
}

impl NewFontSettings {
    fn from_font(font: &Font) -> Self {
        Self {
            width: font.width as i32,
            height: font.height as i32,
            glyphs_per_row: font.glyphs_per_row as i32,
            first_glyph_str: (font.first_glyph as char).to_string(),
            total_glyphs: font.total_glyphs as i32,
            column_major: font.column_major,
        }
    }

    fn first_glyph_byte(&self) -> u8 {
        self.first_glyph_str
            .chars()
            .next()
            .filter(|c| c.is_ascii())
            .map(|c| c as u8)
            .unwrap_or(b'a')
    }

    fn rows_preview(&self) -> i32 {
        if self.glyphs_per_row <= 0 {
            return 0;
        }
        (self.total_glyphs + self.glyphs_per_row - 1) / self.glyphs_per_row
    }

    fn build_font(&self) -> Font {
        Font::new(
            (self.width as u8).max(1),
            (self.height as u8).max(1),
            (self.glyphs_per_row as u8).max(1),
            self.first_glyph_byte(),
            (self.total_glyphs as u16).max(1),
            self.column_major,
        )
    }
}

// ─── Save summary (shown after a successful Save / Save As) ───────────────────

/// Snapshot of the file-size details captured at the moment of a successful
/// save.  Shown inline in the left panel (under the file name) until the next
/// save, open, or new font.  Geometry details aren't stored here because the
/// live Font Info section above already shows them.
struct SaveSummary {
    file_size: usize,
    data_size: usize,
    pixels_per_glyph: usize,
    total_cells: usize,
    filled_cells: usize,
    version: u8,
}

impl SaveSummary {
    fn from_font(font: &Font) -> Self {
        Self {
            file_size: font.file_size(),
            data_size: font.data_size(),
            pixels_per_glyph: font.pixels_per_glyph(),
            total_cells: font.total_pixels(),
            filled_cells: font.filled_pixels(),
            version: font.version,
        }
    }

    /// Cells that are not filled (`total_cells - filled_cells`).
    fn empty_cells(&self) -> usize {
        self.total_cells.saturating_sub(self.filled_cells)
    }
}

// ─── Application state ────────────────────────────────────────────────────────

pub struct FontMakerApp {
    font: Font,
    current_glyph: usize,
    current_path: Option<PathBuf>,
    status: String,

    show_new_dialog: bool,
    new_settings: NewFontSettings,

    /// Details of the last successful save, shown inline in the left panel
    /// while this is `Some`.  Cleared when a different font is opened or created.
    save_summary: Option<SaveSummary>,

    /// When `true`, the ASCII reference table is shown on the right side.
    show_ascii_table: bool,
}

impl FontMakerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let font = Font::default();
        let settings = NewFontSettings::from_font(&font);
        Self {
            font,
            current_glyph: 0,
            current_path: None,
            status: "Ready – create a new font or open an existing one.".to_string(),
            show_new_dialog: false,
            new_settings: settings,
            save_summary: None,
            show_ascii_table: false,
        }
    }

    // ── File helpers ──────────────────────────────────────────────────────────

    fn load_font(&mut self, path: PathBuf) {
        match std::fs::File::open(&path) {
            Ok(mut f) => match Font::load(&mut f) {
                Ok(font) => {
                    self.new_settings = NewFontSettings::from_font(&font);
                    self.font = font;
                    self.current_glyph = 0;
                    self.status = format!("Opened: {}", path.display());
                    self.current_path = Some(path);
                    self.save_summary = None;
                }
                Err(e) => self.status = format!("Error reading font: {e}"),
            },
            Err(e) => self.status = format!("Cannot open file: {e}"),
        }
    }

    fn save_font_to(&mut self, path: PathBuf) {
        match std::fs::File::create(&path) {
            Ok(mut f) => match self.font.save(&mut f) {
                Ok(()) => {
                    // The file on disk is now the current format version;
                    // reflect that in the in-memory font (e.g. after upgrading
                    // a loaded version-0 file).
                    self.font.version = crate::font::FORMAT_VERSION;
                    self.status = format!("Saved: {}", path.display());
                    self.save_summary = Some(SaveSummary::from_font(&self.font));
                    self.current_path = Some(path);
                }
                Err(e) => self.status = format!("Error writing font: {e}"),
            },
            Err(e) => self.status = format!("Cannot create file: {e}"),
        }
    }

    fn pick_save_path() -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("Font files", &["fnt"])
            .add_filter("All files", &["*"])
            .save_file()
            .map(|p| {
                if p.extension().is_none() {
                    p.with_extension("fnt")
                } else {
                    p
                }
            })
    }

    fn pick_open_path() -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("Font files", &["fnt"])
            .add_filter("All files", &["*"])
            .pick_file()
    }

    // ── File actions (shared by the menu and keyboard shortcuts) ──────────────

    /// Standard keyboard shortcuts. `COMMAND` maps to ⌘ on macOS and Ctrl
    /// elsewhere.
    const OPEN_SHORTCUT: egui::KeyboardShortcut =
        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
    const SAVE_SHORTCUT: egui::KeyboardShortcut =
        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
    const SAVE_AS_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::S,
    );

    fn open_file(&mut self) {
        if let Some(path) = Self::pick_open_path() {
            self.load_font(path);
        }
    }

    /// Save to the current path if there is one, otherwise prompt for a path.
    fn save_file(&mut self) {
        let path = self.current_path.clone().or_else(Self::pick_save_path);
        if let Some(p) = path {
            self.save_font_to(p);
        }
    }

    fn save_file_as(&mut self) {
        if let Some(p) = Self::pick_save_path() {
            self.save_font_to(p);
        }
    }

    /// Consume any file-related keyboard shortcuts that were pressed this frame.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_shortcut(&Self::OPEN_SHORTCUT)) {
            self.open_file();
        }
        // Check Save As before Save so ⌘⇧S isn't swallowed by the ⌘S binding.
        if ctx.input_mut(|i| i.consume_shortcut(&Self::SAVE_AS_SHORTCUT)) {
            self.save_file_as();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&Self::SAVE_SHORTCUT)) {
            self.save_file();
        }
    }

    // ── UI helpers ────────────────────────────────────────────────────────────

    fn draw_menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New Font…").clicked() {
                    self.new_settings = NewFontSettings::from_font(&self.font);
                    self.show_new_dialog = true;
                    ui.close();
                }
                let open_btn = egui::Button::new("Open…")
                    .shortcut_text(ui.ctx().format_shortcut(&Self::OPEN_SHORTCUT));
                if ui.add(open_btn).clicked() {
                    self.open_file();
                    ui.close();
                }
                ui.separator();
                let save_label = if self.current_path.is_some() {
                    "Save"
                } else {
                    "Save…"
                };
                let save_btn = egui::Button::new(save_label)
                    .shortcut_text(ui.ctx().format_shortcut(&Self::SAVE_SHORTCUT));
                if ui.add(save_btn).clicked() {
                    self.save_file();
                    ui.close();
                }
                let save_as_btn = egui::Button::new("Save As…")
                    .shortcut_text(ui.ctx().format_shortcut(&Self::SAVE_AS_SHORTCUT));
                if ui.add(save_as_btn).clicked() {
                    self.save_file_as();
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("View", |ui| {
                if ui
                    .checkbox(&mut self.show_ascii_table, "ASCII Table")
                    .clicked()
                {
                    ui.close();
                }
            });
        });
    }

    fn draw_settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("Font Info");
        ui.separator();

        egui::Grid::new("font_info_grid")
            .num_columns(2)
            .spacing([4.0, 2.0])
            .show(ui, |ui| {
                ui.label("Size:");
                ui.label(format!(
                    "{}×{} px",
                    self.font.width, self.font.height
                ));
                ui.end_row();

                ui.label("Glyphs/row:");
                ui.label(self.font.glyphs_per_row.to_string());
                ui.end_row();

                ui.label("Rows:");
                ui.label(self.font.rows().to_string());
                ui.end_row();

                ui.label("Total glyphs:");
                ui.label(self.font.total_glyphs.to_string());
                ui.end_row();

                ui.label("First glyph:");
                ui.label(format!(
                    "'{}' (0x{:02X})",
                    self.font.first_glyph as char,
                    self.font.first_glyph
                ));
                ui.end_row();

                ui.label("Encoding:");
                ui.label(if self.font.column_major {
                    "Column-major"
                } else {
                    "Row-major"
                });
                ui.end_row();

                ui.label("Format version:");
                ui.label(self.font.version.to_string());
                ui.end_row();
            });

        ui.add_space(8.0);
        if ui.button("⊞  New Font…").clicked() {
            self.new_settings = NewFontSettings::from_font(&self.font);
            self.show_new_dialog = true;
        }

        ui.add_space(12.0);
        ui.heading("Current Glyph");
        ui.separator();

        let label = self
            .font
            .glyph_char(self.current_glyph)
            .map(|c| format!("'{c}'"))
            .unwrap_or_else(|| format!("#{}", self.current_glyph));

        egui::Grid::new("glyph_info_grid")
            .num_columns(2)
            .spacing([4.0, 2.0])
            .show(ui, |ui| {
                ui.label("Index:");
                ui.label(self.current_glyph.to_string());
                ui.end_row();
                ui.label("Character:");
                ui.label(&label);
                ui.end_row();
            });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("◀ Prev").clicked() && self.current_glyph > 0 {
                self.current_glyph -= 1;
            }
            if ui.button("Next ▶").clicked()
                && self.current_glyph + 1 < self.font.total_glyphs as usize
            {
                self.current_glyph += 1;
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("↔ Center H").clicked() {
                self.font.center_glyph_horizontally(self.current_glyph);
            }
            if ui.button("↕ Center V").clicked() {
                self.font.center_glyph_vertically(self.current_glyph);
            }
        });

        ui.add_space(4.0);
        if ui.button("Clear Glyph").clicked() {
            self.font.clear_glyph(self.current_glyph);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.label("File:");
        if let Some(ref p) = self.current_path {
            ui.label(
                egui::RichText::new(
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("(unknown)"),
                )
                .small(),
            );
        } else {
            ui.label(egui::RichText::new("(unsaved)").small().italics());
        }

        if let Some(summary) = &self.save_summary {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Last saved").strong());
            egui::Grid::new("save_summary_grid")
                .num_columns(2)
                .spacing([6.0, 2.0])
                .show(ui, |ui| {
                    ui.label("File size:");
                    ui.label(format!("{} bytes", summary.file_size));
                    ui.end_row();

                    ui.label("Glyph data:");
                    ui.label(format!("{} bytes", summary.data_size));
                    ui.end_row();

                    ui.label("Bits/glyph:");
                    ui.label(summary.pixels_per_glyph.to_string());
                    ui.end_row();

                    ui.label("Total cells:");
                    ui.label(summary.total_cells.to_string());
                    ui.end_row();

                    ui.label("Filled:");
                    ui.label(summary.filled_cells.to_string());
                    ui.end_row();

                    ui.label("Empty:");
                    ui.label(summary.empty_cells().to_string());
                    ui.end_row();

                    ui.label("Version:");
                    ui.label(summary.version.to_string());
                    ui.end_row();
                });
        }
    }

    fn draw_glyph_strip(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        egui::ScrollArea::horizontal()
            .id_salt("glyph_strip_scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = self.font.width as usize;
                    let h = self.font.height as usize;
                    let px = 2.0_f32; // mini pixel size
                    let label_h = 12.0;
                    let cell_w = (w as f32 * px + 4.0).max(16.0);
                    let cell_h = h as f32 * px + label_h + 4.0;

                    for i in 0..self.font.total_glyphs as usize {
                        let selected = i == self.current_glyph;

                        let (resp, painter) =
                            ui.allocate_painter(Vec2::new(cell_w, cell_h), Sense::click());

                        let bg = if selected {
                            Color32::from_rgb(50, 120, 220)
                        } else if resp.hovered() {
                            Color32::from_gray(80)
                        } else {
                            Color32::from_gray(55)
                        };
                        painter.rect_filled(resp.rect, 2.0, bg);

                        // Character label
                        let label = self
                            .font
                            .glyph_char(i)
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| format!("#{i}"));
                        painter.text(
                            Pos2::new(
                                resp.rect.center().x,
                                resp.rect.min.y + label_h / 2.0 + 1.0,
                            ),
                            egui::Align2::CENTER_CENTER,
                            &label,
                            egui::FontId::proportional(9.0),
                            Color32::WHITE,
                        );

                        // Mini pixel grid
                        let px_origin = Pos2::new(
                            resp.rect.min.x + 2.0,
                            resp.rect.min.y + label_h + 2.0,
                        );
                        for gy in 0..h {
                            for gx in 0..w {
                                if self.font.get_pixel(i, gx, gy) {
                                    let cell = Rect::from_min_size(
                                        Pos2::new(
                                            px_origin.x + gx as f32 * px,
                                            px_origin.y + gy as f32 * px,
                                        ),
                                        Vec2::splat(px - 0.5),
                                    );
                                    painter.rect_filled(cell, 0.0, Color32::WHITE);
                                }
                            }
                        }

                        if resp.clicked() {
                            self.current_glyph = i;
                        }
                    }
                });
            });
    }

    fn draw_new_font_dialog(&mut self, ui: &mut egui::Ui) {
        if !self.show_new_dialog {
            return;
        }

        let mut window_open = true;
        let mut do_create = false;
        let mut do_cancel = false;

        egui::Window::new("New Font")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut window_open)
            .show(ui.ctx(), |ui| {
                egui::Grid::new("new_font_settings")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.label("Width (pixels):");
                        ui.add(
                            egui::DragValue::new(&mut self.new_settings.width)
                                .range(1..=64),
                        );
                        ui.end_row();

                        ui.label("Height (pixels):");
                        ui.add(
                            egui::DragValue::new(&mut self.new_settings.height)
                                .range(1..=64),
                        );
                        ui.end_row();

                        ui.label("Glyphs per row:");
                        ui.add(
                            egui::DragValue::new(&mut self.new_settings.glyphs_per_row)
                                .range(1..=256),
                        );
                        ui.end_row();

                        ui.label("Rows (calculated):");
                        ui.label(self.new_settings.rows_preview().to_string());
                        ui.end_row();

                        ui.label("First glyph:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(
                                &mut self.new_settings.first_glyph_str,
                            )
                            .desired_width(28.0),
                        );
                        if resp.changed() {
                            self.new_settings.first_glyph_str = self
                                .new_settings
                                .first_glyph_str
                                .chars()
                                .next()
                                .filter(|c| c.is_ascii() && !c.is_ascii_control())
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "a".to_string());
                        }
                        ui.end_row();

                        ui.label("Total glyphs:");
                        ui.add(
                            egui::DragValue::new(&mut self.new_settings.total_glyphs)
                                .range(1..=1024),
                        );
                        ui.end_row();

                        ui.label("Encoding:");
                        ui.vertical(|ui| {
                            ui.radio_value(
                                &mut self.new_settings.column_major,
                                false,
                                "Row-major",
                            );
                            ui.radio_value(
                                &mut self.new_settings.column_major,
                                true,
                                "Column-major",
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        do_create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        do_cancel = true;
                    }
                });
            });

        if do_create {
            self.font = self.new_settings.build_font();
            self.current_glyph = 0;
            self.current_path = None;
            self.save_summary = None;
            self.status = format!(
                "New {}×{} font created ({} glyphs, first='{}', {}).",
                self.font.width,
                self.font.height,
                self.font.total_glyphs,
                self.font.first_glyph as char,
                if self.font.column_major {
                    "column-major"
                } else {
                    "row-major"
                }
            );
            self.show_new_dialog = false;
        } else if do_cancel || !window_open {
            self.show_new_dialog = false;
        }
    }

    fn draw_ascii_table(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("ASCII Table");
        ui.label(
            egui::RichText::new("Rows in the font are highlighted; click to edit.")
                .small()
                .weak(),
        );
        ui.separator();

        let first = self.font.first_glyph as usize;
        let last = first + self.font.total_glyphs as usize; // exclusive

        egui::ScrollArea::vertical()
            .id_salt("ascii_table_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for code in 0u8..=127 {
                    let in_font =
                        (code as usize) >= first && (code as usize) < last;
                    let glyph_idx = (code as usize).wrapping_sub(first);
                    let is_current = in_font && glyph_idx == self.current_glyph;

                    let text = format!(
                        "{:>3}  0x{:02X}  {}",
                        code,
                        code,
                        Self::ascii_display(code),
                    );

                    let rich = if in_font {
                        egui::RichText::new(text).monospace()
                    } else {
                        egui::RichText::new(text).monospace().weak()
                    };

                    let mut clicked = false;
                    ui.add_enabled_ui(in_font, |ui| {
                        clicked = ui.selectable_label(is_current, rich).clicked();
                    });
                    if clicked {
                        self.current_glyph = glyph_idx;
                    }
                }
            });
    }

    /// Human-readable label for an ASCII code: control-character abbreviation,
    /// the printable glyph itself, or a placeholder for DEL.
    fn ascii_display(code: u8) -> String {
        const CTRL: [&str; 32] = [
            "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "TAB",
            "LF", "VT", "FF", "CR", "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4",
            "NAK", "SYN", "ETB", "CAN", "EM", "SUB", "ESC", "FS", "GS", "RS", "US",
        ];
        match code {
            0..=31 => CTRL[code as usize].to_string(),
            32 => "SP".to_string(),
            127 => "DEL".to_string(),
            _ => (code as char).to_string(),
        }
    }

    fn draw_glyph_editor(&mut self, ui: &mut egui::Ui) {
        let w = self.font.width as usize;
        let h = self.font.height as usize;

        if w == 0 || h == 0 {
            ui.centered_and_justified(|ui| ui.label("No font loaded."));
            return;
        }

        // ── Glyph label ───────────────────────────────────────────────────
        let char_label = self
            .font
            .glyph_char(self.current_glyph)
            .map(|c| format!("Glyph {}: '{c}'", self.current_glyph))
            .unwrap_or_else(|| format!("Glyph {}", self.current_glyph));
        ui.heading(&char_label);
        ui.add_space(4.0);

        // ── Calculate pixel size to fill available space ──────────────────
        let available = ui.available_size();
        let padding = 20.0;
        let px_size = ((available.x - padding) / w as f32)
            .min((available.y - padding) / h as f32)
            .clamp(6.0, 48.0);

        let grid_w = px_size * w as f32;
        let grid_h = px_size * h as f32;
        let canvas_size = Vec2::new(available.x, grid_h + padding);

        let (response, painter) =
            ui.allocate_painter(canvas_size, Sense::click_and_drag());

        // Centre the grid inside the canvas
        let grid_origin = Pos2::new(
            response.rect.min.x + (available.x - grid_w) / 2.0,
            response.rect.min.y + padding / 2.0,
        );

        // ── Background ────────────────────────────────────────────────────
        painter.rect_filled(response.rect, 0.0, Color32::from_gray(25));

        // ── Pointer interaction ───────────────────────────────────────────
        // Left button paints pixels on; right button erases them.  Holding
        // either button while dragging keeps painting/erasing.
        let ctx = ui.ctx();
        let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
        let paint_value = ctx.input(|i| {
            if i.pointer.button_down(egui::PointerButton::Primary) {
                Some(true)
            } else if i.pointer.button_down(egui::PointerButton::Secondary) {
                Some(false)
            } else {
                None
            }
        });

        if let (Some(pos), Some(val)) = (pointer_pos, paint_value)
            && response.rect.contains(pos)
        {
            let local = pos - grid_origin;
            let gx = (local.x / px_size) as i32;
            let gy = (local.y / px_size) as i32;

            if gx >= 0 && gy >= 0 {
                let (gx, gy) = (gx as usize, gy as usize);
                if gx < w && gy < h {
                    self.font.set_pixel(self.current_glyph, gx, gy, val);
                }
            }
        }

        // ── Draw pixel cells ──────────────────────────────────────────────
        for gy in 0..h {
            for gx in 0..w {
                let cell = Rect::from_min_size(
                    Pos2::new(
                        grid_origin.x + gx as f32 * px_size + 0.5,
                        grid_origin.y + gy as f32 * px_size + 0.5,
                    ),
                    Vec2::splat(px_size - 1.0),
                );

                let fill =
                    if self.font.get_pixel(self.current_glyph, gx, gy) {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(55)
                    };
                painter.rect_filled(cell, 2.0, fill);
            }
        }

        // ── Grid lines ────────────────────────────────────────────────────
        let grid_stroke = Stroke::new(0.5, Color32::from_gray(90));
        for gx in 0..=w {
            let x = grid_origin.x + gx as f32 * px_size;
            painter.line_segment(
                [
                    Pos2::new(x, grid_origin.y),
                    Pos2::new(x, grid_origin.y + grid_h),
                ],
                grid_stroke,
            );
        }
        for gy in 0..=h {
            let y = grid_origin.y + gy as f32 * px_size;
            painter.line_segment(
                [
                    Pos2::new(grid_origin.x, y),
                    Pos2::new(grid_origin.x + grid_w, y),
                ],
                grid_stroke,
            );
        }
    }
}

// ─── eframe App ───────────────────────────────────────────────────────────────

impl eframe::App for FontMakerApp {
    /// Called each time the UI needs repainting.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts (Open / Save / Save As).
        self.handle_shortcuts(ui.ctx());

        // Panels must be added in outside-in order: fixed borders first, then
        // the central area last.
        egui::Panel::top("menu_bar").show(ui, |ui| {
            self.draw_menu_bar(ui);
        });

        // Status bar (bottom, outermost)
        let status = self.status.clone();
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.label(&status);
        });

        // Glyph strip (bottom, above status bar)
        egui::Panel::bottom("glyph_strip")
            .resizable(true)
            .min_size(60.0)
            .show(ui, |ui| {
                self.draw_glyph_strip(ui);
            });

        // Settings panel on the left
        egui::Panel::left("settings_panel")
            .min_size(180.0)
            .default_size(210.0)
            .show(ui, |ui| {
                self.draw_settings_panel(ui);
            });

        // ASCII table on the right (optional)
        if self.show_ascii_table {
            egui::Panel::right("ascii_table_panel")
                .resizable(true)
                .min_size(160.0)
                .default_size(190.0)
                .show(ui, |ui| {
                    self.draw_ascii_table(ui);
                });
        }

        // Floating dialogs (must be shown before CentralPanel)
        self.draw_new_font_dialog(ui);

        // Central panel – glyph editor (must be last)
        egui::CentralPanel::default().show(ui, |ui| {
            self.draw_glyph_editor(ui);
        });
    }
}


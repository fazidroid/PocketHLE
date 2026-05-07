//! egui application: library screen, settings, per-game sheet.

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::{self, Color32, RichText, ScrollArea, Vec2};

use pocket_library::{CpuBackendPref, GameEntry, GameSettings, LauncherConfig, Library};

use crate::runner::{FrameSnapshot, RunOutcome, Runner};

/// Top-level egui app.
pub struct PocketLauncher {
    library: Library,
    selected_game: Option<String>,
    screen: Screen,
    runner: Runner,
    events_rx: Receiver<UiEvent>,
    events_tx: Sender<UiEvent>,
    /// Live framebuffer updates streamed by [`Runner`] while a game
    /// is running. `Some` between [`Self::spawn_run`] and
    /// `UiEvent::RunFinished`; `None` otherwise.
    frame_rx: Option<Receiver<FrameSnapshot>>,
    /// Game currently being launched, used as a status caption
    /// while the run is in progress.
    running_game: Option<String>,
    status: String,
    config_draft: Option<LauncherConfig>,
    game_settings_draft: Option<(String, GameSettings)>,
    last_frame_texture: Option<egui::TextureHandle>,
    last_frame_status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Library,
    Settings,
    GameSettings,
    Run,
}

#[derive(Debug)]
pub enum UiEvent {
    ImportFinished(Result<String, String>),
    RunFinished(RunOutcome),
}

impl PocketLauncher {
    pub fn new(_cc: &eframe::CreationContext<'_>, library: Library) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            library,
            selected_game: None,
            screen: Screen::Library,
            runner: Runner::new(),
            events_rx: rx,
            events_tx: tx,
            frame_rx: None,
            running_game: None,
            status: "Welcome to PocketHLE.".to_string(),
            config_draft: None,
            game_settings_draft: None,
            last_frame_texture: None,
            last_frame_status: None,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                UiEvent::ImportFinished(Ok(name)) => {
                    self.status = format!("Imported {name}.");
                    self.reload_library();
                }
                UiEvent::ImportFinished(Err(e)) => {
                    self.status = format!("Import failed: {e}");
                }
                UiEvent::RunFinished(outcome) => {
                    self.last_frame_status = Some(outcome.summary.clone());
                    if let Some(frame) = outcome.framebuffer {
                        self.upload_frame_texture(ctx, &frame);
                    }
                    self.status = outcome.summary;
                    self.frame_rx = None;
                    self.running_game = None;
                }
            }
        }
        // Drain any live preview frames the background runner may
        // have produced since the last UI tick.
        let mut latest: Option<FrameSnapshot> = None;
        if let Some(rx) = self.frame_rx.as_ref() {
            while let Ok(frame) = rx.try_recv() {
                latest = Some(frame);
            }
        }
        if let Some(frame) = latest {
            self.upload_frame_texture(ctx, &frame);
        }
    }

    fn upload_frame_texture(&mut self, ctx: &egui::Context, frame: &FrameSnapshot) {
        let size = [frame.width as usize, frame.height as usize];
        let img = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
        let tex = ctx.load_texture("pockethle-fb", img, egui::TextureOptions::NEAREST);
        self.last_frame_texture = Some(tex);
    }

    fn reload_library(&mut self) {
        match Library::open(self.library.root()) {
            Ok(lib) => self.library = lib,
            Err(e) => self.status = format!("Could not reload library: {e}"),
        }
    }

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("PocketHLE");
            ui.label(
                RichText::new("Pocket PC / Windows Mobile launcher")
                    .small()
                    .color(Color32::from_gray(160)),
            );
            ui.add_space(16.0);
            if ui
                .selectable_label(self.screen == Screen::Library, "Library")
                .clicked()
            {
                self.screen = Screen::Library;
            }
            if ui
                .selectable_label(self.screen == Screen::Settings, "Settings")
                .clicked()
            {
                self.config_draft = Some(self.library.config().clone());
                self.screen = Screen::Settings;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Import .CAB...").clicked() {
                    self.spawn_import_dialog();
                }
            });
        });
        ui.separator();
    }

    fn ui_library(&mut self, ui: &mut egui::Ui) {
        let games: Vec<GameEntry> = self.library.games().to_vec();
        if games.is_empty() {
            ui.add_space(80.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No games yet")
                        .heading()
                        .color(Color32::from_gray(160)),
                );
                ui.add_space(8.0);
                ui.label("Click \"Import .CAB...\" to add a Pocket PC game.");
                ui.add_space(20.0);
                if ui.button("Import .CAB...").clicked() {
                    self.spawn_import_dialog();
                }
            });
            return;
        }
        ScrollArea::vertical().show(ui, |ui| {
            let avail = ui.available_width();
            let card_width = 280.0_f32.min(avail);
            let columns = ((avail / (card_width + 12.0)).floor() as usize).max(1);
            egui::Grid::new("library_grid")
                .num_columns(columns)
                .spacing(Vec2::new(12.0, 12.0))
                .show(ui, |ui| {
                    for (i, game) in games.iter().enumerate() {
                        self.ui_game_card(ui, game, card_width);
                        if (i + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
        });
    }

    fn ui_game_card(&mut self, ui: &mut egui::Ui, game: &GameEntry, width: f32) {
        let frame = egui::Frame::group(ui.style())
            .rounding(8.0)
            .inner_margin(12.0);
        frame.show(ui, |ui| {
            ui.set_width(width);
            ui.set_min_height(120.0);
            ui.horizontal(|ui| {
                let icon = RichText::new("\u{1F4F1}").size(32.0);
                ui.label(icon);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&game.display_name).strong().size(16.0));
                    if let Some(p) = &game.provider {
                        ui.label(RichText::new(p).small().color(Color32::from_gray(170)));
                    }
                    ui.label(
                        RichText::new(format!("CAB: {}", game.source_cab))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                    ui.label(
                        RichText::new(format!("Backend: {}", game.settings.cpu_backend.label()))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    self.spawn_run(game);
                }
                if ui.button("Settings").clicked() {
                    self.selected_game = Some(game.id.clone());
                    self.game_settings_draft = Some((game.id.clone(), game.settings.clone()));
                    self.screen = Screen::GameSettings;
                }
                if ui.button("Remove").clicked() {
                    if let Err(e) = self.library.remove(&game.id) {
                        self.status = format!("Remove failed: {e}");
                    } else {
                        self.status = format!("Removed {}", game.display_name);
                    }
                }
            });
        });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.config_draft.take() else {
            return;
        };
        let mut save_clicked = false;
        let mut cancel_clicked = false;
        ui.heading("Launcher settings");
        ui.add_space(8.0);
        let library_root = self.library.root().display().to_string();
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing(Vec2::new(12.0, 8.0))
            .show(ui, |ui| {
                ui.label("Default CPU backend");
                egui::ComboBox::from_id_source("cpu_backend")
                    .selected_text(draft.default_cpu_backend.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut draft.default_cpu_backend,
                            CpuBackendPref::Stub,
                            CpuBackendPref::Stub.label(),
                        );
                        if cfg!(feature = "unicorn") {
                            ui.selectable_value(
                                &mut draft.default_cpu_backend,
                                CpuBackendPref::Unicorn,
                                CpuBackendPref::Unicorn.label(),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Verbosity (0..3)");
                ui.add(egui::Slider::new(&mut draft.verbosity, 0..=3));
                ui.end_row();

                ui.label("Library root");
                ui.label(library_root);
                ui.end_row();
            });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                save_clicked = true;
            }
            if ui.button("Cancel").clicked() {
                cancel_clicked = true;
            }
        });
        if save_clicked {
            *self.library.config_mut() = draft;
            if let Err(e) = self.library.save() {
                self.status = format!("Could not save settings: {e}");
            } else {
                self.status = "Settings saved.".to_string();
            }
            self.screen = Screen::Library;
        } else if cancel_clicked {
            self.screen = Screen::Library;
        } else {
            self.config_draft = Some(draft);
        }
    }

    fn ui_game_settings(&mut self, ui: &mut egui::Ui) {
        let Some((id, mut draft)) = self.game_settings_draft.take() else {
            return;
        };
        let display_name = self
            .library
            .get(&id)
            .map(|g| g.display_name.clone())
            .unwrap_or_else(|| id.clone());
        let mut save_clicked = false;
        let mut cancel_clicked = false;
        ui.heading(format!("Settings: {display_name}"));
        ui.add_space(8.0);
        egui::Grid::new("game_settings_grid")
            .num_columns(2)
            .spacing(Vec2::new(12.0, 8.0))
            .show(ui, |ui| {
                ui.label("CPU backend");
                egui::ComboBox::from_id_source("game_cpu_backend")
                    .selected_text(draft.cpu_backend.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut draft.cpu_backend,
                            CpuBackendPref::Stub,
                            CpuBackendPref::Stub.label(),
                        );
                        if cfg!(feature = "unicorn") {
                            ui.selectable_value(
                                &mut draft.cpu_backend,
                                CpuBackendPref::Unicorn,
                                CpuBackendPref::Unicorn.label(),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Max slices");
                ui.add(egui::DragValue::new(&mut draft.max_slices).clamp_range(1..=u64::MAX));
                ui.end_row();

                ui.label("Instructions / slice");
                ui.add(
                    egui::DragValue::new(&mut draft.instructions_per_slice)
                        .clamp_range(1..=u64::MAX),
                );
                ui.end_row();

                ui.label("Halt on unimplemented API");
                ui.checkbox(&mut draft.halt_on_unimplemented, "");
                ui.end_row();
            });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                save_clicked = true;
            }
            if ui.button("Cancel").clicked() {
                cancel_clicked = true;
            }
        });
        if save_clicked {
            if let Err(e) = self.library.update_settings(&id, draft) {
                self.status = format!("Could not save game settings: {e}");
            } else {
                self.status = "Game settings saved.".to_string();
            }
            self.screen = Screen::Library;
        } else if cancel_clicked {
            self.screen = Screen::Library;
        } else {
            self.game_settings_draft = Some((id, draft));
        }
    }

    fn ui_run(&mut self, ui: &mut egui::Ui) {
        ui.heading("Run output");
        ui.add_space(8.0);
        if let Some(name) = self.running_game.as_ref() {
            ui.label(format!("Running {name}…"));
        }
        if let Some(s) = self.last_frame_status.as_ref() {
            ui.label(s);
        }
        ui.add_space(8.0);
        if let Some(tex) = self.last_frame_texture.as_ref() {
            let size = tex.size_vec2();
            let avail_w = ui.available_width().min(size.x * 2.0);
            let scale = avail_w / size.x;
            ui.add(egui::Image::from_texture(tex).fit_to_exact_size(size * scale));
        } else {
            ui.label("(no framebuffer captured yet — try Run again)");
        }
        ui.add_space(12.0);
        if ui.button("Back to library").clicked() {
            self.screen = Screen::Library;
        }
    }

    fn spawn_import_dialog(&mut self) {
        let library_root = self.library.root().to_path_buf();
        let last_dir = self.library.config().last_import_dir.clone();
        let tx = self.events_tx.clone();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Import Pocket PC .CAB")
                .add_filter("Cabinet archive", &["cab", "CAB"]);
            if let Some(d) = last_dir {
                dialog = dialog.set_directory(d);
            }
            let Some(path) = dialog.pick_file() else {
                let _ = tx.send(UiEvent::ImportFinished(Err("cancelled".into())));
                return;
            };
            let result = (|| -> Result<String, String> {
                let mut lib = Library::open(&library_root).map_err(|e| e.to_string())?;
                let parent = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(library_root.clone());
                lib.config_mut().last_import_dir = Some(parent);
                lib.save().map_err(|e| e.to_string())?;
                let entry = lib.import_cab(&path).map_err(|e| e.to_string())?;
                Ok(entry.display_name.clone())
            })();
            let _ = tx.send(UiEvent::ImportFinished(result));
        });
        self.status = "Importing...".to_string();
    }

    fn spawn_run(&mut self, game: &GameEntry) {
        self.last_frame_texture = None;
        self.last_frame_status = None;
        self.screen = Screen::Run;
        self.running_game = Some(game.display_name.clone());
        let (frame_tx, frame_rx) = mpsc::channel();
        self.frame_rx = Some(frame_rx);
        let library_root = self.library.root().to_path_buf();
        let game = game.clone();
        let tx = self.events_tx.clone();
        let runner = self.runner.clone();
        std::thread::spawn(move || {
            let outcome = runner.run_game(library_root, game, Some(frame_tx));
            let _ = tx.send(UiEvent::RunFinished(outcome));
        });
        self.status = "Starting emulator...".to_string();
    }
}

impl eframe::App for PocketLauncher {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.ui_top_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                });
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.screen {
            Screen::Library => self.ui_library(ui),
            Screen::Settings => self.ui_settings(ui),
            Screen::GameSettings => self.ui_game_settings(ui),
            Screen::Run => self.ui_run(ui),
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

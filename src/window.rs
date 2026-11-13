//! The small window: same list as the tray menu, but it stays open while you click.
//! Closing it returns to the tray; it's created again next time it's opened.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use eframe::egui;

use crate::{APP_ID, Cmd, HEADING, Shared, UiMsg, icon, tray::label};

struct ShareWindow {
    shared: Arc<Shared>,
    tx: pipewire::channel::Sender<Cmd>,
    ui_rx: Arc<Mutex<mpsc::Receiver<UiMsg>>>,
    quitting: Arc<AtomicBool>,
}

/// Show the window; returns when it is closed.
pub fn open(
    shared: &Arc<Shared>,
    tx: &pipewire::channel::Sender<Cmd>,
    ui_rx: &Arc<Mutex<mpsc::Receiver<UiMsg>>>,
    quitting: &Arc<AtomicBool>,
    scale: f32,
) {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Audio sharing")
            // on Wayland the desktop finds the icon via this id + the .desktop file
            .with_app_id(APP_ID)
            .with_icon(Arc::new(icon::window_icon()))
            // window grows with the UI scale
            .with_inner_size([300.0 * scale, 260.0 * scale])
            .with_min_inner_size([220.0 * scale, 160.0 * scale]),
        ..Default::default()
    };
    let app = ShareWindow {
        shared: shared.clone(),
        tx: tx.clone(),
        ui_rx: ui_rx.clone(),
        quitting: quitting.clone(),
    };
    let shared_c = shared.clone();
    let res = eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| {
            // everything (text, checkboxes, buttons) scaled; Ctrl +/- still works on top
            cc.egui_ctx.set_zoom_factor(scale);
            // let the PipeWire thread wake us when something changes
            *shared_c.egui_ctx.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    );
    *shared.egui_ctx.lock().unwrap() = None;
    if let Err(e) = res {
        eprintln!("window: {e}");
    }
}

impl eframe::App for ShareWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // messages that arrive while the window is open
        while let Ok(msg) = self.ui_rx.lock().unwrap().try_recv() {
            match msg {
                UiMsg::Open => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
                UiMsg::Quit => {
                    self.quitting.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        let entries = self.shared.entries.lock().unwrap().clone();
        let status = self.shared.status.lock().unwrap().clone();
        let any_shared = entries.iter().any(|e| e.shared);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(egui::RichText::new(HEADING).strong());
            if let Some(problem) = status {
                ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {problem}"));
                ui.weak(format!("Config: {}", crate::config::path().display()));
            }
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if entries.is_empty() {
                        ui.weak("(nothing is playing)");
                    }
                    for e in &entries {
                        let mut checked = e.shared;
                        let text = if e.present {
                            egui::RichText::new(label(e))
                        } else {
                            egui::RichText::new(label(e)).weak()
                        };
                        if ui.checkbox(&mut checked, text).changed() {
                            let _ = self.tx.send(Cmd::Toggle(e.key.clone()));
                        }
                    }
                });

            ui.separator();
            if ui
                .add_enabled(any_shared, egui::Button::new("Stop sharing everything"))
                .clicked()
            {
                let _ = self.tx.send(Cmd::StopAll);
            }
        });
    }
}

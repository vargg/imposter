//! The eframe application: owns all UI state and the bridge to background work.

use std::time::{Duration, Instant};

use poster_app::runtime::Bridge;

use crate::theme::{self, PALETTE, Tone};

/// How long shutdown waits for background work before aborting it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Results of background work, applied on the UI thread.
#[derive(Debug, Clone, Copy)]
pub enum Message {
    Started,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
}

pub struct PosterApp {
    bridge: Bridge<Message>,
    status: Status,
    created: Instant,
    first_frame_logged: bool,
}

impl PosterApp {
    pub fn new(ctx: &egui::Context) -> std::io::Result<Self> {
        theme::apply(ctx);
        let repaint = ctx.clone();
        let bridge = Bridge::new(move || repaint.request_repaint())?;
        bridge.spawn(|_| async { Message::Started });
        Ok(Self {
            bridge,
            status: Status::Starting,
            created: Instant::now(),
            first_frame_logged: false,
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn bridge(&self) -> &Bridge<Message> {
        &self.bridge
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Started => self.status = Status::Ready,
        }
    }
}

impl eframe::App for PosterApp {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for message in self.bridge.drain() {
            self.apply(message);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("title").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("Poster").color(PALETTE.accent));
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            let (text, tone) = match self.status {
                Status::Starting => ("Starting…", Tone::Muted),
                Status::Ready => ("Ready", Tone::Ok),
            };
            ui.label(egui::RichText::new(text).color(PALETTE.tone(tone)));
        });
        egui::CentralPanel::default().show(ui, |_ui| {});
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            tracing::info!(
                elapsed_ms = self.created.elapsed().as_millis(),
                "first frame"
            );
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.bridge.shutdown(SHUTDOWN_GRACE) {
            tracing::warn!("background work did not finish before exit");
        }
    }
}

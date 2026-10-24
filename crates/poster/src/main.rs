//! Poster desktop app entry point.

use std::io::IsTerminal as _;

use anyhow::Context as _;
use poster_ui::app::PosterApp;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("POSTER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Poster")
            .with_inner_size([1400.0, 800.0])
            .with_min_inner_size([1000.0, 650.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Poster",
        options,
        Box::new(|cc| Ok(Box::new(PosterApp::new(&cc.egui_ctx)?))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
    .context("the window failed")
}

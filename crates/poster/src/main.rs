//! Poster desktop app entry point.

use std::io::IsTerminal as _;

use anyhow::Context as _;
use poster_app::paths;
use poster_ui::app::{PosterApp, window_state};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("POSTER_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let dir = paths::data_dir().context("no home directory; set POSTER_DATA_DIR")?;
    let store = paths::open_store(&dir)?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Poster")
        .with_inner_size([1400.0, 800.0])
        .with_min_inner_size([1000.0, 650.0]);
    if let Some(window) = window_state(&store.handle()) {
        viewport = viewport
            .with_inner_size([window.width.max(1000.0), window.height.max(650.0)])
            .with_position([window.x, window.y])
            .with_maximized(window.maximized);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Poster",
        options,
        Box::new(|cc| Ok(Box::new(PosterApp::new(&cc.egui_ctx, store)?))),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
    .context("the window failed")
}

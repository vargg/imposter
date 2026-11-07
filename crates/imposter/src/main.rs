//! Imposter entry point: the desktop app, or the command line when given a command.

use std::io::IsTerminal as _;
use std::process::ExitCode;
use std::sync::{Arc, OnceLock};

use anyhow::Context as _;
use imposter_app::paths;
use imposter_ui::app::{ImposterApp, SmokeResult, window_state};
use imposter_ui::layout::MIN_WINDOW;

/// `imposter --smoke-test <url>`: start the real app, send `GET <url>`, close, and exit 0 only
/// when a response arrived. The release workflow runs it on every packaged binary.
fn smoke_url() -> anyhow::Result<Option<String>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(None),
        Some("--smoke-test") => args.next().map(Some).context("--smoke-test needs a URL"),
        Some(other) => anyhow::bail!("unknown argument {other:?}; see imposter --help"),
    }
}

/// Logs go to stderr; the command line keeps them to warnings, since stderr is where it
/// writes its summary.
fn init_logging(default: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(paths::log_filter(default))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default)),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if imposter_cli::wants_command_line(&args) {
        init_logging("warn");
        let status = imposter_cli::main(
            args,
            &mut imposter_cli::Io {
                stdout: &mut std::io::stdout(),
                stderr: &mut std::io::stderr(),
                var: &|name| std::env::var(name).ok(),
            },
        );
        return ExitCode::from(status);
    }
    init_logging("info");
    match desktop() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn desktop() -> anyhow::Result<()> {
    let smoke = smoke_url()?;
    let store = paths::open_profile()?;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("I’mposter")
        .with_inner_size([1400.0, 800.0])
        .with_min_inner_size(MIN_WINDOW)
        .with_icon(Arc::new(egui::IconData {
            rgba: imposter_ui::icons::app_icon(256),
            width: 256,
            height: 256,
        }));
    if let Some(window) = window_state(&store.handle()) {
        viewport = viewport
            .with_inner_size([
                window.width.max(MIN_WINDOW[0]),
                window.height.max(MIN_WINDOW[1]),
            ])
            .with_position([window.x, window.y])
            .with_maximized(window.maximized);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..eframe::NativeOptions::default()
    };
    let report = Arc::new(OnceLock::<SmokeResult>::new());
    let requested = smoke.is_some();
    let shared = Arc::clone(&report);
    eframe::run_native(
        "I’mposter",
        options,
        Box::new(move |cc| {
            let mut app = ImposterApp::new(&cc.egui_ctx, store)?;
            if let Some(url) = smoke {
                app.smoke_test(url, shared);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))
    .context("the window failed")?;
    match (requested, report.get()) {
        (false, _) => Ok(()),
        (true, Some(Ok(status))) => {
            println!("smoke test: HTTP {status}");
            Ok(())
        }
        (true, Some(Err(error))) => anyhow::bail!("smoke test failed: {error}"),
        (true, None) => {
            anyhow::bail!("smoke test failed: the app closed before a response arrived")
        }
    }
}

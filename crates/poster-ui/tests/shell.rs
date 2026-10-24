//! The application shell through the real eframe app.
#![cfg(test)]

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use poster_ui::app::{PosterApp, Status};

fn harness() -> Harness<'static, PosterApp> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 650.0))
        .build_eframe(|cc| PosterApp::new(&cc.egui_ctx).unwrap())
}

/// Wait for background work, then run frames so the UI applies its results.
fn settle(harness: &mut Harness<'_, PosterApp>) {
    assert!(harness.state().bridge().wait_idle(Duration::from_secs(5)));
    harness.run();
}

#[test]
fn startup_work_reaches_the_ui_through_the_bridge() {
    let mut harness = harness();

    settle(&mut harness);

    assert_eq!(harness.state().status(), &Status::Ready);
    harness.get_by_label("Ready");
    harness.get_by_label("Poster");
}

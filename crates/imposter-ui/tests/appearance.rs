//! Appearance: the density setting packs lists and controls, applies at once and is
//! remembered.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use imposter_app::store::kv;
use imposter_ui::app::{Action, Appearance};
use imposter_ui::layout::MIN_WINDOW;
use imposter_ui::theme::{Density, ThemeChoice};
use support::{click, collection, request, settle, start_with};

fn many_requests() -> support::App {
    start_with(|store| {
        let api = collection(store, "API");
        for i in 0..20 {
            request(store, api.id, &format!("Request {i:02}"), "http://x");
        }
    })
}

fn tops(app: &support::App) -> Vec<f32> {
    let mut tops: Vec<f32> = app
        .harness
        .query_all_by_label_contains("GET Request ")
        .map(|node| node.rect().top())
        .collect();
    tops.sort_by(f32::total_cmp);
    tops
}

fn set_density(app: &mut support::App, density: Density) {
    app.harness
        .state_mut()
        .state
        .actions
        .push(Action::SetAppearance(Appearance {
            theme: ThemeChoice::Dark,
            density,
        }));
    app.harness.run_steps(3);
}

#[test]
fn rows_follow_the_density() {
    let mut app = many_requests();
    for density in [Density::Compact, Density::Comfortable] {
        set_density(&mut app, density);
        let tops = tops(&app);
        let pitch = tops[1] - tops[0];
        let row = density.spacing().row;
        assert!(
            (pitch - (row + 2.0)).abs() <= 1.0,
            "{density:?}: rows {pitch} apart, expected {row} + 2"
        );
    }
}

#[test]
fn compact_shows_more_of_the_tree_at_the_smallest_window() {
    let mut app = many_requests();
    app.harness.set_size(egui::Vec2::from(MIN_WINDOW));
    app.harness.run_steps(3);
    let visible = |app: &support::App| {
        let bottom = app.harness.get_by_label("Hide Collections").rect().top();
        tops(app).iter().filter(|top| **top + 20.0 < bottom).count()
    };
    let compact = visible(&app);
    assert!(
        compact >= 10,
        "{compact} requests visible (7 before densities)"
    );
    set_density(&mut app, Density::Comfortable);
    assert!(visible(&app) < compact, "comfortable shows fewer");
}

#[test]
fn the_density_is_chosen_in_settings_and_remembered() {
    let mut app = many_requests();
    click(&mut app.harness, "Settings");
    settle(&mut app.harness);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Density")
        .click();
    app.harness.run_steps(2);
    click(&mut app.harness, "Comfortable");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .ctx
            .style_of(egui::Theme::Dark)
            .spacing
            .interact_size
            .y,
        Density::Comfortable.spacing().control
    );
    let stored: serde_json::Value = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "appearance"))
        .unwrap()
        .unwrap();
    assert_eq!(stored["density"], "comfortable");
    assert_eq!(stored["theme"], "dark", "the theme is kept");
}

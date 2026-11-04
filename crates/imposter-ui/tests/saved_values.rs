//! Saved values: kept per scope beside a variable, chosen from the inspector to change what a
//! request sends, and edited on the Variables screen under its unsaved-changes guard.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{SavedValues, Variables};
use imposter_app::store::{environments, scopes};
use imposter_testkit::{Server, response};
use imposter_ui::app::Screen;
use imposter_ui::vars::Scope;
use imposter_ui::widgets::Inspection;
use support::{click, collection, request, settle, start_with};

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn saved(name: &str, values: &[&str]) -> SavedValues {
    let mut saved = SavedValues::default();
    saved.set(name, values.iter().map(|v| (*v).to_owned()));
    saved
}

/// Globals `base` and `user` (42, with 42 and 77 saved) and an open request using them.
fn with_saved_user(base: String) -> support::App {
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                scopes::write(
                    db,
                    Scope::Global,
                    &vars(&[("base", &base), ("user", "42")]),
                    &saved("user", &["42", "77"]),
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "User", "{{base}}/users/{{user}}");
    });
    click(&mut app.harness, "GET User");
    settle(&mut app.harness);
    app
}

fn inspect(app: &mut support::App, name: &str, masked: bool) {
    let anchor = support::text_input(&app.harness, "Request URL")
        .rect()
        .left_bottom();
    app.harness.state_mut().state.inspect = Some(Inspection {
        name: name.into(),
        masked,
        anchor,
    });
    app.harness.run_steps(2);
}

fn stored(app: &support::App) -> (Variables, SavedValues) {
    app.harness
        .state()
        .store()
        .call_blocking(|db| scopes::read(db, Scope::Global))
        .unwrap()
}

fn disabled(app: &support::App, label: &str) -> bool {
    app.harness
        .get_by_label(label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn choosing_a_saved_value_changes_what_the_request_sends() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"")));
    let mut app = with_saved_user(server.url.clone());

    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    inspect(&mut app, "user", false);
    app.harness.get_by_label("Saved values");
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);
    assert!(
        app.harness.state().state.inspect.is_none(),
        "choosing closes the inspector"
    );
    app.harness.get_by_label("user: saved value chosen");
    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    let targets: Vec<String> = server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets, ["/users/42", "/users/77"]);
    assert_eq!(
        stored(&app),
        (
            vars(&[("base", &server.url), ("user", "77")]),
            saved("user", &["42", "77"])
        ),
        "the list stays; only the current value changed"
    );
}

#[test]
fn the_variables_screen_saves_and_picks_values_under_its_guard() {
    let mut app = with_saved_user("http://h".into());
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);

    click(&mut app.harness, "Variable 2 saved values");
    click(&mut app.harness, "Use saved value 2: 77");
    assert_eq!(app.harness.state().state.variables.rows[1].value, "77");
    app.harness.state_mut().state.variables.rows[1].value = "91".into();
    app.harness.run_steps(2);
    click(&mut app.harness, "Save current value");
    assert!(
        disabled(&app, "Save current value"),
        "a value is saved once"
    );
    click(&mut app.harness, "Remove saved value 1");
    click(&mut app.harness, "Click again to remove saved value 1");
    assert!(app.harness.state().state.variables.dirty());
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);

    assert_eq!(
        stored(&app),
        (
            vars(&[("base", "http://h"), ("user", "91")]),
            saved("user", &["77", "91"])
        )
    );

    app.harness.state_mut().state.variables.rows[1]
        .saved
        .clear();
    app.harness.run_steps(2);
    assert!(
        app.harness.state().state.variables.dirty(),
        "list edits are unsaved changes"
    );
    click(&mut app.harness, "Revert unsaved changes");
    assert!(
        app.harness.state().state.variables.dirty(),
        "a stray click on Revert loses nothing: it asks first"
    );
    support::click_button(&mut app.harness, "Discard changes");
    assert_eq!(
        app.harness.state().state.variables.rows[1].saved,
        ["77", "91"]
    );
}

#[test]
fn unsaved_edits_to_the_scope_block_choosing_and_a_clean_screen_follows_the_choice() {
    let mut app = with_saved_user("http://h".into());
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    app.harness.state_mut().state.variables.rows[0].value = "http://edited".into();
    app.harness.run_steps(2);
    app.harness.state_mut().state.screen = Screen::Requests;
    app.harness.run_steps(2);

    inspect(&mut app, "user", false);
    assert!(disabled(&app, "Use saved value 2: 77"));
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);
    assert_eq!(stored(&app).0["user"], "42", "nothing was written");

    click(&mut app.harness, "Close");
    app.harness.state_mut().state.screen = Screen::Variables;
    app.harness.run_steps(2);
    click(&mut app.harness, "Revert unsaved changes");
    support::click_button(&mut app.harness, "Discard changes");
    app.harness.state_mut().state.screen = Screen::Requests;
    app.harness.run_steps(2);
    inspect(&mut app, "user", false);
    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);

    assert_eq!(stored(&app).0["user"], "77");
    assert_eq!(
        app.harness.state().state.variables.rows[1].value,
        "77",
        "the Variables screen shows the chosen value"
    );
}

#[test]
fn saved_values_inspected_from_a_password_field_stay_masked() {
    let mut app = with_saved_user("http://h".into());
    inspect(&mut app, "user", true);

    app.harness.get_by_label("Use saved value 2: ••••••••");
    assert!(app.harness.query_by_label_contains("77").is_none());
    click(&mut app.harness, "Reveal");
    app.harness.get_by_label("Use saved value 2: 77");
}

#[test]
fn a_value_removed_meanwhile_is_reported_and_not_recreated() {
    let mut app = with_saved_user("http://h".into());
    inspect(&mut app, "user", false);
    app.harness
        .state()
        .store()
        .call_blocking(|db| environments::set_globals(db, &vars(&[("base", "http://h")])))
        .unwrap();

    click(&mut app.harness, "Use saved value 2: 77");
    settle(&mut app.harness);

    app.harness.get_by_label("Value not changed");
    assert_eq!(stored(&app).0, vars(&[("base", "http://h")]));
}

#[test]
fn the_saved_values_menu_marks_the_current_value_and_keeps_remove_at_the_far_end() {
    let mut app = with_saved_user("http://h".into());
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
    // The trigger counts the list, in a neutral color: orange is for selection.
    let trigger = app.harness.get_by_label("Variable 2 saved values").rect();
    let count = app
        .harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text)
                if trigger.contains(text.pos) && text.galley.job.text == "2" =>
            {
                Some(text.fallback_color)
            }
            _ => None,
        })
        .expect("a count beside the icon");
    assert_ne!(count, imposter_ui::theme::palette().accent);
    // Rows with and without saved values keep their remove buttons in one column.
    let first = app.harness.get_by_label("Remove variable 1").rect();
    let second = app.harness.get_by_label("Remove variable 2").rect();
    assert!(
        (first.left() - second.left()).abs() < 0.5,
        "{first:?} {second:?}"
    );

    click(&mut app.harness, "Variable 2 saved values");
    let painted: Vec<(String, egui::Rect)> = app
        .harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some((
                text.galley.job.text.clone(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => None,
        })
        .collect();
    let current: Vec<&String> = painted
        .iter()
        .map(|(text, _)| text)
        .filter(|text| text.ends_with("current"))
        .collect();
    assert_eq!(current, ["42current"], "only the current value is marked");
    assert!(
        painted
            .iter()
            .any(|(text, _)| text == "+ Save \u{201c}42\u{201d}"),
        "the save row names the value it keeps"
    );
    for (position, value) in [(1, "42"), (2, "77")] {
        let text = painted
            .iter()
            .find(|(text, _)| text.starts_with(value))
            .unwrap()
            .1;
        let remove = app
            .harness
            .get_by_label(&format!("Remove saved value {position}"))
            .rect();
        assert!(
            remove.left() - text.right() >= 12.0,
            "{value}: {text:?} too close to {remove:?}"
        );
        let row = app
            .harness
            .get_by_label(&format!("Use saved value {position}: {value}"))
            .rect();
        assert!(
            (row.right() - remove.left()).abs() < 12.0,
            "the value fills the row up to remove"
        );
    }
}

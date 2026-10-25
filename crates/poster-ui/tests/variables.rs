//! Variables and environments: the screen, its guard, the environment picker, autocomplete and
//! the inspector, through the real UI and database.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use poster_app::core::model::{AuthKind, Variables};
use poster_app::store::{environments, executions};
use poster_testkit::{Server, response};
use poster_ui::app::{CloseState, EditorTab, Screen};
use poster_ui::vars::Scope;
use support::{click, click_button, collection, request, settle, start_with, type_into};

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open_screen(app: &mut support::App) {
    click(&mut app.harness, "Variables");
    settle(&mut app.harness);
}

#[test]
fn the_environment_picker_changes_what_a_request_sends() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(Server::start(|_, _| response("200 OK", &[], b"")));
    let base = server.url.clone();
    let mut app = start_with(move |store| {
        store
            .call_blocking(move |db| {
                environments::set_globals(db, &vars(&[("base", &base), ("who", "global")]))
            })
            .unwrap();
        store
            .call_blocking(|db| {
                environments::create_environment(db, "Dev", &vars(&[("who", "dev")]))
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Who", "{{base}}/{{who}}");
    });
    click(&mut app.harness, "GET Who");
    settle(&mut app.harness);

    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    click(&mut app.harness, "Environment");
    click(&mut app.harness, "Dev");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);

    let targets: Vec<String> = server
        .requests()
        .iter()
        .map(|r| r.target().to_owned())
        .collect();
    assert_eq!(targets, ["/global", "/dev"]);
    let records = app
        .harness
        .state()
        .store()
        .call_blocking(|db| executions::page(db, None, 0, 2))
        .unwrap();
    assert_eq!(records[0].environment, "Dev");
    assert!(app.harness.state().state.active_environment.is_some());
}

#[test]
fn globals_are_edited_and_saved_on_the_variables_screen() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("host", "old")])))
            .unwrap();
    });
    open_screen(&mut app);
    assert_eq!(app.harness.state().state.screen, Screen::Variables);

    type_into(&mut app.harness, "Variable 1 value", "-new");
    app.harness.get_by_label("Unsaved");
    click(&mut app.harness, "Add Variable");
    type_into(&mut app.harness, "Variable 2 name", "token");
    type_into(&mut app.harness, "Variable 2 value", "t");
    click(&mut app.harness, "Save Variables");
    settle(&mut app.harness);

    app.harness.get_by_label("Saved");
    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(|db| environments::globals(db))
        .unwrap();
    assert_eq!(stored, vars(&[("host", "old-new"), ("token", "t")]));
}

#[test]
fn duplicate_names_are_refused_and_nothing_is_saved() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
    });
    open_screen(&mut app);
    click(&mut app.harness, "Add Variable");
    type_into(&mut app.harness, "Variable 2 name", "a");
    click(&mut app.harness, "Save Variables");
    settle(&mut app.harness);

    app.harness
        .get_by_label_contains("Variable names must be unique");
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| environments::globals(db))
            .unwrap(),
        vars(&[("a", "1")])
    );
}

#[test]
fn switching_scope_with_unsaved_rows_asks_and_discard_keeps_the_stored_values() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
        collection(store, "API");
    });
    open_screen(&mut app);
    type_into(&mut app.harness, "Variable 1 value", "2");

    click(&mut app.harness, "Editing");
    click(&mut app.harness, "Collection · API");
    app.harness
        .get_by_label("Save variable changes before continuing?");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);

    assert!(matches!(
        app.harness.state().state.variables.scope,
        Scope::Collection(_)
    ));
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| environments::globals(db))
            .unwrap(),
        vars(&[("a", "1")])
    );
}

#[test]
fn saving_from_the_guard_stores_then_switches() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
        collection(store, "API");
    });
    open_screen(&mut app);
    type_into(&mut app.harness, "Variable 1 value", "2");

    click(&mut app.harness, "Editing");
    click(&mut app.harness, "Collection · API");
    click(&mut app.harness, "Save");
    settle(&mut app.harness);

    assert!(matches!(
        app.harness.state().state.variables.scope,
        Scope::Collection(_)
    ));
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| environments::globals(db))
            .unwrap(),
        vars(&[("a", "12")])
    );
}

#[test]
fn environments_are_created_renamed_and_deleted() {
    let mut app = start_with(|_| {});
    open_screen(&mut app);

    click(&mut app.harness, "New environment");
    app.harness.run_steps(2);
    app.harness
        .get_by_label("Environment Name:")
        .type_text("Staging");
    app.harness.run_steps(1);
    click(&mut app.harness, "OK");
    settle(&mut app.harness);
    assert!(matches!(
        app.harness.state().state.variables.scope,
        Scope::Environment(_)
    ));

    click(&mut app.harness, "Rename environment");
    app.harness.run_steps(2);
    app.harness.get_by_label("Name:").type_text(" EU");
    app.harness.run_steps(1);
    click(&mut app.harness, "OK");
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| environments::environments(db))
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["Staging EU"]);

    click(&mut app.harness, "Delete environment");
    click_button(&mut app.harness, "Delete");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| environments::environments(db))
            .unwrap(),
        []
    );
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}

fn with_request() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                environments::set_globals(
                    db,
                    &vars(&[("host", "example.com"), ("secret", "s3cr3t")]),
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Vars", "http://x");
    });
    click(&mut app.harness, "GET Vars");
    settle(&mut app.harness);
    app
}

#[test]
fn typing_an_open_brace_pair_suggests_names_and_enter_completes() {
    let mut app = with_request();
    type_into(&mut app.harness, "Request URL", "/{{ho");

    app.harness.get_by_label("host · Global · Global");
    assert!(
        app.harness.query_by_label_contains("example.com").is_none(),
        "suggestions never show values"
    );
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);

    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .url,
        "http://x/{{host}}"
    );
}

#[test]
fn clicking_a_token_inspects_it_and_open_definition_goes_to_its_scope() {
    let mut app = with_request();
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .url = "http://{{host}}/{{missing}}".into();
    app.harness.run_steps(2);

    let field = support::text_input(&app.harness, "Request URL").rect();
    let at = egui::pos2(field.left() + 70.0, field.center().y);
    app.harness.drag_at(at);
    app.harness.drop_at(at);
    app.harness.run_steps(2);
    let inspected = app
        .harness
        .state()
        .state
        .inspect
        .clone()
        .map(|i| (i.name, i.masked));
    assert_eq!(
        inspected,
        Some(("host".to_owned(), false)),
        "clicking a token opens its inspector"
    );

    app.harness.get_by_label("Effective · Global · Global");
    app.harness.get_by_label("example.com");
    click(&mut app.harness, "Open definition");
    settle(&mut app.harness);

    assert_eq!(app.harness.state().state.screen, Screen::Variables);
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
}

#[test]
fn values_inspected_from_a_password_field_stay_masked_until_revealed() {
    let mut app = with_request();
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = EditorTab::Auth;
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .auth
        .kind = Some(AuthKind::BearerToken);
    app.harness.run_steps(2);
    let anchor = support::text_input(&app.harness, "Token")
        .rect()
        .left_bottom();
    app.harness.state_mut().state.inspect = Some(poster_ui::widgets::Inspection {
        name: "secret".into(),
        masked: true,
        anchor,
    });
    app.harness.run_steps(2);

    app.harness.get_by_label("••••••••");
    assert!(app.harness.query_by_label("s3cr3t").is_none());
    click(&mut app.harness, "Reveal");
    app.harness.get_by_label("s3cr3t");
    click(&mut app.harness, "Hide value");
    assert!(app.harness.query_by_label("s3cr3t").is_none());
    click(&mut app.harness, "Close");
    assert!(app.harness.state().state.inspect.is_none());
}

#[test]
fn undefined_names_show_as_not_defined() {
    let mut app = with_request();
    let anchor = support::text_input(&app.harness, "Request URL")
        .rect()
        .left_bottom();
    app.harness.state_mut().state.inspect = Some(poster_ui::widgets::Inspection {
        name: "nope".into(),
        masked: false,
        anchor,
    });
    app.harness.run_steps(2);
    app.harness.get_by_label("Not defined");
}

#[test]
fn closing_with_unsaved_variables_asks_and_cancel_stays_open() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
    });
    open_screen(&mut app);
    type_into(&mut app.harness, "Variable 1 value", "x");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    app.harness
        .get_by_label("Save variable changes before continuing?");
    click(&mut app.harness, "Cancel");
    assert_eq!(app.harness.state().close_state(), CloseState::Open);

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    click(&mut app.harness, "Save");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(|db| environments::globals(db))
            .unwrap(),
        vars(&[("a", "1x")])
    );
}

#[test]
fn a_suggestion_can_be_chosen_with_the_pointer() {
    let mut app = with_request();
    type_into(&mut app.harness, "Request URL", "/{{ho");

    // A real press and release: the press takes focus from the field first.
    let at = app
        .harness
        .get_by_label("host · Global · Global")
        .rect()
        .center();
    app.harness.drag_at(at);
    app.harness.run_steps(1);
    app.harness.drop_at(at);
    app.harness.run_steps(2);

    let editor = &app.harness.state().state.editor.as_ref().unwrap().editor;
    assert_eq!(editor.url, "http://x/{{host}}");
    assert!(
        app.harness
            .query_by_label("host · Global · Global")
            .is_none(),
        "the list closes after choosing"
    );
}

#[test]
fn clicking_inside_a_finished_token_does_not_suggest() {
    let mut app = with_request();
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .url = "http://{{host}}/".into();
    app.harness.run_steps(2);

    let field = support::text_input(&app.harness, "Request URL").rect();
    let at = egui::pos2(field.left() + 70.0, field.center().y);
    app.harness.drag_at(at);
    app.harness.drop_at(at);
    app.harness.run_steps(2);

    assert!(app.harness.state().state.inspect.is_some());
    assert!(
        app.harness
            .query_by_label("host · Global · Global")
            .is_none(),
        "only the inspector opens"
    );
}

#[test]
fn a_new_variable_keeps_focus_while_its_name_is_typed() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
    });
    open_screen(&mut app);
    click(&mut app.harness, "Add Variable");
    type_into(&mut app.harness, "Variable 2 name", "t");
    app.harness.get_by_label("Unsaved");

    // Keep typing without clicking the field again.
    app.harness.event(egui::Event::Text("oken".into()));
    app.harness.run_steps(2);

    assert_eq!(app.harness.state().state.variables.rows[1].name, "token");
}

#[test]
fn removing_a_row_takes_two_clicks() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1"), ("b", "2")])))
            .unwrap();
    });
    open_screen(&mut app);
    let names = |app: &support::App| -> Vec<String> {
        let rows = &app.harness.state().state.variables.rows;
        rows.iter().map(|r| r.name.clone()).collect()
    };

    click(&mut app.harness, "Remove variable 1");
    assert_eq!(names(&app), ["a", "b"], "the first click only arms");
    click(&mut app.harness, "Click again to remove variable 1");
    assert_eq!(names(&app), ["b"]);

    // Moving the pointer away disarms.
    click(&mut app.harness, "Remove variable 1");
    app.harness.get_by_label("Click again to remove variable 1");
    app.harness.hover_at(egui::pos2(5.0, 5.0));
    app.harness.run_steps(2);
    app.harness.get_by_label("Remove variable 1");

    // So does waiting.
    click(&mut app.harness, "Remove variable 1");
    let armed = app.harness.ctx.input(|i| i.time);
    while app.harness.ctx.input(|i| i.time) - armed <= poster_ui::widgets::REMOVE_ARMED_SECONDS {
        app.harness.step();
    }
    app.harness.run_steps(1);
    app.harness.get_by_label("Remove variable 1");
    assert_eq!(names(&app), ["b"]);
}

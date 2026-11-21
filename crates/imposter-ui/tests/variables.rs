//! Variables and environments: the screen, its guard, the environment picker, autocomplete and
//! the inspector, through the real UI and database.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::{AuthKind, EnvColor, Variables};
use imposter_app::core::variables::SecretScope;
use imposter_app::http::cookies::Jar;
use imposter_app::store::{cookies, environments, executions, secrets};
use imposter_testkit::{Server, response};
use imposter_ui::app::{CloseState, EditorTab, Screen};
use imposter_ui::icons::Icon;
use imposter_ui::vars::Scope;
use support::{
    click, click_button, collection, request, save_state, settle, start_with, type_into,
};

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn open_screen(app: &mut support::App) {
    support::open_screen(&mut app.harness, "Variables");
    settle(&mut app.harness);
}

fn open_wide_screen(app: &mut support::App) {
    app.harness.set_size(egui::vec2(1600.0, 900.0));
    open_screen(app);
}

fn assert_columns_align(app: &support::App, first: &str, second: &str) {
    let left = |label| app.harness.get_by_label(label).rect().left();
    assert!(
        (left(first) - left(second)).abs() <= 1.0,
        "{first} at {} and {second} at {} should align",
        left(first),
        left(second)
    );
}

fn assert_row_reaches_pane_edge(app: &support::App, fields: &[&str]) {
    let mut bounds = fields
        .iter()
        .map(|label| app.harness.get_by_label(label).rect());
    let pane_edge = app.harness.ctx.content_rect().right();
    let margin = pane_edge - bounds.clone().next_back().unwrap().right();
    assert!(
        (margin - 16.0).abs() <= 1.0,
        "{fields:?} ends {margin} points from the pane edge, expected the 16-point screen margin"
    );

    let mut last = bounds.next().unwrap();
    let gap = imposter_ui::theme::spacing().sm;
    for next in bounds {
        assert!(
            (next.left() - last.right() - gap).abs() <= 1.0
                && (next.center().y - last.center().y).abs() <= 1.0,
            "{fields:?} should share a row with standard spacing: {last:?}, {next:?}"
        );
        last = next;
    }
}

#[test]
fn wide_variables_rows_reach_the_pane_edge() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                environments::set_globals(db, &vars(&[("host", "test"), ("lang", "en")]))
            })
            .unwrap();
    });
    open_wide_screen(&mut app);

    for (row, value) in [(1, "test"), (2, "en")] {
        assert_row_reaches_pane_edge(
            &app,
            &[
                &format!("Variable {row} name"),
                &format!("Variable {row} value 1: {value}"),
                &format!("Add value to variable {row}"),
                &format!("Remove variable {row}"),
            ],
        );
    }
    assert_columns_align(&app, "Variable name", "New variable name");
    assert_columns_align(&app, "Variable name", "Variable 2 name");
    assert_columns_align(&app, "Values", "Variable 2 value 1: en");

    let name = app.harness.get_by_label("Variable 2 name").rect();
    let remove = app.harness.get_by_label("Remove variable 2").rect();
    let share = name.width() / (remove.right() - name.left());
    assert!((0.33..=0.36).contains(&share), "name column share: {share}");
}

#[test]
fn wide_secrets_rows_reach_the_pane_edge_with_the_eye_button() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                secrets::write(db, SecretScope::Global, &vars(&[("token", "token-1234")]))
            })
            .unwrap();
    });
    open_wide_screen(&mut app);
    click(&mut app.harness, "Secrets");

    assert_row_reaches_pane_edge(
        &app,
        &[
            "Secret 1 name",
            "Secret 1 value",
            "Show secret 1",
            "Remove secret 1",
        ],
    );
    assert_columns_align(&app, "Secret name", "Secret 1 name");
    assert_columns_align(&app, "Secret name", "New secret name");
    assert_columns_align(&app, "Value", "Secret 1 value");
    assert_columns_align(&app, "Value", "New secret value");
}

#[test]
fn wide_cookies_rows_and_the_clear_button_reach_the_pane_edge_in_comfortable_density() {
    let mut app = start_with(|store| {
        let mut jar = Jar::default();
        assert!(jar.receive("https://example.test", "session=example; Path=/"));
        store
            .call_blocking(move |db| cookies::write(db, None, &jar.to_json()))
            .unwrap();
    });
    support::open_screen(&mut app.harness, "Settings");
    settle(&mut app.harness);
    click(&mut app.harness, "Density: Comfortable");
    settle(&mut app.harness);
    open_wide_screen(&mut app);
    click(&mut app.harness, "Cookies");

    assert_row_reaches_pane_edge(
        &app,
        &[
            "Cookie session value",
            "Show cookie session",
            "Remove cookie session",
        ],
    );
    assert_row_reaches_pane_edge(&app, &["Clear cookies…"]);
    assert_columns_align(&app, "Value", "Cookie session value");
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

    type_into(&mut app.harness, "Add value to variable 1", "old-new");
    assert_eq!(save_state(&app.harness, "Save variables"), "Unsaved");
    type_into(&mut app.harness, "New variable name", "token");
    type_into(&mut app.harness, "Add value to variable 2", "t");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);

    assert_eq!(save_state(&app.harness, "Save variables"), "Saved");
    let stored = app
        .harness
        .state()
        .store()
        .call_blocking(environments::globals)
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
    type_into(&mut app.harness, "New variable name", "a");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);

    app.harness
        .get_by_label_contains("Variable names must be unique");
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(environments::globals)
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
    type_into(&mut app.harness, "Add value to variable 1", "12");

    click(&mut app.harness, "Collection · API");
    app.harness
        .get_by_label("Save variable changes before continuing?");
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
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
            .call_blocking(environments::globals)
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
    type_into(&mut app.harness, "Add value to variable 1", "12");

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
            .call_blocking(environments::globals)
            .unwrap(),
        vars(&[("a", "12")])
    );
}

#[test]
fn environments_are_created_renamed_and_deleted() {
    let mut app = start_with(|_| {});
    open_screen(&mut app);

    support::assert_icon_button(&app.harness, "New environment");
    click(&mut app.harness, "New environment");
    app.harness.run_steps(2);
    support::text_input(&app.harness, "Name").type_text("Staging");
    app.harness.run_steps(1);
    click_button(&mut app.harness, "Create");
    settle(&mut app.harness);
    assert!(matches!(
        app.harness.state().state.variables.scope,
        Scope::Environment(_)
    ));

    support::open_environment_menu(&mut app.harness, "Staging");
    click(&mut app.harness, "Edit name and color…");
    app.harness.run_steps(2);
    support::text_input(&app.harness, "Name").type_text(" EU");
    app.harness.run_steps(1);
    click_button(&mut app.harness, "Save");
    settle(&mut app.harness);
    let names: Vec<String> = app
        .harness
        .state()
        .store()
        .call_blocking(environments::environments)
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["Staging EU"]);

    support::open_environment_menu(&mut app.harness, "Staging EU");
    click(&mut app.harness, "Delete environment…");
    click_button(&mut app.harness, "Delete environment");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(environments::environments)
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
fn a_placeholder_typed_in_a_json_body_closes_and_still_suggests() {
    let mut app = with_request();
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = EditorTab::Body;
        pane.editor.body_type = imposter_app::core::model::BodyType::Json;
        pane.editor.body_text = "{\"n\": }".into();
    }
    app.harness.run_steps(2);
    support::text_input(&app.harness, "Request body").focus();
    app.harness.run_steps(1);
    // Before the closing brace.
    app.harness.key_press(egui::Key::End);
    app.harness.run_steps(1);
    app.harness.key_press(egui::Key::ArrowLeft);
    app.harness.run_steps(1);
    for typed in ["{", "{", "h"] {
        app.harness.event(egui::Event::Text(typed.into()));
        app.harness.run_steps(1);
    }
    let body = |app: &support::App| {
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .body_text
            .clone()
    };
    assert_eq!(body(&app), "{\"n\": {{h}}}");
    app.harness.get_by_label("host · Global · Global");
    app.harness.key_press(egui::Key::Enter);
    app.harness.run_steps(2);
    assert_eq!(body(&app), "{\"n\": {{host}}}", "no stray braces");
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

    app.harness.get_by_label("Defined in Global");
    app.harness.get_by_label("example.com");
    // The pointer up to the token is drawn in the control-edge color, so it can be seen.
    let edge = imposter_ui::theme::palette().field_border;
    let tip = app.harness.state().state.inspect.as_ref().unwrap().anchor + egui::vec2(0.0, 1.0);
    let pointer = app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::LineSegment { points, stroke }
            if stroke.color == edge && points.contains(&tip))
    });
    assert!(pointer, "a visible pointer at {tip:?}");
    // Opened under the token, not at the field's edge: "http://" comes before it.
    let anchor = app.harness.state().state.inspect.as_ref().unwrap().anchor;
    assert!(
        anchor.x > field.left() + 30.0 && anchor.x < at.x,
        "{anchor:?} in {field:?}"
    );
    app.harness.get_by_label("Copy value").click();
    app.harness.step();
    let copied: Vec<String> = app
        .harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, ["example.com"]);
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
    app.harness.state_mut().state.inspect = Some(imposter_ui::widgets::Inspection {
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
    // Copy and close are icons, as everywhere else.
    support::assert_icon_button(&app.harness, "Copy value");
    support::assert_icon_button(&app.harness, "Close");
    click(&mut app.harness, "Close");
    assert!(app.harness.state().state.inspect.is_none());
}

fn inspect(app: &mut support::App, name: &str) {
    let anchor = support::text_input(&app.harness, "Request URL")
        .rect()
        .left_bottom();
    app.harness.state_mut().state.inspect = Some(imposter_ui::widgets::Inspection {
        name: name.into(),
        masked: false,
        anchor,
    });
    app.harness.run_steps(2);
}

#[test]
fn undefined_names_show_as_not_defined_and_can_be_defined() {
    let mut app = with_request();
    inspect(&mut app, "nope");
    app.harness.get_by_label("Not defined");
    assert!(app.harness.query_by_label("Open definition").is_none());

    click(&mut app.harness, "Define in Global");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::Variables);
    assert_eq!(app.harness.state().state.variables.scope, Scope::Global);
    let rows = &app.harness.state().state.variables.rows;
    assert_eq!(rows.last().map(|r| r.name.as_str()), Some("nope"));
}

#[test]
fn generated_values_have_no_definition_to_open() {
    let mut app = with_request();
    inspect(&mut app, "$uuid");
    app.harness.get_by_label_contains("For example: ");
    assert!(app.harness.query_by_label("Open definition").is_none());
    assert!(app.harness.query_by_label_contains("Define in").is_none());
}

#[test]
fn in_a_body_the_inspector_and_suggestions_open_under_the_token_line() {
    let mut app = with_request();
    {
        let pane = app.harness.state_mut().state.editor.as_mut().unwrap();
        pane.tab = EditorTab::Body;
        pane.editor.body_type = imposter_app::core::model::BodyType::Json;
        pane.editor.body_text = "{{host}}\n".into();
    }
    app.harness.run_steps(2);
    let body = support::text_input(&app.harness, "Request body").rect();
    assert!(body.height() > 120.0, "a tall editor: {body:?}");
    let line = egui::pos2(body.left() + 20.0, body.top() + 10.0);
    app.harness.drag_at(line);
    app.harness.drop_at(line);
    app.harness.run_steps(2);
    let anchor = app.harness.state().state.inspect.as_ref().unwrap().anchor;
    assert!(
        anchor.y < body.top() + 40.0,
        "under the first line, not the editor's bottom: {anchor:?} in {body:?}"
    );
    app.harness.key_press(egui::Key::Escape);
    app.harness.run_steps(2);

    support::text_input(&app.harness, "Request body").focus();
    app.harness.run_steps(1);
    app.harness.key_press(egui::Key::End);
    app.harness.run_steps(1);
    app.harness.key_press(egui::Key::ArrowDown);
    app.harness.run_steps(1);
    for typed in ["{", "{", "h"] {
        app.harness.event(egui::Event::Text(typed.into()));
        app.harness.run_steps(1);
    }
    let suggestion = app.harness.get_by_label("host · Global · Global").rect();
    assert!(
        suggestion.top() < body.top() + 80.0,
        "under the second line: {suggestion:?} in {body:?}"
    );
}

#[test]
fn closing_with_unsaved_variables_asks_and_cancel_stays_open() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1")])))
            .unwrap();
    });
    open_screen(&mut app);
    type_into(&mut app.harness, "Add value to variable 1", "1x");

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
            .call_blocking(environments::globals)
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
    type_into(&mut app.harness, "New variable name", "t");
    assert_eq!(save_state(&app.harness, "Save variables"), "Unsaved");

    assert!(support::text_input(&app.harness, "Variable 2 name").is_focused());
    support::text_input(&app.harness, "New variable name");
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
    while app.harness.ctx.input(|i| i.time) - armed <= imposter_ui::widgets::REMOVE_ARMED_SECONDS {
        app.harness.step();
    }
    app.harness.run_steps(1);
    app.harness.get_by_label("Remove variable 1");
    assert_eq!(names(&app), ["b"]);
}

#[test]
fn a_dollar_suggests_generated_values_and_the_inspector_explains_them() {
    let mut app = with_request();
    type_into(&mut app.harness, "Request URL", "/{{$rand");
    app.harness
        .get_by_label_contains("$randomInt · Generated · ");
    assert!(
        app.harness.query_by_label_contains("host ·").is_none(),
        "only names containing the query"
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
        "http://x/{{$randomUUID}}"
    );

    for (name, expected) in [
        ("$randomUUID", "A random UUID (Postman's name)"),
        ("$randomInt 9 1", "$randomInt needs min ≤ max"),
        (
            "$randomFirstName",
            "$randomFirstName is not a generated value",
        ),
    ] {
        app.harness.state_mut().state.inspect = Some(imposter_ui::widgets::Inspection {
            name: name.into(),
            masked: false,
            anchor: egui::pos2(100.0, 100.0),
        });
        app.harness.run_steps(2);
        app.harness.get_by_label(expected);
    }
    app.harness.state_mut().state.inspect = Some(imposter_ui::widgets::Inspection {
        name: "$uuid".into(),
        masked: false,
        anchor: egui::pos2(100.0, 100.0),
    });
    app.harness.run_steps(2);
    app.harness
        .get_by_label_contains("Generated on each send; the response's Request tab");
    let example = app
        .harness
        .get_by_label_contains("For example: ")
        .value()
        .unwrap_or_default();
    app.harness.run_steps(3);
    assert_eq!(
        app.harness
            .get_by_label_contains("For example: ")
            .value()
            .unwrap_or_default(),
        example,
        "the example holds still while the inspector is open"
    );
    assert_eq!(example.trim_start_matches("For example: ").len(), 36);
}

#[test]
fn the_scope_list_orders_groups_and_marks_the_active_environment() {
    let mut app = with_scopes();
    let global = app.harness.get_by_label("Global");
    assert_eq!(global.accesskit_node().is_selected(), Some(true));
    let dev = app.harness.get_by_label("Environment · Dev");
    let prod = app.harness.get_by_label("Environment · Prod");
    assert_eq!(dev.accesskit_node().is_selected(), Some(false));
    assert_eq!(
        dev.accesskit_node().description().as_deref(),
        Some("Active: requests use this environment"),
    );
    assert_eq!(prod.accesskit_node().description().as_deref(), Some("Prod"));
    let glyph_in = |row: egui::Rect, icon: Icon| {
        let glyph = icon.glyph().to_string();
        app.harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Text(text)
                if text.galley.job.text == glyph && row.contains(text.pos))
        })
    };
    assert!(glyph_in(global.rect(), Icon::Globe));
    assert!(glyph_in(
        app.harness.get_by_label("Collection · API").rect(),
        Icon::Folder
    ));
    assert!(glyph_in(dev.rect(), Icon::Check));
    assert!(!glyph_in(prod.rect(), Icon::Check));
    let red = imposter_ui::theme::env_color(EnvColor::Red);
    assert!(app.harness.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, egui::Shape::Circle(dot)
            if dot.fill == red && prod.rect().contains(dot.center))
    }));
    for pair in [
        ["Global", "Environments"],
        ["Environments", "Environment · Dev"],
        ["Environment · Dev", "Environment · Prod"],
        ["Environment · Prod", "Collections"],
        ["Collections", "Collection · API"],
        ["Collection · API", "Collection · Webhooks"],
    ] {
        let first = app.harness.get_by_label(pair[0]).rect();
        let second = app.harness.get_by_label(pair[1]).rect();
        assert!(
            first.bottom() <= second.top(),
            "{pair:?} should be in order"
        );
    }
    assert!(app.harness.query_by_label("Editing").is_none());
    assert!(app.harness.query_by_label("Variable scope").is_none());

    click(&mut app.harness, "Environment · Prod");
    settle(&mut app.harness);
    let state = &app.harness.state().state;
    let dev = state
        .environments
        .iter()
        .find(|env| env.name == "Dev")
        .unwrap();
    let prod = state
        .environments
        .iter()
        .find(|env| env.name == "Prod")
        .unwrap();
    assert_eq!(state.active_environment, Some(dev.id));
    assert_eq!(state.variables.scope, Scope::Environment(prod.id));
    assert_eq!(
        app.harness
            .get_by_label("Environment · Prod")
            .accesskit_node()
            .is_selected(),
        Some(true),
    );
}

fn list_column(app: &support::App, name: &str) -> (egui::Rect, f32) {
    use egui::accesskit::Role;
    let column = app.harness.get_by_role_and_label(Role::Group, name).rect();
    let toolbar = app.harness.get_by_role(Role::Toolbar).rect();
    let gap = column.top() - toolbar.bottom();
    assert!(gap > 0.0, "{name} must begin below the toolbar");
    let rail = app.harness.ctx.content_rect().left() + imposter_ui::layout::RAIL_WIDTH;
    assert!(
        (column.left() - rail).abs() <= 0.5,
        "{name} must touch the rail"
    );
    let palette = imposter_ui::theme::palette();
    let sample = column.center_bottom() - egui::vec2(0.0, 4.0);
    let shapes = &app.harness.output().shapes;
    let background = shapes
        .iter()
        .rev()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if rect.rect.contains(sample)
                    && clipped.clip_rect.contains(sample)
                    && rect.fill != egui::Color32::TRANSPARENT =>
            {
                Some(rect.fill)
            }
            _ => None,
        });
    assert_eq!(background, Some(palette.background), "{name} background");
    assert!(
        shapes.iter().any(|clipped| matches!(clipped.shape,
            egui::Shape::LineSegment { points: [a, b], stroke }
                if (a.x - column.right()).abs() <= 1.0
                    && (a.x - b.x).abs() < 0.5
                    && (a.y - column.top()).abs() < 0.5
                    && (b.y - column.bottom()).abs() < 0.5
                    && (stroke.width - 1.0).abs() < 0.001
                    && stroke.color == palette.border
        )),
        "{name} divider"
    );
    (column, gap)
}

fn list_label_left(app: &support::App, row_name: &str, label: &str) -> f32 {
    let bounds = app.harness.get_by_label(row_name).rect();
    app.harness
        .output()
        .shapes
        .iter()
        .find_map(|clipped| {
            let egui::Shape::Text(text) = &clipped.shape else {
                return None;
            };
            if !bounds.contains(text.pos) {
                return None;
            }
            let prefix = text.galley.job.text.strip_suffix(label)?;
            let line = text.galley.rows.first()?;
            let first = line.glyphs.get(prefix.chars().count())?;
            Some(text.pos.x + line.pos.x + first.pos.x)
        })
        .unwrap_or_else(|| panic!("No painted label for {row_name}"))
}

#[test]
fn scope_names_and_settings_categories_align_in_matching_columns() {
    let mut app = with_scopes();
    for size in [
        egui::Vec2::from(imposter_ui::layout::MIN_WINDOW),
        egui::vec2(1400.0, 900.0),
    ] {
        app.harness.set_size(size);
        open_screen(&mut app);
        let (scopes, scopes_gap) = list_column(&app, "Variable scopes");
        let global = list_label_left(&app, "Global", "Global");
        for (row, label) in [
            ("Environment · Dev", "Dev"),
            ("Environment · Prod", "Prod"),
            ("Collection · API", "API"),
            ("Collection · Webhooks", "Webhooks"),
        ] {
            let left = list_label_left(&app, row, label);
            assert!(
                (left - global).abs() <= 1.0,
                "{row} at {left}, Global at {global}, size {size:?}"
            );
        }
        support::open_screen(&mut app.harness, "Settings");
        settle(&mut app.harness);
        let (categories, categories_gap) = list_column(&app, "Settings categories");
        assert_eq!(scopes.x_range(), categories.x_range(), "size {size:?}");
        assert_eq!(scopes_gap, categories_gap, "size {size:?}");
        for category in [
            "Appearance",
            "Collections",
            "General",
            "Network",
            "TLS",
            "History",
            "Command line",
        ] {
            let row = format!("{category} settings");
            let left = list_label_left(&app, &row, category);
            assert!(
                (left - global).abs() <= 1.0,
                "{row} at {left}, Global at {global}, size {size:?}"
            );
            let bounds = app.harness.get_by_label(&row).rect();
            assert_eq!(
                bounds.x_range(),
                categories.x_range(),
                "{row} spans the column"
            );
        }
        let selected = app.harness.get_by_label("Appearance settings").rect();
        let network = app.harness.get_by_label("Network settings");
        let hovered = network.rect();
        network.hover();
        app.harness.run_steps(2);
        let palette = imposter_ui::theme::palette();
        for (row, fill) in [(selected, palette.selected), (hovered, palette.hover)] {
            assert!(
                app.harness
                    .output()
                    .shapes
                    .iter()
                    .any(|clipped| matches!(&clipped.shape,
                        egui::Shape::Rect(shape) if shape.rect == row && shape.fill == fill
                    )),
                "missing row fill at {row:?}"
            );
        }
        assert!(
            app.harness
                .output()
                .shapes
                .iter()
                .any(|clipped| matches!(&clipped.shape,
                    egui::Shape::Rect(shape) if shape.fill == palette.selected_bar
                        && (shape.rect.left() - selected.left()).abs() < 0.5
                        && selected.contains_rect(shape.rect)
                        && (shape.rect.width() - 2.0).abs() < 0.001
                )),
            "missing selected row's accent bar"
        );
    }
}

#[test]
fn the_variables_column_resizes_with_settings_and_keeps_its_width_after_restart() {
    let mut app = with_scopes();
    let wide = egui::vec2(1400.0, 900.0);
    let small = egui::Vec2::from(imposter_ui::layout::MIN_WINDOW);
    let column = |app: &support::App, name| {
        app.harness
            .get_by_role_and_label(egui::accesskit::Role::Group, name)
            .rect()
    };
    let scopes = |app: &support::App| column(app, "Variable scopes");
    let categories = |app: &support::App| column(app, "Settings categories");

    open_screen(&mut app);
    let before = scopes(&app);
    assert!((before.width() - 191.0).abs() <= 1.0);
    app.harness.set_size(small);
    app.harness.run_steps(2);
    let narrow = scopes(&app);
    assert!(narrow.width() < before.width() - 30.0);
    app.harness.set_size(wide);
    app.harness.run_steps(2);
    assert_eq!(scopes(&app).x_range(), before.x_range());

    support::drag_horizontally(&mut app.harness, before.right_center(), 72.0);
    let wider = scopes(&app);
    assert!(
        wider.width() > before.width() + 60.0,
        "{before:?} -> {wider:?}"
    );
    support::assert_split_grip(&app.harness, wider.right_center());
    let global = list_label_left(&app, "Global", "Global");
    support::open_screen(&mut app.harness, "Settings");
    settle(&mut app.harness);
    assert_eq!(categories(&app).x_range(), wider.x_range());
    support::assert_split_grip(&app.harness, wider.right_center());
    let appearance = list_label_left(&app, "Appearance settings", "Appearance");
    assert!((appearance - global).abs() <= 1.0);

    support::drag_horizontally(&mut app.harness, wider.right_center(), -180.0);
    let minimum = categories(&app);
    assert!((minimum.width() - 144.0).abs() <= 1.0, "{minimum:?}");
    open_screen(&mut app);
    assert_eq!(scopes(&app).x_range(), minimum.x_range());
    let edge = scopes(&app).right_center();
    support::drag_horizontally(&mut app.harness, edge, 480.0);
    let maximum = scopes(&app);
    assert!((maximum.width() - 320.0).abs() <= 1.0, "{maximum:?}");

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    assert_eq!(app.harness.state().close_state(), CloseState::Ready);
    drop(app.harness);
    app.harness = support::harness_at(app.dir.path(), small);
    open_screen(&mut app);
    assert_eq!(scopes(&app).x_range(), maximum.x_range());
    support::open_screen(&mut app.harness, "Settings");
    settle(&mut app.harness);
    assert_eq!(categories(&app).x_range(), maximum.x_range());
}

#[test]
fn scope_guidance_is_in_the_tooltip_and_inactivity_stays_visible() {
    let mut app = with_scopes();
    let usage = "Used by every request. Saved to globals.yaml in the collections folder.";
    let precedence = "A collection's variables and the active environment override these.";
    let global_guidance = format!("{usage}\n{precedence}");
    support::assert_help(&app.harness, &global_guidance);
    assert!(app.harness.query_by_label(usage).is_none());
    assert!(app.harness.query_by_label(precedence).is_none());
    app.harness.get_by_label("Help").hover();
    app.harness.run_steps(4);
    app.harness.get_by_label(&global_guidance);

    for (scope, usage) in [
        (
            "Environment · Dev",
            "Dev is the active environment: requests use these now.",
        ),
        (
            "Environment · Prod",
            "Prod is not active: requests use these only while it is.",
        ),
    ] {
        click(&mut app.harness, scope);
        settle(&mut app.harness);
        assert!(app.harness.query_by_label(usage).is_none());
        let precedence = "While active, these override the collection's variables and Global.";
        support::assert_help(&app.harness, &format!("{usage}\n{precedence}"));
    }
    app.harness
        .get_by_label("Inactive environment. Activate it to use these values in requests.");

    click(&mut app.harness, "Collection · API");
    settle(&mut app.harness);
    let usage = "Used by the requests in API.";
    let precedence = "The active environment overrides these, and they override Global.";
    support::assert_help(&app.harness, &format!("{usage}\n{precedence}"));
    assert!(app.harness.query_by_label(usage).is_none());
    assert!(app.harness.query_by_label(precedence).is_none());
}

fn with_scopes() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                let dev = environments::create_environment(db, "Dev", &vars(&[]))?;
                environments::set_active_environment(db, Some(dev.id))?;
                let prod = environments::create_environment(db, "Prod", &vars(&[]))?;
                environments::set_environment_color(db, prod.id, Some(EnvColor::Red))
            })
            .unwrap();
        collection(store, "API");
        collection(store, "Webhooks");
    });
    open_screen(&mut app);
    app
}

#[test]
fn clear_all_is_in_the_menu_and_asks_before_removing() {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| environments::set_globals(db, &vars(&[("a", "1"), ("b", "2")])))
            .unwrap();
    });
    open_screen(&mut app);
    assert!(
        app.harness.query_by_label("Clear All").is_none(),
        "the table has no clear action"
    );
    click(&mut app.harness, "Variable actions");
    assert!(
        app.harness.query_by_label("Edit name and color…").is_none(),
        "Global cannot be renamed"
    );
    click(&mut app.harness, "Clear all variables…");
    app.harness
        .get_by_label_contains("Remove all 2 variables from Global?");
    click(&mut app.harness, "Cancel");
    assert_eq!(app.harness.state().state.variables.rows.len(), 2);

    click(&mut app.harness, "Variable actions");
    click(&mut app.harness, "Clear all variables…");
    click_button(&mut app.harness, "Remove all");
    assert_eq!(app.harness.state().state.variables.rows.len(), 0);
    assert_eq!(save_state(&app.harness, "Save variables"), "Unsaved");
    click(&mut app.harness, "Save variables");
    settle(&mut app.harness);
    assert_eq!(
        app.harness
            .state()
            .store()
            .call_blocking(environments::globals)
            .unwrap(),
        vars(&[])
    );
}

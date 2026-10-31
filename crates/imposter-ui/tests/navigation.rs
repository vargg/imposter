//! Getting around: the command palette, through the real UI and database.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};
use imposter_app::core::model::Variables;
use imposter_app::core::settings::AppSettings;
use imposter_app::store::{environments, kv};
use imposter_ui::app::Screen;
use support::{click, collection, request, settle, start_with, type_into};

fn palette(app: &mut support::App, query: &str) {
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
    settle(&mut app.harness);
    support::text_input(&app.harness, "Command palette search").type_text(query);
    app.harness.run_steps(2);
}

fn enter(app: &mut support::App) {
    app.harness.key_press(Key::Enter);
    settle(&mut app.harness);
}

/// Two collections: API (Health, Users/List users) and Admin (Stats), API's Health open.
fn two_collections() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &AppSettings {
                        autosave: false,
                        ..AppSettings::default()
                    },
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Health", "http://h/health");
        let admin = collection(store, "Admin");
        request(store, admin.id, "Stats", "http://h/stats");
        store
            .call_blocking(|db| environments::create_environment(db, "Dev", &Variables::new()))
            .unwrap();
    });
    app.harness.state_mut().state.settings.autosave = false;
    settle(&mut app.harness);
    app
}

fn open_name(app: &support::App) -> Option<String> {
    app.harness
        .state()
        .state
        .editor
        .as_ref()
        .map(|p| p.editor.name().to_owned())
}

fn collection_name(app: &support::App) -> String {
    let state = &app.harness.state().state;
    state
        .collections
        .iter()
        .find(|c| Some(c.id) == state.collection)
        .unwrap()
        .name
        .clone()
}

#[test]
fn the_palette_opens_a_request_from_another_collection() {
    let mut app = two_collections();
    let start = collection_name(&app);
    let (other, wanted) = if start == "API" {
        ("Admin", "stats")
    } else {
        ("API", "health")
    };

    palette(&mut app, wanted);
    enter(&mut app);

    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(collection_name(&app), other);
    assert_eq!(
        open_name(&app).map(|n| n.to_lowercase()),
        Some(wanted.to_owned())
    );
}

#[test]
fn unsaved_edits_are_guarded_before_the_palette_navigates() {
    let mut app = two_collections();
    palette(&mut app, "health");
    enter(&mut app);
    type_into(&mut app.harness, "Request URL", "/edited");

    palette(&mut app, "stats");
    enter(&mut app);

    app.harness
        .get_by_label("Save changes to \"Health\" before continuing?");
    click(&mut app.harness, "Discard");
    settle(&mut app.harness);
    assert_eq!(open_name(&app).as_deref(), Some("Stats"));
}

#[test]
fn the_palette_runs_actions_and_switches_environments() {
    let mut app = two_collections();
    palette(&mut app, "history");
    click(&mut app.harness, "Go to History");
    settle(&mut app.harness);
    assert_eq!(app.harness.state().state.screen, Screen::History);

    palette(&mut app, "dev");
    enter(&mut app);
    let state = &app.harness.state().state;
    let dev = state.environments.iter().find(|e| e.name == "Dev").unwrap();
    assert_eq!(state.active_environment, Some(dev.id));
}

#[test]
fn arrows_choose_and_escape_closes_without_acting() {
    let mut app = two_collections();
    palette(&mut app, "go to");
    app.harness.key_press(Key::ArrowDown);
    app.harness.run_steps(1);
    app.harness.key_press(Key::ArrowUp);
    app.harness.run_steps(1);
    app.harness.key_press(Key::ArrowUp);
    app.harness.run_steps(1);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(app.harness.state().state.screen, Screen::Requests);

    palette(&mut app, "zzzz");
    app.harness.get_by_label("Nothing matches");
    enter(&mut app);
    assert!(
        app.harness.state().state.dialog.is_some(),
        "Enter with no match keeps the palette open"
    );
}

/// One collection with requests A, B and C, opened in that order (C is open).
fn opened_abc() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        for name in ["A", "B", "C"] {
            request(store, api.id, name, &format!("http://h/{name}"));
        }
    });
    for name in ["A", "B", "C"] {
        click(&mut app.harness, &format!("GET {name}"));
        settle(&mut app.harness);
    }
    app
}

fn hold_ctrl(app: &mut support::App) {
    app.harness
        .event(egui::Event::ModifiersChanged(Modifiers::CTRL));
}

fn tab_while_holding(app: &mut support::App) {
    app.harness.event(egui::Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::CTRL,
    });
    settle(&mut app.harness);
}

fn release_ctrl(app: &mut support::App) {
    app.harness
        .event(egui::Event::ModifiersChanged(Modifiers::NONE));
    settle(&mut app.harness);
}

fn recent_names(app: &support::App) -> Vec<String> {
    let Some(imposter_ui::dialogs::Dialog::Palette(palette)) = &app.harness.state().state.dialog
    else {
        panic!("the palette is not open");
    };
    palette.items().into_iter().map(|i| i.label).collect()
}

#[test]
fn recent_requests_come_first_and_survive_a_restart() {
    let mut app = opened_abc();
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut app.harness);
    assert_eq!(recent_names(&app), ["API › C", "API › B", "API › A"]);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);

    app.harness.state_mut().request_close();
    settle(&mut app.harness);
    let dir = app.dir;
    drop(app.harness);
    let mut reopened = support::harness_at(dir.path(), egui::vec2(1400.0, 900.0));
    reopened.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut reopened);
    let mut app = support::App {
        dir,
        harness: reopened,
    };
    assert_eq!(recent_names(&app), ["API › C", "API › B", "API › A"]);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
}

#[test]
fn a_quick_ctrl_tab_goes_back_and_forth() {
    let mut app = opened_abc();
    app.harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    settle(&mut app.harness);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("B"));

    app.harness.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    settle(&mut app.harness);
    assert_eq!(open_name(&app).as_deref(), Some("C"));
}

#[test]
fn holding_ctrl_steps_through_recent_requests_until_it_is_released() {
    let mut app = opened_abc();
    hold_ctrl(&mut app);
    tab_while_holding(&mut app);
    assert!(
        app.harness.state().state.dialog.is_some(),
        "open while held"
    );
    tab_while_holding(&mut app);
    release_ctrl(&mut app);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("A"));

    app.harness
        .key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
    settle(&mut app.harness);
    assert_eq!(
        open_name(&app).as_deref(),
        Some("B"),
        "Shift starts from the oldest"
    );

    hold_ctrl(&mut app);
    tab_while_holding(&mut app);
    app.harness.key_press(Key::Escape);
    settle(&mut app.harness);
    release_ctrl(&mut app);
    assert!(app.harness.state().state.dialog.is_none());
    assert_eq!(open_name(&app).as_deref(), Some("B"), "Esc cancels");
}

#[test]
fn deleted_requests_leave_the_recent_list() {
    let mut app = opened_abc();
    let b = app
        .harness
        .state()
        .state
        .tree
        .as_ref()
        .unwrap()
        .source
        .requests
        .iter()
        .find(|r| r.name == "B")
        .unwrap()
        .id;
    app.harness
        .state()
        .store()
        .call_blocking(move |db| imposter_app::store::collections::delete_request(db, b))
        .unwrap();

    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    settle(&mut app.harness);
    assert_eq!(recent_names(&app), ["API › C", "API › A"]);
    let stored: Vec<imposter_app::core::ids::RequestId> = app
        .harness
        .state()
        .store()
        .call_blocking(|db| kv::get(db, kv::Table::UiState, "recent_requests"))
        .unwrap()
        .unwrap();
    assert!(!stored.contains(&b), "the stored list is pruned too");
}

/// API: Health (GET, root), Users/{List users GET, Create user POST}, Orders/{Place order POST}.
fn foldered() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                use imposter_app::core::model::{BodyType, RequestTemplate};
                use imposter_app::store::collections::{
                    create_collection, create_folder, create_request,
                };
                let api = create_collection(db, "API", "", &Variables::new())?;
                let users = create_folder(db, api.id, None, "Users", "", 1)?;
                let orders = create_folder(db, api.id, None, "Orders", "", 2)?;
                for (folder, name, method, url) in [
                    (None, "Health", "GET", "http://h/health"),
                    (Some(users.id), "List users", "GET", "http://h/users"),
                    (Some(users.id), "Create user", "POST", "http://h/users"),
                    (
                        Some(orders.id),
                        "Place order",
                        "POST",
                        "http://h/store/orders",
                    ),
                ] {
                    let template = RequestTemplate {
                        name: name.into(),
                        method: method.into(),
                        url: url.into(),
                        body_type: BodyType::None,
                        ..RequestTemplate::default()
                    };
                    create_request(db, api.id, folder, &template)?;
                }
                Ok(())
            })
            .unwrap();
    });
    settle(&mut app.harness);
    app
}

fn shown(app: &support::App, label: &str) -> bool {
    app.harness.query_by_label(label).is_some()
}

fn filter(app: &mut support::App, text: &str) {
    app.harness.state_mut().state.sidebar_filter = text.into();
    app.harness.run_steps(2);
}

#[test]
fn the_sidebar_filter_shows_matches_with_their_folders() {
    let mut app = foldered();
    filter(&mut app, "method:POST -create");
    assert!(shown(&app, "POST Place order"));
    assert!(shown(&app, "Folder Orders"));
    assert!(
        !shown(&app, "Folder Users"),
        "a folder left empty is hidden"
    );
    assert!(!shown(&app, "GET Health"));

    filter(&mut app, "\"list users\" OR url:health");
    assert!(shown(&app, "GET List users"));
    assert!(shown(&app, "GET Health"));
    assert!(!shown(&app, "POST Create user"));

    filter(&mut app, "nothing-like-this");
    app.harness.get_by_label("No requests match");
    click(&mut app.harness, "Clear the filter");
    assert_eq!(app.harness.state().state.sidebar_filter, "");
    assert!(shown(&app, "POST Create user"));
}

#[test]
fn clearing_the_filter_keeps_the_folders_the_user_closed() {
    let mut app = foldered();
    let header = app.harness.get_by_label("Folder Users").rect();
    let arrow = egui::pos2(header.left() - 10.0, header.center().y);
    app.harness.drag_at(arrow);
    app.harness.drop_at(arrow);
    app.harness.run_steps(3);
    assert!(!shown(&app, "GET List users"), "the folder is closed");

    filter(&mut app, "list");
    assert!(
        shown(&app, "GET List users"),
        "matches show inside closed folders"
    );
    filter(&mut app, "");
    assert!(
        !shown(&app, "GET List users"),
        "and the folder is closed again"
    );
}

#[test]
fn find_over_the_sidebar_focuses_the_filter_and_escape_clears_it() {
    let mut app = foldered();
    let field = app.harness.get_by_label("Filter requests").rect();
    app.harness
        .hover_at(egui::pos2(field.center().x, field.bottom() + 60.0));
    app.harness.run_steps(1);
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    app.harness.run_steps(2);
    assert!(app.harness.get_by_label("Filter requests").is_focused());

    app.harness.event(egui::Event::Text("health".into()));
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().state.sidebar_filter, "health");
    assert!(!shown(&app, "Folder Users"));
    app.harness.key_press(Key::Escape);
    app.harness.run_steps(2);
    assert_eq!(app.harness.state().state.sidebar_filter, "");
}

/// The "5 of 9 requests" line, if shown.
fn match_count(app: &support::App) -> Option<String> {
    app.harness
        .query_all_by_label_contains(" requests")
        .filter_map(|node| {
            let node = node.accesskit_node();
            node.label().or_else(|| node.value())
        })
        .find(|label| label.starts_with(|c: char| c.is_ascii_digit()))
}

#[test]
fn a_typed_filter_keeps_the_field_and_sidebar_widths_and_counts_matches() {
    let mut app = foldered();
    let before = app.harness.state().state.layout.sidebar_width;
    let field = app.harness.get_by_label("Filter requests").rect();
    assert_eq!(match_count(&app), None, "only while filtering");
    filter(&mut app, "method:POST");
    app.harness.run_steps(20);
    assert_eq!(app.harness.state().state.layout.sidebar_width, before);
    assert_eq!(app.harness.get_by_label("Filter requests").rect(), field);
    let clear = app.harness.get_by_label("Clear the filter").rect();
    assert!(field.contains_rect(clear), "{clear:?} inside {field:?}");
    let count = match_count(&app).unwrap();
    let numbers: Vec<usize> = count
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect();
    assert!(
        matches!(numbers.as_slice(), [shown, total] if 0 < *shown && shown < total),
        "{count:?}"
    );
}

/// Request "Old" open, with autosave off so the paste stays unsaved.
fn old_request() -> support::App {
    let mut app = start_with(|store| {
        store
            .call_blocking(|db| {
                kv::set_app_settings(
                    db,
                    &AppSettings {
                        autosave: false,
                        ..AppSettings::default()
                    },
                )
            })
            .unwrap();
        let api = collection(store, "API");
        request(store, api.id, "Old", "http://h/old");
    });
    app.harness.state_mut().state.settings.autosave = false;
    click(&mut app.harness, "GET Old");
    settle(&mut app.harness);
    app
}

fn paste(app: &mut support::App, text: &str) {
    app.harness.event(egui::Event::Paste(text.into()));
    settle(&mut app.harness);
}

fn editor(app: &support::App) -> &imposter_ui::editor::Editor {
    &app.harness.state().state.editor.as_ref().unwrap().editor
}

const CURL: &str = r#"curl -X POST 'https://api.test/users?page=2' -H 'X-Trace: 1' -d '{"a":1}'"#;

#[test]
fn a_curl_command_pasted_over_the_whole_url_replaces_the_request() {
    let mut app = old_request();
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::L);
    app.harness.run_steps(2);
    paste(&mut app, CURL);

    let edited = editor(&app);
    assert_eq!(edited.name(), "Old", "the request keeps its name");
    assert_eq!(edited.method, "POST");
    assert_eq!(edited.url, "https://api.test/users?page=2");
    assert!(edited.headers.iter().any(|h| h.name == "X-Trace"));
    assert_eq!(edited.body_text, r#"{"a":1}"#);
    app.harness.get_by_label("Unsaved");
    app.harness
        .get_by_label("Replaced from curl — not saved yet");
}

#[test]
fn other_pastes_into_the_url_are_ordinary_text() {
    let mut app = old_request();
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::L);
    app.harness.run_steps(2);
    app.harness.key_press(Key::End);
    app.harness.run_steps(1);
    paste(&mut app, "/more");
    assert_eq!(editor(&app).url, "http://h/old/more");

    app.harness.key_press(Key::End);
    app.harness.run_steps(1);
    paste(&mut app, " curl x");
    assert_eq!(
        editor(&app).url,
        "http://h/old/more curl x",
        "a command pasted into part of the URL is inserted as text"
    );
    assert_eq!(editor(&app).method, "GET");

    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::L);
    app.harness.run_steps(2);
    paste(&mut app, "http://h/replaced");
    assert_eq!(editor(&app).url, "http://h/replaced");
}

#[test]
fn ignored_curl_options_are_listed_after_the_replacement() {
    let mut app = old_request();
    app.harness.key_press_modifiers(Modifiers::COMMAND, Key::L);
    app.harness.run_steps(2);
    paste(&mut app, "curl --limit-rate 1k https://api.test/slow");

    assert_eq!(editor(&app).url, "https://api.test/slow");
    app.harness
        .get_by_label("Some curl options were not imported");
    app.harness
        .get_by_label_contains("is not supported and was ignored");
}

#[test]
fn a_word_in_a_request_name_outranks_scattered_letters_in_an_action() {
    let mut app = start_with(|store| {
        let shop = collection(store, "Shop API");
        request(store, shop.id, "Order {{user_id}}", "http://h/orders/1");
    });
    settle(&mut app.harness);
    palette(&mut app, "ord");
    let first = recent_names(&app).into_iter().next();
    assert_eq!(
        first.as_deref(),
        Some("Shop API › Order {{user_id}}"),
        "not Keyboard Shortcuts"
    );
}

//! Editor widgets through the UI: auth, rows, multipart, binary bodies and custom methods.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{ApiKeyIn, AuthKind, BodyType, FormKind, RequestAuth};
use imposter_app::store::collections;
use imposter_ui::app::EditorTab;
use support::{click, collection, request, settle, start_with, type_into};

fn opened() -> support::App {
    let mut app = start_with(|store| {
        let api = collection(store, "API");
        request(store, api.id, "Widgets", "http://x");
    });
    click(&mut app.harness, "GET Widgets");
    settle(&mut app.harness);
    app
}

fn tab(app: &mut support::App, tab: EditorTab) {
    app.harness.state_mut().state.editor.as_mut().unwrap().tab = tab;
    app.harness.run_steps(2);
}

fn saved(app: &mut support::App) -> imposter_app::core::model::Request {
    for _ in 0..4 {
        app.harness.step();
    }
    settle(&mut app.harness);
    let id = app
        .harness
        .state()
        .state
        .editor
        .as_ref()
        .unwrap()
        .editor
        .id()
        .unwrap();
    app.harness
        .state()
        .store()
        .call_blocking(move |db| collections::request(db, id))
        .unwrap()
        .unwrap()
}

#[test]
fn basic_auth_fields_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .get_by_label("No authentication is configured for this request.");

    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .auth
        .kind = Some(AuthKind::Basic);
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Username", "ann");
    type_into(&mut app.harness, "Password", "secret");

    assert_eq!(
        saved(&mut app).template.auth,
        RequestAuth::basic("ann", "secret")
    );
}

#[test]
fn bearer_tokens_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
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
    type_into(&mut app.harness, "Token", "t0k");

    assert_eq!(saved(&mut app).template.auth, RequestAuth::bearer("t0k"));
}

#[test]
fn api_keys_are_chosen_placed_and_saved() {
    use egui::accesskit::Role;
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Type")
        .click();
    app.harness.run_steps(3);
    app.harness.get_by_label("API Key").click();
    app.harness.run_steps(2);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Add to")
        .click();
    app.harness.run_steps(3);
    // "Query" is also a tab: the option is the menu's, drawn last.
    app.harness
        .get_all_by_label("Query")
        .next_back()
        .unwrap()
        .click();
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Key name", "api_key");
    type_into(&mut app.harness, "Value", "{{key}}");
    app.harness.get_by_label("Show value");

    assert_eq!(
        saved(&mut app).template.auth,
        RequestAuth::api_key("api_key", "{{key}}", ApiKeyIn::Query)
    );
}

#[test]
fn oauth2_client_credentials_are_entered_and_saved() {
    use egui::accesskit::Role;
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Type")
        .click();
    app.harness.run_steps(3);
    app.harness.get_by_label("OAuth 2.0").click();
    app.harness.run_steps(2);
    // Client credentials is the grant until another is picked.
    app.harness.get_by_role_and_label(Role::ComboBox, "Grant");
    type_into(&mut app.harness, "Token URL", "{{base}}/token");
    type_into(&mut app.harness, "Client ID", "app");
    type_into(&mut app.harness, "Client secret", "{{secret:client}}");
    app.harness.get_by_label("Show client secret");
    type_into(&mut app.harness, "Scope", "read write");
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Client auth")
        .click();
    app.harness.run_steps(3);
    // "Body" is also a tab: the option is the menu's, drawn last.
    app.harness
        .get_all_by_label("Body")
        .next_back()
        .unwrap()
        .click();
    app.harness.run_steps(2);

    assert_eq!(
        saved(&mut app).template.auth,
        RequestAuth::oauth2(imposter_app::core::model::OAuth2 {
            token_url: "{{base}}/token".into(),
            client_id: "app".into(),
            client_secret: "{{secret:client}}".into(),
            scope: "read write".into(),
            client_auth: imposter_app::core::model::ClientAuth::Body,
            ..Default::default()
        })
    );
}

#[test]
fn the_authorization_code_grant_asks_where_to_sign_in() {
    use egui::accesskit::Role;
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Type")
        .click();
    app.harness.run_steps(3);
    app.harness.get_by_label("OAuth 2.0").click();
    app.harness.run_steps(2);
    assert!(app.harness.query_by_label("Auth URL").is_none());
    app.harness
        .get_by_role_and_label(Role::ComboBox, "Grant")
        .click();
    app.harness.run_steps(3);
    app.harness.get_by_label("Authorization code").click();
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Auth URL", "https://auth.test/authorize");
    type_into(&mut app.harness, "Token URL", "https://auth.test/token");
    app.harness
        .get_by_role_and_label(Role::SpinButton, "Redirect port");

    let auth = saved(&mut app).template.auth;
    let oauth = auth.oauth2.unwrap();
    assert_eq!(
        oauth.grant,
        imposter_app::core::model::OAuthGrant::AuthorizationCode
    );
    assert_eq!(oauth.auth_url, "https://auth.test/authorize");
    assert_eq!(oauth.redirect_port, None, "any free port until one is set");
}

#[test]
fn secrets_are_masked_until_shown_and_fields_share_a_column() {
    use egui::accesskit::Role;
    let mut app = opened();
    tab(&mut app, EditorTab::Auth);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .auth
        .kind = Some(AuthKind::Basic);
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Password", "secret");
    app.harness
        .get_by_role_and_label(Role::PasswordInput, "Password");

    click(&mut app.harness, "Show password");
    app.harness
        .get_by_role_and_label(Role::TextInput, "Password");
    click(&mut app.harness, "Hide password");
    app.harness
        .get_by_role_and_label(Role::PasswordInput, "Password");

    // Username and password start in one column and reach the eye's edge.
    let user = support::text_input(&app.harness, "Username").rect();
    let password = support::text_input(&app.harness, "Password").rect();
    let eye = app.harness.get_by_label("Show password").rect();
    assert!(
        (user.left() - password.left()).abs() < 0.5,
        "{user:?} {password:?}"
    );
    assert!(eye.left() > password.right(), "{password:?} {eye:?}");
    assert!(
        (user.right() - eye.right()).abs() < 2.0,
        "{user:?} ends with {eye:?}"
    );
}

#[test]
fn header_rows_can_be_added_edited_disabled_and_cleared() {
    let mut app = opened();
    tab(&mut app, EditorTab::Headers);

    click(&mut app.harness, "Add header");
    type_into(&mut app.harness, "Headers row 1 name", "X-Trace");
    type_into(&mut app.harness, "Headers row 1 value", "on");
    click(&mut app.harness, "Add header");
    type_into(&mut app.harness, "Headers row 2 name", "X-Off");
    click(&mut app.harness, "Headers row 2 enabled");
    let headers = saved(&mut app).template.headers;
    assert_eq!(headers.len(), 2);
    assert_eq!(
        (
            headers[0].name.as_str(),
            headers[0].value.as_str(),
            headers[0].is_enabled
        ),
        ("X-Trace", "on", true)
    );
    assert!(!headers[1].is_enabled);

    // No saving between the clicks: that takes frames, and the button disarms with time.
    let rows = |app: &support::App| {
        let editor = &app.harness.state().state.editor.as_ref().unwrap().editor;
        editor.headers.len()
    };
    click(&mut app.harness, "Remove Headers row 2");
    assert_eq!(rows(&app), 2, "one click only arms");
    click(&mut app.harness, "Click again to remove Headers row 2");
    assert_eq!(rows(&app), 1);
    assert_eq!(saved(&mut app).template.headers.len(), 1);

    click(&mut app.harness, "Clear all");
    app.harness.get_by_label("Remove all rows?");
    click(&mut app.harness, "Remove all");
    assert_eq!(saved(&mut app).template.headers, []);
}

#[test]
fn query_rows_rewrite_the_url() {
    let mut app = opened();
    tab(&mut app, EditorTab::Params);
    click(&mut app.harness, "Add query");
    type_into(&mut app.harness, "Query row 1 name", "q");
    type_into(&mut app.harness, "Query row 1 value", "a b");

    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .editor
            .url,
        "http://x?q=a+b"
    );
    assert_eq!(saved(&mut app).template.url, "http://x?q=a+b");
}

#[test]
fn multipart_fields_and_binary_paths_are_saved() {
    let mut app = opened();
    tab(&mut app, EditorTab::Body);
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Multipart;
    app.harness.run_steps(2);
    click(&mut app.harness, "Add field");
    type_into(&mut app.harness, "Multipart row 1 name", "upload");
    type_into(&mut app.harness, "Multipart row 1 value", "/tmp/file.bin");
    type_into(
        &mut app.harness,
        "Multipart row 1 content type",
        "application/octet-stream",
    );
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .multipart[0]
        .kind = FormKind::File;
    app.harness
        .state_mut()
        .state
        .actions
        .push(imposter_ui::app::Action::Edited {
            url: false,
            params: false,
        });
    let fields = saved(&mut app).template.multipart_fields().unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].name.as_str(), fields[0].kind),
        ("upload", FormKind::File)
    );
    assert_eq!(fields[0].content_type, "application/octet-stream");

    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .editor
        .body_type = BodyType::Binary;
    app.harness.run_steps(2);
    type_into(&mut app.harness, "Binary file path", "/data/blob.bin");
    let request = saved(&mut app);
    assert_eq!(
        (request.template.body_type, request.template.body.as_slice()),
        (BodyType::Binary, b"/data/blob.bin".as_slice())
    );
}

#[test]
fn a_custom_method_keeps_its_case() {
    let mut app = opened();
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "Custom method…");
    assert_eq!(
        app.harness
            .state()
            .state
            .editor
            .as_ref()
            .unwrap()
            .custom_method
            .as_deref(),
        Some("GET"),
        "choosing a custom method opens its editor"
    );
    type_into(&mut app.harness, "Custom HTTP method", "-search");
    app.harness.get_by_label("Use");
    assert!(
        app.harness.query_by_label("HTTP method").is_none(),
        "the field takes the picker's place"
    );
    type_into(&mut app.harness, "Custom HTTP method", " bad");
    app.harness.get_by_label("No spaces allowed");
    app.harness
        .state_mut()
        .state
        .editor
        .as_mut()
        .unwrap()
        .custom_method = Some("PropFind".into());
    app.harness.run_steps(2);
    click(&mut app.harness, "Use");

    assert_eq!(saved(&mut app).template.method, "PropFind");
}

#[test]
fn standard_methods_are_picked_from_the_list() {
    let mut app = opened();
    click(&mut app.harness, "HTTP method");
    click(&mut app.harness, "DELETE");

    assert_eq!(saved(&mut app).template.method, "DELETE");
    app.harness.get_by_label("DELETE Widgets");
}

mod controls_kit {
    use egui::{
        Color32, Rect, Shape,
        accesskit::{Role, Toggled},
    };
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use imposter_ui::{
        icons::Icon,
        theme::{self, Density, ThemeChoice},
        widgets::{self, Variant},
    };

    const VARIANTS: [(Variant, &str); 4] = [
        (Variant::Primary, "Create request"),
        (Variant::Secondary, "Save draft"),
        (Variant::Ghost, "Preview"),
        (Variant::Danger, "Delete request"),
    ];

    fn harness(choice: ThemeChoice, density: Density) -> Harness<'static, (bool, u8, usize)> {
        Harness::builder()
            .with_context_setup(move |ctx| {
                ctx.set_fonts(theme::fonts());
                theme::apply(ctx, choice, density);
            })
            .build_ui_state(
                |ui, state| {
                    for (variant, label) in VARIANTS {
                        if widgets::button(ui, variant, Some(Icon::Plus), label).clicked() {
                            state.2 += 1;
                        }
                        ui.add_enabled_ui(false, |ui| {
                            widgets::button(ui, variant, None, &format!("Disabled {label}"));
                        });
                    }
                    if widgets::checkbox(ui, &mut state.0, "Enabled").changed() {
                        state.2 += 1;
                    }
                    if widgets::segmented(ui, "Mode", &mut state.1, &[(0, "First"), (1, "Second")])
                        .changed()
                    {
                        state.2 += 1;
                    }
                    widgets::label_with_help(ui, "Timeout", "Maximum time to wait for a response.");
                },
                (false, 0, 0),
            )
    }

    fn rect_shape<'a, T>(harness: &'a Harness<'_, T>, rect: Rect) -> &'a egui::epaint::RectShape {
        harness
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Rect(shape) if shape.rect == rect => Some(shape),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing painted rectangle {rect:?}"))
    }

    fn text_shape<'a, T>(harness: &'a Harness<'_, T>, label: &str) -> &'a egui::epaint::TextShape {
        harness
            .output()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(shape) if shape.galley.job.text == label => Some(shape),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing painted text {label}"))
    }

    #[test]
    fn buttons_share_geometry_and_keep_their_hierarchy_in_both_themes_and_densities() {
        for choice in [ThemeChoice::Dark, ThemeChoice::Light] {
            for density in [Density::Compact, Density::Comfortable] {
                let mut harness = harness(choice, density);
                let p = theme::palette();
                for (variant, label) in VARIANTS {
                    let rect = harness.get_by_role_and_label(Role::Button, label).rect();
                    assert_eq!(rect.height(), density.spacing().control);
                    let shape = rect_shape(&harness, rect);
                    let (fill, text, edge) = match variant {
                        Variant::Primary => (p.accent, p.on_accent, p.accent),
                        Variant::Secondary => (p.input, p.text, p.field_border),
                        Variant::Ghost => {
                            (Color32::TRANSPARENT, p.text_muted, Color32::TRANSPARENT)
                        }
                        Variant::Danger => (p.input, p.error, p.field_border),
                    };
                    assert_eq!(shape.fill, fill, "{label}");
                    assert_eq!(shape.stroke.color, edge, "{label}");
                    assert_eq!(shape.corner_radius, egui::CornerRadius::same(6));
                    let label_shape = text_shape(&harness, label);
                    assert_eq!(label_shape.fallback_color, text);
                    assert!(
                        (label_shape.pos.x - rect.left() - 32.0).abs() < 0.1,
                        "10 padding + 16 icon + 6 gap"
                    );
                    assert!(
                        (rect.right() - label_shape.pos.x - label_shape.galley.size().x - 10.0)
                            .abs()
                            < 0.1
                    );
                    let icon = harness
                        .output()
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            Shape::Text(text)
                                if text.galley.job.text == Icon::Plus.glyph().to_string()
                                    && rect.contains(text.pos) =>
                            {
                                Some(text)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(
                        icon.galley.job.sections[0].format.font_id.family,
                        egui::FontFamily::Name(theme::ICONS.into())
                    );
                    assert!(
                        (icon.pos.x + icon.galley.size().x / 2.0 - rect.left() - 18.0).abs() < 0.1
                    );
                    let disabled_label = format!("Disabled {label}");
                    let disabled = harness.get_by_role_and_label(Role::Button, &disabled_label);
                    assert!(disabled.accesskit_node().is_disabled());
                    let shape = rect_shape(&harness, disabled.rect());
                    let alpha = harness
                        .ctx
                        .style_of(harness.ctx.theme())
                        .visuals
                        .disabled_alpha();
                    assert_eq!(shape.fill, p.input.gamma_multiply(alpha));
                    assert_eq!(
                        text_shape(&harness, &format!("Disabled {label}")).fallback_color,
                        p.text.gamma_multiply(alpha)
                    );
                }
                harness.get_by_label("Preview").hover();
                harness.run_steps(2);
                let rect = harness.get_by_label("Preview").rect();
                assert_eq!(rect_shape(&harness, rect).stroke.color, p.text_muted);
                assert_eq!(rect_shape(&harness, rect).fill, Color32::TRANSPARENT);
                harness.get_by_label("Create request").click();
                harness.run_steps(2);
                assert_eq!(harness.state().2, 1);
            }
        }
    }

    #[test]
    fn checkbox_and_segments_report_changes_and_accessible_state() {
        for choice in [ThemeChoice::Dark, ThemeChoice::Light] {
            for density in [Density::Compact, Density::Comfortable] {
                let mut harness = harness(choice, density);
                let p = theme::palette();
                let checked = |h: &Harness<'_, (bool, u8, usize)>, role, label| {
                    h.get_by_role_and_label(role, label)
                        .accesskit_node()
                        .toggled()
                };
                assert_eq!(
                    checked(&harness, Role::CheckBox, "Enabled"),
                    Some(Toggled::False)
                );
                let rect = harness.get_by_label("Enabled").rect();
                assert_eq!(rect.height(), density.spacing().control);
                let square = Rect::from_center_size(
                    egui::pos2(rect.left() + 12.0, rect.center().y),
                    egui::Vec2::splat(14.0),
                );
                assert_eq!(rect_shape(&harness, square).fill, p.input);
                assert_eq!(rect_shape(&harness, square).stroke.color, p.field_border);
                assert_eq!(
                    rect_shape(&harness, square).corner_radius,
                    egui::CornerRadius::same(4)
                );
                harness.get_by_label("Enabled").click();
                harness.run_steps(2);
                assert!(harness.state().0);
                assert_eq!(harness.state().2, 1);
                assert_eq!(
                    checked(&harness, Role::CheckBox, "Enabled"),
                    Some(Toggled::True)
                );
                assert_eq!(rect_shape(&harness, square).fill, p.accent);
                let glyph = text_shape(&harness, &Icon::Check.glyph().to_string());
                assert_eq!(glyph.fallback_color, p.on_accent);
                assert_eq!(
                    glyph.galley.job.sections[0].format.font_id.family,
                    egui::FontFamily::Name(theme::ICONS.into())
                );
                assert_eq!(
                    checked(&harness, Role::Button, "First"),
                    Some(Toggled::True)
                );
                assert_eq!(
                    checked(&harness, Role::Button, "Second"),
                    Some(Toggled::False)
                );
                let first = harness.get_by_label("First").rect();
                let second = harness.get_by_label("Second").rect();
                assert_eq!(first.height(), density.spacing().control);
                assert_eq!(first.width(), second.width());
                assert_eq!(first.right(), second.left());
                assert_eq!(rect_shape(&harness, first.union(second)).fill, p.input);
                assert_eq!(
                    rect_shape(&harness, first.union(second)).stroke.color,
                    p.field_border
                );
                harness.get_by_label("Second").click();
                harness.step();
                assert_eq!(harness.state().1, 1);
                assert_eq!(harness.state().2, 2);
                assert_eq!(
                    checked(&harness, Role::Button, "First"),
                    Some(Toggled::False)
                );
                assert_eq!(
                    checked(&harness, Role::Button, "Second"),
                    Some(Toggled::True)
                );
                assert_eq!(
                    rect_shape(&harness, first.shrink(1.0)).fill,
                    Color32::TRANSPARENT
                );
                assert_eq!(rect_shape(&harness, second.shrink(1.0)).fill, p.selected);
                assert_eq!(text_shape(&harness, "Second").fallback_color, p.accent);
                assert_eq!(
                    text_shape(&harness, "Second").galley.job.sections[0]
                        .format
                        .font_id,
                    theme::semibold(theme::BODY)
                );
                assert_eq!(
                    text_shape(&harness, "First").galley.job.sections[0]
                        .format
                        .font_id,
                    egui::FontId::proportional(theme::BODY)
                );
                harness.get_by_label("Second").click();
                harness.run_steps(2);
                assert_eq!(
                    harness.state().2,
                    2,
                    "clicking the selected segment is not a change"
                );
                harness.get_by_label("Enabled").click();
                harness.run_steps(2);
                assert!(!harness.state().0);
            }
        }
    }

    #[test]
    fn help_exposes_its_explanation_and_draws_a_small_unframed_info_glyph() {
        let mut harness = harness(ThemeChoice::Dark, Density::Compact);
        harness.get_by_role_and_label(Role::Label, "Help");
        for label in ["Help", "Timeout"] {
            assert_eq!(
                harness
                    .get_by_label(label)
                    .accesskit_node()
                    .description()
                    .as_deref(),
                Some("Maximum time to wait for a response.")
            );
        }
        let text = text_shape(&harness, &Icon::Info.glyph().to_string());
        assert_eq!(text.galley.job.sections[0].format.font_id.size, 13.0);
        assert_eq!(
            text.galley.job.sections[0].format.font_id.family,
            egui::FontFamily::Name(theme::ICONS.into())
        );
        assert_eq!(text.fallback_color, theme::palette().text_muted);
        let rect = harness.get_by_label("Help").rect();
        assert!(
            !harness
                .output()
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, Shape::Rect(s) if s.rect == rect))
        );
        assert!(
            harness
                .query_by_label("Maximum time to wait for a response.")
                .is_none()
        );
        harness.get_by_label("Help").hover();
        harness.run_steps(4);
        harness.get_by_label("Maximum time to wait for a response.");
        harness.get_by_label("Second").hover();
        harness.run_steps(4);
        assert!(
            harness
                .query_by_label("Maximum time to wait for a response.")
                .is_none()
        );
        harness.get_by_label("Help").focus();
        harness.run_steps(4);
        harness.get_by_label("Maximum time to wait for a response.");
    }

    #[test]
    fn dropdowns_and_collapsing_headers_paint_phosphor_carets() {
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| {
                ctx.set_fonts(theme::fonts());
                theme::apply(ctx, ThemeChoice::Light, Density::Compact);
            })
            .build_ui(|ui| {
                let combo = widgets::combo("picker")
                    .selected_text("Choose")
                    .show_ui(ui, |ui| {
                        ui.label("Option");
                    });
                widgets::name(&combo.response, "Picker");
                egui::CollapsingHeader::new("Details")
                    .icon(widgets::collapsing_icon)
                    .show(ui, |ui| {
                        ui.label("Contents");
                    });
            });
        let picker = harness
            .get_by_role_and_label(Role::ComboBox, "Picker")
            .rect();
        let caret = text_shape(&harness, &Icon::ChevronDown.glyph().to_string());
        assert!(caret.pos.x > picker.center().x);
        for icon in [Icon::ChevronDown, Icon::ChevronRight] {
            let glyph = text_shape(&harness, &icon.glyph().to_string());
            assert_eq!(
                glyph.galley.job.sections[0].format.font_id.family,
                egui::FontFamily::Name(theme::ICONS.into())
            );
            assert_eq!(glyph.fallback_color, theme::palette().text_muted);
        }
        harness.get_by_label("Details").click();
        harness.run_steps(4);
        harness.get_by_label("Contents");
        assert!(!harness.output().shapes.iter().any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.job.text == Icon::ChevronRight.glyph().to_string())));
        harness.get_by_label("Picker").click();
        harness.run_steps(2);
        harness.get_by_label("Option");
    }

    #[test]
    fn type_helpers_use_the_shared_scale_and_weights() {
        let harness = Harness::builder()
            .with_context_setup(|ctx| {
                ctx.set_fonts(theme::fonts());
            })
            .build_ui(|ui| {
                ui.label(theme::page_title("Page"));
                ui.label(theme::section("Section"));
                ui.label(theme::body("Body"));
                ui.label(theme::meta("Meta"));
            });
        for (label, font) in [
            ("Page", theme::semibold(18.0)),
            ("Section", theme::semibold(14.0)),
            ("Body", egui::FontId::proportional(13.0)),
            ("Meta", egui::FontId::proportional(12.0)),
        ] {
            assert_eq!(
                text_shape(&harness, label).galley.job.sections[0]
                    .format
                    .font_id,
                font
            );
        }
    }

    #[test]
    fn controls_keep_keyboard_activation_and_focus_marks() {
        let mut harness = harness(ThemeChoice::Dark, Density::Compact);
        harness.get_by_label("Enabled").focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::Space);
        harness.run_steps(2);
        assert!(harness.state().0);
        harness.get_by_label("Second").focus();
        harness.run_steps(2);
        let rect = harness.get_by_label("Second").rect();
        assert_eq!(
            rect_shape(&harness, rect.shrink(1.0)).stroke,
            egui::Stroke::new(1.5, theme::palette().accent)
        );
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().1, 1);
        harness.get_by_label("Create request").focus();
        harness.run_steps(2);
        let rect = harness.get_by_label("Create request").rect();
        assert_eq!(
            rect_shape(&harness, rect).stroke,
            egui::Stroke::new(1.5, theme::palette().on_accent)
        );
        harness.key_press(egui::Key::Space);
        harness.run_steps(2);
        assert_eq!(harness.state().2, 3);
    }

    #[test]
    fn sidebar_folder_toggles_use_the_shared_caret_painter() {
        let mut app = super::start_with(|store| {
            let api = super::collection(store, "API");
            store
                .call_blocking(move |db| {
                    super::collections::create_folder(db, api.id, None, "Group", "", 0)
                })
                .unwrap();
        });
        let folder_toggle = app
            .harness
            .get_by_role_and_label(Role::Button, "Toggle folder Group");
        let toggle = folder_toggle.rect();
        assert_eq!(
            folder_toggle.accesskit_node().data().is_expanded(),
            Some(true)
        );
        let has_caret = |app: &super::support::App, icon: Icon| {
            app.harness.output().shapes.iter().any(|s| matches!(&s.shape,
                Shape::Text(t) if t.galley.job.text == icon.glyph().to_string()
                    && toggle.contains(t.pos + t.galley.size() / 2.0)
                    && t.galley.job.sections[0].format.font_id.family == egui::FontFamily::Name(theme::ICONS.into())))
        };
        assert!(has_caret(&app, Icon::ChevronDown));
        app.harness.get_by_label("Toggle folder Group").click();
        app.harness.run_steps(4);
        assert!(has_caret(&app, Icon::ChevronRight));
        assert_eq!(
            app.harness
                .get_by_label("Toggle folder Group")
                .accesskit_node()
                .data()
                .is_expanded(),
            Some(false)
        );
        app.harness.get_by_label("Toggle folder Group").click();
        app.harness.run_steps(4);
        assert!(has_caret(&app, Icon::ChevronDown));
    }

    #[test]
    fn screens_cannot_reintroduce_stock_picker_or_checkbox_marks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains(".checkbox("),
                "{} bypasses the checkbox helper",
                path.display()
            );
            assert!(
                !source.contains("egui::Checkbox::"),
                "{} uses a stock checkbox",
                path.display()
            );
            let constructors = source.matches("egui::ComboBox::").count();
            assert_eq!(
                constructors,
                usize::from(path.ends_with("widgets.rs")),
                "{} bypasses the combo helper",
                path.display()
            );
        }
    }

    #[test]
    fn narrow_controls_truncate_inside_their_targets_and_keep_full_names() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(180.0, 160.0))
            .with_context_setup(|ctx| {
                ctx.set_fonts(theme::fonts());
                theme::apply(ctx, ThemeChoice::Dark, Density::Compact);
            })
            .build_ui_state(
                |ui, value| {
                    widgets::button(
                        ui,
                        Variant::Secondary,
                        None,
                        "A long action that cannot fit",
                    );
                    ui.horizontal(|ui| {
                        widgets::segmented(
                            ui,
                            "narrow",
                            value,
                            &[(0, "First long choice"), (1, "Second long choice")],
                        );
                    });
                },
                0,
            );
        for label in [
            "A long action that cannot fit",
            "First long choice",
            "Second long choice",
        ] {
            let rect = harness.get_by_role_and_label(Role::Button, label).rect();
            assert!(harness.ctx.content_rect().contains_rect(rect));
            let text = text_shape(&harness, label);
            assert!(text.galley.elided);
            assert!(rect.contains_rect(text.visual_bounding_rect()));
        }
        harness.get_by_label("Second long choice").hover();
        harness.run_steps(4);
        harness.get_by_role_and_label(Role::Label, "Second long choice");
    }
}

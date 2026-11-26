//! The Settings screen: application defaults, edited against a saved baseline.

use egui::{Align, Frame, Response, Ui};
use imposter_app::core::settings::{AppSettings, HostIdentity};

use crate::app::{Action, State};
use crate::icons::{self, Icon};
use crate::theme::{self, Tone};
use crate::widgets::{
    self, ListEnd, ListIcon, SaveStatus, SaveWord, Variant, settings_heading, settings_row,
    tone_text,
};
use crate::workspace;
use imposter_app::store::executions::Usage;

const MIB: u64 = 1024 * 1024;

/// The editable fields, in the units the screen shows.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsForm {
    /// Whole seconds, 1–300.
    pub timeout: u32,
    pub autosave: bool,
    pub verify_ssl: bool,
    pub follow_redirects: bool,
    pub cookie_jar: bool,
    pub proxy_url: String,
    /// KiB, 0 for unlimited.
    pub download_limit: u64,
    pub ca_bundle: String,
    pub certificate_path: String,
    pub key_file: String,
    pub password: String,
    pub host_identities: Vec<HostIdentity>,
    pub history_max_entries: u64,
    pub history_mib: u64,
    pub allow_production_from_cli: bool,
}

impl SettingsForm {
    pub fn new(settings: &AppSettings) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped first
        let timeout = settings.timeout.round().clamp(1.0, 300.0) as u32;
        Self {
            timeout,
            autosave: settings.autosave,
            verify_ssl: settings.verify_ssl,
            follow_redirects: settings.follow_redirects,
            cookie_jar: settings.cookie_jar,
            proxy_url: settings.proxy_url.clone(),
            download_limit: settings.download_limit.min(100 * 1024),
            ca_bundle: settings.ca_bundle.clone(),
            certificate_path: settings.certificate_path.clone(),
            key_file: settings.key_file.clone(),
            password: settings.password.clone(),
            host_identities: settings.host_identities.clone(),
            history_max_entries: settings.history_max_entries.clamp(1, 1_000_000),
            history_mib: (settings.history_max_bytes / MIB).clamp(1, 102_400),
            allow_production_from_cli: settings.allow_production_from_cli,
        }
    }

    /// `base` with the form's fields; settings the screen does not show are kept.
    pub fn apply(&self, base: &AppSettings) -> AppSettings {
        AppSettings {
            timeout: f64::from(self.timeout),
            autosave: self.autosave,
            verify_ssl: self.verify_ssl,
            follow_redirects: self.follow_redirects,
            cookie_jar: self.cookie_jar,
            proxy_url: self.proxy_url.clone(),
            download_limit: self.download_limit,
            ca_bundle: self.ca_bundle.clone(),
            certificate_path: self.certificate_path.clone(),
            key_file: self.key_file.clone(),
            password: self.password.clone(),
            host_identities: self
                .host_identities
                .iter()
                .cloned()
                .map(|mut identity| {
                    identity.name = identity.name.trim().to_owned();
                    identity
                })
                .collect(),
            history_max_entries: self.history_max_entries,
            history_max_bytes: self.history_mib * MIB,
            allow_production_from_cli: self.allow_production_from_cli,
            ..base.clone()
        }
    }
}

/// A path field of the form, for its Browse… button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathField {
    CaBundle,
    Certificate,
    Key,
    HostCertificate(usize),
    HostKey(usize),
}

impl PathField {
    /// The field in `form`; `None` when its host certificate was removed meanwhile.
    pub fn value(self, form: &mut SettingsForm) -> Option<&mut String> {
        match self {
            Self::CaBundle => Some(&mut form.ca_bundle),
            Self::Certificate => Some(&mut form.certificate_path),
            Self::Key => Some(&mut form.key_file),
            Self::HostCertificate(index) => form
                .host_identities
                .get_mut(index)
                .map(|identity| &mut identity.certificate_path),
            Self::HostKey(index) => form
                .host_identities
                .get_mut(index)
                .map(|identity| &mut identity.key_file),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Category {
    #[default]
    Appearance,
    Collections,
    General,
    Network,
    Tls,
    History,
    CommandLine,
}

impl Category {
    const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Collections,
        Self::General,
        Self::Network,
        Self::Tls,
        Self::History,
        Self::CommandLine,
    ];

    const fn item(self) -> (&'static str, Icon) {
        match self {
            Self::Appearance => ("Appearance", Icon::Appearance),
            Self::Collections => ("Collections", Icon::Folder),
            Self::General => ("General", Icon::Settings),
            Self::Network => ("Network", Icon::Globe),
            Self::Tls => ("TLS", Icon::Lock),
            Self::History => ("History", Icon::History),
            Self::CommandLine => ("Command line", Icon::Terminal),
        }
    }

    pub(crate) fn changed(self, form: &SettingsForm, baseline: &SettingsForm) -> bool {
        match self {
            Self::Appearance | Self::Collections => false,
            Self::General => form.autosave != baseline.autosave,
            Self::Network => {
                (
                    form.timeout,
                    &form.proxy_url,
                    form.download_limit,
                    form.follow_redirects,
                    form.cookie_jar,
                ) != (
                    baseline.timeout,
                    &baseline.proxy_url,
                    baseline.download_limit,
                    baseline.follow_redirects,
                    baseline.cookie_jar,
                )
            }
            Self::Tls => {
                (
                    form.verify_ssl,
                    &form.ca_bundle,
                    &form.certificate_path,
                    &form.key_file,
                    &form.password,
                    &form.host_identities,
                ) != (
                    baseline.verify_ssl,
                    &baseline.ca_bundle,
                    &baseline.certificate_path,
                    &baseline.key_file,
                    &baseline.password,
                    &baseline.host_identities,
                )
            }
            Self::History => {
                (form.history_max_entries, form.history_mib)
                    != (baseline.history_max_entries, baseline.history_mib)
            }
            Self::CommandLine => {
                form.allow_production_from_cli != baseline.allow_production_from_cli
            }
        }
    }
}

fn row(ui: &mut Ui, label: &str, help: &str, field: impl FnOnce(&mut Ui) -> Response) -> bool {
    named_row(ui, label, help, label, field)
}

fn named_row(
    ui: &mut Ui,
    label: &str,
    help: &str,
    name: &str,
    field: impl FnOnce(&mut Ui) -> Response,
) -> bool {
    settings_row(ui, label, help, |ui| {
        let response = field(ui);
        widgets::name(&response, name);
        response.changed()
    })
}

fn toggle(ui: &mut Ui, value: &mut bool, label: &str, help: &str) -> bool {
    row(ui, label, help, |ui| widgets::toggle(ui, value))
}

fn text(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .password(password)
            .desired_width(ui.available_width()),
    )
}

/// A file path: typed, or chosen with Browse…; a path that names no file says so.
fn path(
    ui: &mut Ui,
    value: &mut String,
    field: PathField,
    name: &str,
    paths: &std::collections::HashMap<String, Option<bool>>,
    actions: &mut Vec<Action>,
) -> Response {
    let response = ui
        .horizontal(|ui| {
            let browse_width = icons::size() + ui.spacing().item_spacing.x;
            let response = ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text("Path to a PEM file")
                    .desired_width((ui.available_width() - browse_width).max(0.0)),
            );
            let browse = icons::button_with_hint(
                ui,
                Icon::Folder,
                &format!("Browse for {}", name.to_lowercase()),
                &format!("Choose the {} file", name.to_lowercase()),
                false,
            );
            if browse.clicked() {
                actions.push(Action::PickFile(widgets::FileRequest::Setting(field)));
            }
            response
        })
        .inner;
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        match paths.get(trimmed) {
            None => actions.push(Action::CheckPath(trimmed.to_owned())),
            Some(Some(false)) => {
                ui.label(tone_text("File not found", Tone::Error));
            }
            Some(_) => {}
        }
    }
    response
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    workspace::list_screen(
        state,
        ui,
        |state, ui| {
            let (width, list) =
                widgets::list_column(ui, "Settings categories", state.layout.list_width);
            state.layout.list_width = width;
            list
        },
        |state, ui, list| {
            draw_toolbar(state, ui);
            widgets::scroll_list(list, |ui| categories(state, ui));
            ui.add_space(theme::spacing().lg);
            let category = state.settings_screen.category;
            egui::ScrollArea::vertical()
                .id_salt(("settings group", category))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    widgets::scroll_view_bounds(ui);
                    ui.set_max_width(ui.available_width().min(widgets::SETTINGS_WIDTH));
                    draw_category(state, ui, category);
                });
        },
    );
}

fn draw_toolbar(state: &mut State, ui: &mut Ui) {
    let screen = &state.settings_screen;
    let word = match screen.status {
        Some(word) => word,
        None if !screen.is_loaded() => SaveWord::Loading,
        None if screen.is_saving() => SaveWord::Saving,
        None if screen.dirty() => SaveWord::Unsaved,
        None => SaveWord::Saved,
    };
    let detail = match word {
        SaveWord::Unsaved => "Unsaved application settings",
        SaveWord::Saved => "The application settings are saved",
        _ => "",
    };
    let status = SaveStatus {
        word,
        detail,
        enabled: screen.dirty() && !screen.is_saving(),
    };
    let mut save = false;
    widgets::toolbar_row(
        ui,
        |_| {},
        |ui| {
            save = widgets::save_controls(ui, &status, "Save settings");
        },
    );
    if save {
        state.actions.push(Action::Save);
    }
}

fn categories(state: &mut State, ui: &mut Ui) {
    let screen = &mut state.settings_screen;
    for category in Category::ALL {
        let (label, icon) = category.item();
        let dirty = screen.category_dirty(category);
        let end = if dirty {
            ListEnd::Unsaved
        } else {
            ListEnd::None
        };
        let (row, _) = widgets::list_row(
            ui,
            label,
            ListIcon::Glyph(icon),
            screen.category == category,
            end,
            dirty.then_some("Unsaved changes"),
        );
        widgets::name(&row, &format!("{label} settings"));
        if row.clicked() {
            screen.category = category;
        }
    }
}

/// Use the same inset as the certificate frames so their controls share the column.
fn group<T>(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui) -> T) -> T {
    Frame::new()
        .inner_margin(egui::vec2(theme::spacing().sm + 1.0, 0.0))
        .show(ui, |ui| {
            settings_heading(ui, title);
            contents(ui)
        })
        .inner
}

fn draw_category(state: &mut State, ui: &mut Ui, category: Category) {
    let enabled = matches!(category, Category::Appearance | Category::Collections)
        || state.settings_screen.is_loaded();
    let edited = ui
        .add_enabled_ui(enabled, |ui| {
            let mut edited = group(ui, category.item().0, |ui| {
                let form = &mut state.settings_screen.form;
                match category {
                    Category::Appearance => {
                        appearance(state, ui);
                        false
                    }
                    Category::Collections => {
                        collections(state, ui);
                        false
                    }
                    Category::General => toggle(
                        ui,
                        &mut form.autosave,
                        "Automatically save changes",
                        "Automatically saves changes to requests, variables and mocks. You can also use Save or Cmd/Ctrl+S.",
                    ),
                    Category::Network => network(ui, form),
                    Category::Tls => {
                        tls(ui, form, &state.settings_screen.paths, &mut state.actions)
                    }
                    Category::History => history(ui, form, state.history.usage),
                    Category::CommandLine => toggle(
                        ui,
                        &mut form.allow_production_from_cli,
                        "Allow production environments",
                        concat!(
                            "imposter send and agents using it may run requests with environments ",
                            "marked red. A command-line flag cannot turn this on; a program ",
                            "that edits the app's database can.",
                        ),
                    ),
                }
            });
            if category == Category::Tls {
                let screen = &mut state.settings_screen;
                edited |= host_certificates(
                    ui,
                    &mut screen.form.host_identities,
                    &screen.paths,
                    &mut state.actions,
                );
            }
            edited
        })
        .inner;
    if edited {
        state.actions.push(Action::SettingsEdited);
    }
}

fn appearance(state: &mut State, ui: &mut Ui) {
    // These preferences apply at once and are stored with the window state.
    let mut appearance = state.appearance;
    settings_row(ui, "Theme", "System follows the operating system.", |ui| {
        widgets::named_segments(
            ui,
            "Theme",
            &mut appearance.theme,
            &theme::ThemeChoice::ALL,
            None,
        );
    });
    settings_row(
        ui,
        "Density",
        "Comfortable gives rows and controls more room.",
        |ui| {
            widgets::named_segments(
                ui,
                "Density",
                &mut appearance.density,
                &theme::Density::ALL,
                None,
            );
        },
    );
    settings_row(
        ui,
        "Response preview",
        "How much of a response body the viewer formats and shows. Save body… and Copy full body always use all of it. Larger previews are slower to scroll.",
        |ui| {
            widgets::named_segments(
                ui,
                "Response preview",
                &mut appearance.preview_limit,
                &crate::viewer::PreviewLimit::ALL,
                None,
            );
        },
    );
    if appearance != state.appearance {
        state.actions.push(Action::SetAppearance(appearance));
    }
}

fn collections(state: &mut State, ui: &mut Ui) {
    let folder = state.collections_folder.display().to_string();
    let folder_error = state.folder_error.as_deref();
    let help = if folder_error.is_some() {
        "It could not be read, so nothing is written to it. Fix the file it names, or choose another folder."
    } else {
        "Each request is a file here. Choose a folder in your repository to share collections with your team and CI."
    };
    settings_row(ui, "Collections folder", help, |ui| {
        ui.horizontal(|ui| {
            let change = widgets::button(ui, Variant::Secondary, Some(Icon::FolderOpen), "Change…");
            widgets::name(&change, "Change collections folder");
            if change.clicked() {
                state.actions.push(Action::ChooseCollectionsFolder);
            }
            ui.add(
                egui::Label::new(theme::meta(&folder).color(theme::palette().text_muted))
                    .truncate(),
            )
            .on_hover_text(&folder);
        });
        if let Some(error) = folder_error {
            ui.add(egui::Label::new(tone_text(error, Tone::Error)).wrap());
        }
    });
}

fn network(ui: &mut Ui, form: &mut SettingsForm) -> bool {
    let mut edited = row(
        ui,
        "Request timeout",
        "For connecting and for each read, not the whole exchange.",
        |ui| {
            ui.add(
                egui::DragValue::new(&mut form.timeout)
                    .range(1..=300)
                    .suffix(" seconds"),
            )
        },
    );
    edited |= row(
        ui,
        "Proxy server",
        "Every request goes through it; empty for none. System proxies are never used.",
        |ui| {
            text(
                ui,
                &mut form.proxy_url,
                "http://proxy.example.com:8080",
                false,
            )
        },
    );
    edited |= row(
        ui,
        "Response body limit",
        "Larger bodies are cut at this size; 0 for no limit.",
        |ui| {
            ui.add(
                egui::DragValue::new(&mut form.download_limit)
                    .range(0..=100 * 1024)
                    .suffix(" KiB"),
            )
        },
    );
    edited |= toggle(
        ui,
        &mut form.follow_redirects,
        "Follow redirects",
        "Follow redirects returned by servers.",
    );
    edited |= toggle(
        ui,
        &mut form.cookie_jar,
        "Keep cookies",
        "Cookies servers set are sent back on later requests, per environment. They are listed on the Variables screen.",
    );
    edited
}

fn tls(
    ui: &mut Ui,
    form: &mut SettingsForm,
    paths: &std::collections::HashMap<String, Option<bool>>,
    actions: &mut Vec<Action>,
) -> bool {
    let mut edited = toggle(
        ui,
        &mut form.verify_ssl,
        "Verify TLS certificates",
        "Turn off only for servers you trust, such as local ones with self-signed certificates.",
    );
    for (label, help, value, field) in [
        (
            "CA bundle",
            "Replaces the system's trusted roots.",
            &mut form.ca_bundle,
            PathField::CaBundle,
        ),
        (
            "Client certificate",
            "Sent when a server asks for one.",
            &mut form.certificate_path,
            PathField::Certificate,
        ),
        (
            "Private key",
            "The private key for the client certificate.",
            &mut form.key_file,
            PathField::Key,
        ),
    ] {
        edited |= row(ui, label, help, |ui| {
            path(ui, value, field, label, paths, actions)
        });
    }
    edited |= row(
        ui,
        "Key password",
        "The password for an encrypted private key.",
        |ui| text(ui, &mut form.password, "", true),
    );
    edited
}

fn host_certificates(
    ui: &mut Ui,
    identities: &mut Vec<HostIdentity>,
    paths: &std::collections::HashMap<String, Option<bool>>,
    actions: &mut Vec<Action>,
) -> bool {
    let mut edited = widgets::add_row_header(
        ui,
        "Host client certificates",
        "Add host certificate",
        Some("Used instead of the client certificate above for one host and port."),
    );
    if edited {
        identities.push(HostIdentity::default());
    }
    let mut remove = None;
    for (index, identity) in identities.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            Frame::new()
                .stroke(egui::Stroke::new(1.0, theme::palette().field_border))
                .corner_radius(6.0)
                .inner_margin(theme::spacing().sm)
                .show(ui, |ui| {
                    let (changed, deleted) = host_certificate(ui, index, identity, paths, actions);
                    edited |= changed;
                    if deleted {
                        remove = Some(index);
                    }
                });
        });
    }
    if let Some(index) = remove {
        identities.remove(index);
        edited = true;
    }
    edited
}

fn host_certificate(
    ui: &mut Ui,
    index: usize,
    identity: &mut HostIdentity,
    paths: &std::collections::HashMap<String, Option<bool>>,
    actions: &mut Vec<Action>,
) -> (bool, bool) {
    let prefix = format!("Host certificate {}", index + 1);
    let title = if !identity.name.trim().is_empty() {
        identity.name.trim().to_owned()
    } else if !identity.host.trim().is_empty() {
        format!("{}:{}", identity.host.trim(), identity.port)
    } else {
        prefix.clone()
    };
    let remove = ui
        .with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let remove = widgets::remove_icon_button(ui, ui.scope_id().with("remove"), &prefix);
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(theme::section(&title))
                        .truncate()
                        .show_tooltip_when_elided(false),
                )
                .on_hover_text(&title);
            });
            remove
        })
        .inner;
    let mut edited = named_row(
        ui,
        "Name",
        "An optional name for this certificate entry.",
        &format!("{prefix} Name"),
        |ui| text(ui, &mut identity.name, "e.g. Staging API", false),
    );
    edited |= named_row(
        ui,
        "Host",
        "The exact HTTPS host this certificate applies to.",
        &format!("{prefix} Host"),
        |ui| text(ui, &mut identity.host, "", false),
    );
    edited |= named_row(
        ui,
        "Port",
        "The HTTPS port. The default is 443.",
        &format!("{prefix} Port"),
        |ui| ui.add(egui::DragValue::new(&mut identity.port).range(1..=65535)),
    );
    for (label, help, value, field) in [
        (
            "Certificate",
            "The client certificate sent to this host.",
            &mut identity.certificate_path,
            PathField::HostCertificate(index),
        ),
        (
            "Key",
            "The private key for this client certificate.",
            &mut identity.key_file,
            PathField::HostKey(index),
        ),
    ] {
        let name = format!("{prefix} {label}");
        edited |= named_row(ui, label, help, &name, |ui| {
            path(ui, value, field, &name, paths, actions)
        });
    }
    edited |= named_row(
        ui,
        "Password",
        "The password for an encrypted private key.",
        &format!("{prefix} Password"),
        |ui| text(ui, &mut identity.password, "", true),
    );
    (edited, remove)
}

fn history(ui: &mut Ui, form: &mut SettingsForm, usage: Option<Usage>) -> bool {
    const RETENTION: &str = "The oldest runs go first when either limit is reached.";
    let mut edited = row(ui, "History entries", RETENTION, |ui| {
        ui.add(egui::DragValue::new(&mut form.history_max_entries).range(1..=1_000_000))
    });
    edited |= row(ui, "History storage", RETENTION, |ui| {
        let response = ui.add(
            egui::DragValue::new(&mut form.history_mib)
                .range(1..=102_400)
                .suffix(" MiB"),
        );
        if let Some(usage) = usage {
            let summary = format!(
                "History holds {} execution{}, {}",
                usage.entries,
                if usage.entries == 1 { "" } else { "s" },
                crate::viewer::format_size(usize::try_from(usage.bytes).unwrap_or(usize::MAX))
            );
            ui.label(theme::meta(summary).color(theme::palette().text_muted));
        }
        response
    });
    edited
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_round_trips_the_settings_it_shows() {
        let settings = AppSettings {
            timeout: 12.0,
            proxy_url: "http://p".into(),
            history_max_bytes: 3 * MIB,
            attach_cookies: false,
            allow_production_from_cli: true,
            ..AppSettings::default()
        };
        let form = SettingsForm::new(&settings);
        assert_eq!(form.apply(&settings), settings);
        assert!(form.allow_production_from_cli);
    }

    #[test]
    fn out_of_range_values_are_clamped_to_what_the_fields_allow() {
        let settings = AppSettings {
            timeout: 0.2,
            download_limit: u64::MAX,
            history_max_entries: 0,
            history_max_bytes: 10,
            ..AppSettings::default()
        };
        let form = SettingsForm::new(&settings);
        assert_eq!(form.timeout, 1);
        assert_eq!(form.download_limit, 100 * 1024);
        assert_eq!(form.history_max_entries, 1);
        assert_eq!(form.history_mib, 1);
        assert!(form.apply(&settings).validate().is_ok());
        assert_eq!(
            SettingsForm::new(&AppSettings {
                timeout: 1e9,
                ..AppSettings::default()
            })
            .timeout,
            300
        );
    }
}

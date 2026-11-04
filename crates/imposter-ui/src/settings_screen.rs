//! The Settings screen: application defaults, edited against a saved baseline. Fields and
//! ranges follow the Python app.

use egui::{Align, Grid, Layout, Ui};
use imposter_app::core::settings::{AppSettings, HostIdentity};

use crate::app::{Action, State};
use crate::theme::Tone;
use crate::widgets::{SaveStatus, SaveWord, save_controls};
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
            proxy_url: settings.proxy_url.clone(),
            download_limit: settings.download_limit.min(100 * 1024),
            ca_bundle: settings.ca_bundle.clone(),
            certificate_path: settings.certificate_path.clone(),
            key_file: settings.key_file.clone(),
            password: settings.password.clone(),
            host_identities: settings.host_identities.clone(),
            history_max_entries: settings.history_max_entries.clamp(1, 1_000_000),
            history_mib: (settings.history_max_bytes / MIB).clamp(1, 102_400),
        }
    }

    /// `base` with the form's fields; settings the screen does not show are kept.
    pub fn apply(&self, base: &AppSettings) -> AppSettings {
        AppSettings {
            timeout: f64::from(self.timeout),
            autosave: self.autosave,
            verify_ssl: self.verify_ssl,
            follow_redirects: self.follow_redirects,
            proxy_url: self.proxy_url.clone(),
            download_limit: self.download_limit,
            ca_bundle: self.ca_bundle.clone(),
            certificate_path: self.certificate_path.clone(),
            key_file: self.key_file.clone(),
            password: self.password.clone(),
            host_identities: self.host_identities.clone(),
            history_max_entries: self.history_max_entries,
            history_max_bytes: self.history_mib * MIB,
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

/// Every section's labels share this column, so fields line up down the whole screen.
const LABEL_COLUMN: f32 = 170.0;
/// The widest the form gets, so fields stay a readable length in a wide window.
const FORM_WIDTH: f32 = 720.0;

/// A section: its title, then its fields in the shared two-column grid.
fn section(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).font(crate::theme::semibold(15.0)));
    ui.add_space(6.0);
    Grid::new(title)
        .num_columns(2)
        .min_col_width(LABEL_COLUMN)
        .max_col_width((ui.available_width() - LABEL_COLUMN - 16.0).max(160.0))
        .spacing([16.0, 10.0])
        .show(ui, contents);
}

/// A drop-down over `options`, showing the label of `value`.
fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &'static str)],
) -> egui::Response {
    let selected = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or("", |(_, label)| *label);
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (option, label) in options {
                ui.selectable_value(value, *option, *label);
            }
        })
        .response
}

/// One labelled row of the grid: the field, and under it what it is for when `help` says.
/// Returns whether the value changed.
fn row(
    ui: &mut Ui,
    label: &str,
    help: Option<&str>,
    field: impl FnOnce(&mut Ui) -> egui::Response,
) -> bool {
    let caption = ui.label(label);
    let changed = ui
        .vertical(|ui| {
            let response = field(ui).labelled_by(caption.id);
            if let Some(help) = help {
                ui.add(egui::Label::new(help_text(help)).wrap());
            }
            response.changed()
        })
        .inner;
    ui.end_row();
    changed
}

/// Under the history limits: what history holds now, so the limits can be judged.
fn storage_help(usage: Option<Usage>) -> String {
    let rule = "The oldest runs go first when either limit is reached.";
    match usage {
        Some(usage) => format!(
            "History holds {} execution{}, {}. {rule}",
            usage.entries,
            if usage.entries == 1 { "" } else { "s" },
            crate::viewer::format_size(usize::try_from(usage.bytes).unwrap_or(usize::MAX))
        ),
        None => rule.to_owned(),
    }
}

fn help_text(text: &str) -> egui::RichText {
    crate::widgets::tone_text(text, Tone::Muted).small()
}

fn text(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .password(password)
            .desired_width(ui.available_width()),
    )
}

/// A check box in the field column, with help under it.
fn check(ui: &mut Ui, value: &mut bool, label: &str, help: Option<&str>) -> bool {
    ui.label("");
    let changed = ui
        .vertical(|ui| {
            let changed = ui.checkbox(value, label).changed();
            if let Some(help) = help {
                ui.add(egui::Label::new(help_text(help)).wrap());
            }
            changed
        })
        .inner;
    ui.end_row();
    changed
}

/// A file path: typed, or chosen with Browse…; a path that names no file says so.
fn path(
    ui: &mut Ui,
    value: &mut String,
    field: PathField,
    name: &str,
    paths: &std::collections::HashMap<String, Option<bool>>,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let response = ui
        .horizontal(|ui| {
            let browse_width = crate::icons::size() + ui.spacing().item_spacing.x;
            let response = ui.add(
                egui::TextEdit::singleline(value)
                    .hint_text("Path to a PEM file")
                    .desired_width((ui.available_width() - browse_width).max(120.0)),
            );
            crate::widgets::name(&response, name);
            let browse = crate::icons::button_with_hint(
                ui,
                crate::icons::Icon::Folder,
                &format!("Browse for {}", name.to_lowercase()),
                &format!("Choose the {} file", name.to_lowercase()),
                false,
            );
            if browse.clicked() {
                actions.push(Action::PickFile(crate::widgets::FileRequest::Setting(
                    field,
                )));
            }
            response
        })
        .inner;
    let trimmed = value.trim();
    if !trimmed.is_empty() {
        match paths.get(trimmed) {
            None => actions.push(Action::CheckPath(trimmed.to_owned())),
            Some(Some(false)) => {
                ui.label(crate::widgets::tone_text("File not found", Tone::Error));
            }
            Some(_) => {}
        }
    }
    response
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let storage_help = storage_help(state.history.usage);
    let screen = &mut state.settings_screen;
    let mut edited = false;
    let mut save = false;
    let mut actions = Vec::new();
    let loaded = screen.is_loaded();
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let word = match screen.status {
                Some(word) => word,
                None if !loaded => SaveWord::Loading,
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
            save = save_controls(ui, &status, "Save settings");
        });
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // At most FORM_WIDTH, never wider than the window: `set_max_width` would widen.
            ui.set_max_width(ui.available_width().min(FORM_WIDTH));
            // Appearance applies at once and is remembered apart from Save: it is how this
            // app looks, not an application setting.
            let mut appearance = state.appearance;
            section(ui, "Appearance", |ui| {
                row(ui, "Theme", Some("System follows the operating system."), |ui| {
                    choice(
                        ui,
                        "theme",
                        &mut appearance.theme,
                        &crate::theme::ThemeChoice::ALL,
                    )
                });
                row(ui, "Density", Some("Comfortable gives rows and controls more room."), |ui| {
                    choice(
                        ui,
                        "density",
                        &mut appearance.density,
                        &crate::theme::Density::ALL,
                    )
                });
            });
            if appearance != state.appearance {
                state.actions.push(Action::SetAppearance(appearance));
            }
            ui.add_enabled_ui(loaded, |ui| {
                let paths = &screen.paths;
                let form = &mut screen.form;
                section(ui, "General", |ui| {
                    edited |= check(
                        ui,
                        &mut form.autosave,
                        "Automatically save request changes",
                        Some("Otherwise Save, or Cmd/Ctrl+S, keeps them."),
                    );
                });
                section(ui, "Network", |ui| {
                    edited |= row(
                        ui,
                        "Request timeout",
                        Some("For connecting and for each read, not the whole exchange."),
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
                        Some("Every request goes through it; empty for none. System proxies are never used."),
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
                        Some("Larger bodies are cut at this size; 0 for no limit."),
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut form.download_limit)
                                    .range(0..=100 * 1024)
                                    .suffix(" KB"),
                            )
                        },
                    );
                    edited |= check(ui, &mut form.follow_redirects, "Follow redirects", None);
                });
                section(ui, "TLS", |ui| {
                    edited |= check(
                        ui,
                        &mut form.verify_ssl,
                        "Verify TLS certificates",
                        Some("Turn off only for servers you trust, such as local ones with self-signed certificates."),
                    );
                    edited |= row(
                        ui,
                        "CA bundle",
                        Some("Replaces the system's trusted roots."),
                        |ui| path(ui, &mut form.ca_bundle, PathField::CaBundle, "CA bundle", paths, &mut actions),
                    );
                    edited |= row(
                        ui,
                        "Client certificate",
                        Some("Sent when a server asks for one."),
                        |ui| {
                            path(
                                ui,
                                &mut form.certificate_path,
                                PathField::Certificate,
                                "Client certificate",
                                paths,
                                &mut actions,
                            )
                        },
                    );
                    edited |= row(ui, "Private key", None, |ui| {
                        path(ui, &mut form.key_file, PathField::Key, "Private key", paths, &mut actions)
                    });
                    edited |= row(ui, "Key password", None, |ui| {
                        text(ui, &mut form.password, "", true)
                    });
                });
                section(ui, "Host client certificates", |ui| {
                    ui.label("");
                    ui.add(
                        egui::Label::new(help_text(
                            "Used instead of the client certificate above for one host and port.",
                        ))
                        .wrap(),
                    );
                    ui.end_row();
                });
                let mut remove = None;
                for (index, identity) in form.host_identities.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        let prefix = format!("Host certificate {}", index + 1);
                        section(ui, &prefix, |ui| {
                            for (label, value, secret) in [
                                ("Host", &mut identity.host, false),
                                ("Password", &mut identity.password, true),
                            ] {
                                ui.label(label);
                                let response = text(ui, value, "", secret);
                                crate::widgets::name(&response, &format!("{prefix} {label}"));
                                edited |= response.changed();
                                ui.end_row();
                            }
                            ui.label("Port");
                            let response =
                                ui.add(egui::DragValue::new(&mut identity.port).range(1..=65535));
                            crate::widgets::name(&response, &format!("{prefix} Port"));
                            edited |= response.changed();
                            ui.end_row();
                            for (label, value, field) in [
                                ("Certificate", &mut identity.certificate_path, PathField::HostCertificate(index)),
                                ("Key", &mut identity.key_file, PathField::HostKey(index)),
                            ] {
                                ui.label(label);
                                let name = format!("{prefix} {label}");
                                edited |= path(ui, value, field, &name, paths, &mut actions).changed();
                                ui.end_row();
                            }
                            ui.label("");
                            if crate::widgets::delete_button(
                                ui,
                                ui.scope_id().with("remove"),
                                &prefix,
                                "Remove host certificate",
                            ) {
                                remove = Some(index);
                            }
                            ui.end_row();
                        });
                    });
                }
                if let Some(index) = remove {
                    form.host_identities.remove(index);
                    edited = true;
                }
                ui.add_space(6.0);
                if ui.button("Add host certificate").clicked() {
                    form.host_identities.push(HostIdentity::default());
                    edited = true;
                }
                section(ui, "History", |ui| {
                    edited |= row(ui, "History entries", None, |ui| {
                        ui.add(
                            egui::DragValue::new(&mut form.history_max_entries)
                                .range(1..=1_000_000),
                        )
                    });
                    edited |= row(
                        ui,
                        "History storage",
                        Some(&storage_help),
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut form.history_mib)
                                    .range(1..=102_400)
                                    .suffix(" MiB"),
                            )
                        },
                    );
                });
                ui.add_space(16.0);
            });
        });
    if edited {
        state.actions.push(Action::SettingsEdited);
    }
    if save {
        state.actions.push(Action::Save);
    }
    state.actions.extend(actions);
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
            ..AppSettings::default()
        };
        let form = SettingsForm::new(&settings);
        assert_eq!(form.apply(&settings), settings);
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

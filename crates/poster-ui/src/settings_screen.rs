//! The Settings screen: application defaults, edited against a saved baseline. Fields and
//! ranges follow the Python app.

use egui::{Grid, Ui};
use poster_app::core::settings::AppSettings;

use crate::app::{Action, State};
use crate::theme::{PALETTE, Tone};
use crate::widgets::tone_text;

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
            history_max_entries: self.history_max_entries,
            history_max_bytes: self.history_mib * MIB,
            ..base.clone()
        }
    }
}

fn group(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(PALETTE.surface)
        .stroke(egui::Stroke::new(1.0, PALETTE.border))
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(6.0);
            Grid::new(title)
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, contents);
        });
    ui.add_space(10.0);
}

/// A labelled field row; returns whether the value changed.
fn row(ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    let caption = ui.label(label);
    let response = field(ui).labelled_by(caption.id);
    ui.end_row();
    response.changed()
}

fn text(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .password(password)
            .desired_width(360.0),
    )
}

fn check(ui: &mut Ui, value: &mut bool, label: &str) -> bool {
    ui.label("");
    let changed = ui.checkbox(value, label).changed();
    ui.end_row();
    changed
}

pub fn draw(state: &mut State, ui: &mut Ui) {
    let screen = &mut state.settings_screen;
    let mut edited = false;
    let mut save = false;
    let loaded = screen.is_loaded();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(ui.available_height() - 72.0)
        .show(ui, |ui| {
            ui.set_max_width(760.0);
            ui.add_enabled_ui(loaded, |ui| {
                let form = &mut screen.form;
                group(ui, "General", |ui| {
                    edited |= check(ui, &mut form.autosave, "Automatically save request changes");
                });
                group(ui, "Network", |ui| {
                    edited |= row(ui, "Request timeout", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut form.timeout)
                                .range(1..=300)
                                .suffix(" seconds"),
                        )
                    });
                    edited |= row(ui, "Proxy server", |ui| {
                        text(
                            ui,
                            &mut form.proxy_url,
                            "http://proxy.example.com:8080",
                            false,
                        )
                    });
                    edited |= row(ui, "Response body limit", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut form.download_limit)
                                .range(0..=100 * 1024)
                                .suffix(" KB (0 for unlimited)"),
                        )
                    });
                    edited |= check(ui, &mut form.follow_redirects, "Follow redirects");
                });
                group(ui, "TLS", |ui| {
                    edited |= check(ui, &mut form.verify_ssl, "Verify SSL certificates");
                    edited |= row(ui, "CA bundle", |ui| {
                        text(ui, &mut form.ca_bundle, "", false)
                    });
                    edited |= row(ui, "Client certificate", |ui| {
                        text(ui, &mut form.certificate_path, "", false)
                    });
                    edited |= row(ui, "Private key", |ui| {
                        text(ui, &mut form.key_file, "", false)
                    });
                    edited |= row(ui, "Key password", |ui| {
                        text(ui, &mut form.password, "", true)
                    });
                });
                group(ui, "History", |ui| {
                    edited |= row(ui, "History entries", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut form.history_max_entries)
                                .range(1..=1_000_000),
                        )
                    });
                    edited |= row(ui, "History storage", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut form.history_mib)
                                .range(1..=102_400)
                                .suffix(" MiB"),
                        )
                    });
                });
            });
        });
    ui.add_space(8.0);
    if let Some((status, tone)) = &screen.status {
        ui.label(tone_text(status.as_str(), *tone));
    } else if !loaded {
        ui.label(tone_text("Loading settings…", Tone::Muted));
    }
    let enabled = screen.dirty() && !screen.is_saving();
    if ui
        .add_enabled(enabled, egui::Button::new("Save Settings"))
        .clicked()
    {
        save = true;
    }
    if edited {
        state.actions.push(Action::SettingsEdited);
    }
    if save {
        state.actions.push(Action::Save);
    }
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

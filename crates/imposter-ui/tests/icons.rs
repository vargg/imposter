//! Phosphor glyphs preserve native button behavior and fit both control densities.
#![cfg(test)]

#[allow(clippy::disallowed_methods)] // test harness uses temp stores and worker waits
mod support;

use egui::{Color32, Shape, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use imposter_ui::{
    icons::{self, Icon},
    theme::{self, Density, ThemeChoice},
};

fn harness(theme: ThemeChoice, density: Density) -> Harness<'static> {
    Harness::builder()
        .with_context_setup(move |ctx| {
            ctx.set_fonts(theme::fonts());
            theme::apply(ctx, theme, density);
        })
        .build_ui(|ui| {
            ui.horizontal(|ui| {
                icons::button_with_hint(ui, Icon::Copy, "Copy", "Copy response", false);
                icons::button(ui, Icon::Wrap, "Wrap", true);
                icons::ready_button(ui, Icon::Save, "Save", "Save changes");
                ui.add_enabled_ui(false, |ui| {
                    icons::button_with_hint(ui, Icon::Revert, "Revert", "Nothing to revert", false);
                });
            });
        })
}

fn painted_icon<'a, State>(
    harness: &'a Harness<'_, State>,
    icon: Icon,
) -> &'a egui::epaint::TextShape {
    harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.job.text == icon.glyph().to_string() => Some(text),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{icon:?} was not painted"))
}

fn color(harness: &Harness<'_>, icon: Icon) -> Color32 {
    let text = painted_icon(harness, icon);
    let color = text.galley.job.sections[0].format.color;
    if color == Color32::PLACEHOLDER {
        text.fallback_color
    } else {
        color
    }
}

#[test]
fn glyph_buttons_keep_their_names_size_alignment_and_state_colors() {
    for choice in [ThemeChoice::Dark, ThemeChoice::Light] {
        for density in [Density::Compact, Density::Comfortable] {
            let mut harness = harness(choice, density);
            let palette = theme::palette();
            for (icon, name) in [
                (Icon::Copy, "Copy"),
                (Icon::Wrap, "Wrap"),
                (Icon::Save, "Save"),
                (Icon::Revert, "Revert"),
            ] {
                let rect = harness.get_by_role_and_label(Role::Button, name).rect();
                assert_eq!(rect.size(), egui::Vec2::splat(density.spacing().control));
                let text = painted_icon(&harness, icon);
                let center = text.pos + text.galley.size() / 2.0;
                assert!(
                    center.distance(rect.center()) < 0.1,
                    "{icon:?}: {center:?} in {rect:?}"
                );
                assert!(
                    rect.contains_rect(text.visual_bounding_rect()),
                    "{icon:?} fits its target"
                );
                assert_eq!(text.galley.job.sections[0].format.font_id.size, 16.0);
            }
            assert_eq!(color(&harness, Icon::Copy), palette.text_muted);
            assert_eq!(color(&harness, Icon::Wrap), palette.accent);
            assert_eq!(color(&harness, Icon::Save), palette.text);
            assert_eq!(
                color(&harness, Icon::Revert),
                palette.text_muted.gamma_multiply(0.45).gamma_multiply(
                    harness
                        .ctx
                        .style_of(harness.ctx.theme())
                        .visuals
                        .disabled_alpha()
                )
            );

            harness.get_by_label("Copy").hover();
            harness.run_steps(2);
            assert_eq!(color(&harness, Icon::Copy), palette.text);
            harness.get_by_label("Copy").focus();
            harness.run_steps(2);
            assert_eq!(color(&harness, Icon::Copy), palette.accent);
        }
    }
}

#[test]
fn enabled_and_disabled_buttons_keep_their_hover_text() {
    let mut harness = harness(ThemeChoice::Dark, Density::Compact);
    harness.get_by_label("Copy").hover();
    harness.run_steps(4);
    harness.get_by_label("Copy response");
    harness.get_by_label("Revert").hover();
    harness.run_steps(4);
    harness.get_by_label("Nothing to revert");
}

#[test]
fn every_icon_can_be_used_in_rich_text() {
    let mut harness = Harness::builder()
        .with_context_setup(|ctx| ctx.set_fonts(theme::fonts()))
        .build_ui(|ui| {
            ui.horizontal_wrapped(|ui| {
                for icon in Icon::ALL {
                    ui.label(icons::glyph_text(icon, 20.0));
                }
            });
        });
    harness.run_steps(1);
    for icon in Icon::ALL {
        assert_eq!(
            painted_icon(&harness, icon).galley.job.sections[0]
                .format
                .font_id
                .size,
            20.0
        );
    }
}

#[test]
fn icons_use_phosphor_even_when_inter_maps_the_same_codepoint() {
    use skrifa::MetadataProvider;

    let fonts = theme::fonts();
    assert_eq!(Icon::Settings.glyph(), '\u{e272}');
    for name in ["Inter", "Inter-SemiBold"] {
        let font = skrifa::FontRef::new(fonts.font_data[name].font.as_ref().as_ref()).unwrap();
        assert!(
            font.charmap().map(Icon::Settings.glyph()).is_some(),
            "{name} reproduces the private-use collision that drew Settings as a letter"
        );
    }

    for as_button in [false, true] {
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| ctx.set_fonts(theme::fonts()))
            .build_ui(move |ui| {
                ui.horizontal_wrapped(|ui| {
                    for icon in Icon::ALL {
                        if as_button {
                            icons::button(ui, icon, &format!("{icon:?}"), false);
                        } else {
                            ui.label(icons::glyph_text(icon, 16.0));
                        }
                    }
                });
            });
        harness.run_steps(1);
        for icon in Icon::ALL {
            let text = painted_icon(&harness, icon);
            let family = &text.galley.job.sections[0].format.font_id.family;
            assert_eq!(
                family,
                &egui::FontFamily::Name("icons".into()),
                "{icon:?} must bypass Inter for both labels and buttons"
            );
            assert_eq!(fonts.families[family], ["Phosphor"]);
        }
    }
}

#[derive(Debug, PartialEq)]
struct GlyphPixels {
    advance: f32,
    offset: egui::Vec2,
    size: [usize; 2],
    pixels: Vec<Color32>,
}

fn rasterized_icons(
    fonts: egui::FontDefinitions,
    warm_inter: bool,
    as_button: bool,
) -> Vec<GlyphPixels> {
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        if warm_inter {
            ui.fonts_mut(|fonts| {
                for family in [
                    egui::FontFamily::Proportional,
                    egui::FontFamily::Name(theme::SEMIBOLD.into()),
                ] {
                    let font = egui::FontId::new(16.0, family);
                    for icon in Icon::ALL {
                        if fonts.has_glyph(&font, icon.glyph()) {
                            fonts.layout_no_wrap(
                                icon.glyph().to_string(),
                                font.clone(),
                                Color32::WHITE,
                            );
                        }
                    }
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            for icon in Icon::ALL {
                if as_button {
                    icons::button(ui, icon, &format!("{icon:?}"), false);
                } else {
                    ui.label(icons::glyph_text(icon, 16.0));
                }
            }
        });
    });
    output.textures_delta.clear();
    let atlas = ctx.fonts_mut(|fonts| fonts.image());
    Icon::ALL
        .into_iter()
        .map(|icon| {
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    Shape::Text(text) if text.galley.job.text == icon.glyph().to_string() => {
                        Some(text)
                    }
                    _ => None,
                })
                .unwrap();
            glyph_pixels(text, &atlas)
        })
        .collect()
}

fn glyph_pixels(text: &egui::epaint::TextShape, atlas: &egui::ColorImage) -> GlyphPixels {
    let glyph = &text.galley.rows[0].glyphs[0];
    let uv = glyph.uv_rect;
    let pixels = (uv.min[1]..uv.max[1])
        .flat_map(|y| (uv.min[0]..uv.max[0]).map(move |x| [usize::from(x), usize::from(y)]))
        .map(|[x, y]| atlas.pixels[y * atlas.size[0] + x])
        .collect();
    GlyphPixels {
        advance: glyph.advance_width,
        offset: uv.offset,
        size: [
            usize::from(uv.max[0] - uv.min[0]),
            usize::from(uv.max[1] - uv.min[1]),
        ],
        pixels,
    }
}

fn phosphor_only() -> egui::FontDefinitions {
    let mut reference = egui::FontDefinitions::empty();
    reference.font_data.insert(
        "Phosphor".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Phosphor.ttf"
        ))),
    );
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
        egui::FontFamily::Name(theme::ICONS.into()),
    ] {
        reference.families.insert(family, vec!["Phosphor".into()]);
    }
    reference
}

#[test]
fn icon_pixels_match_a_phosphor_only_atlas_even_after_inter_warms_the_cache() {
    let expected = rasterized_icons(phosphor_only(), false, false);
    for as_button in [false, true] {
        let actual = rasterized_icons(theme::fonts(), true, as_button);
        for ((icon, actual), expected) in Icon::ALL.into_iter().zip(actual).zip(&expected) {
            assert!(
                expected.pixels.iter().any(|pixel| pixel.a() > 0),
                "{icon:?} has ink"
            );
            assert_eq!(&actual, expected, "{icon:?}, button={as_button}");
        }
    }
    let mut inter_first = theme::fonts();
    inter_first.families.insert(
        egui::FontFamily::Name(theme::ICONS.into()),
        vec!["Inter".into(), "Phosphor".into()],
    );
    let wrong = rasterized_icons(inter_first, true, true);
    for icon in [Icon::Settings, Icon::More, Icon::Save, Icon::Filter] {
        let index = Icon::ALL
            .iter()
            .position(|candidate| *candidate == icon)
            .unwrap();
        assert_ne!(
            wrong[index], expected[index],
            "{icon:?}: the pixel comparison must detect Inter's colliding glyph"
        );
    }
}

#[test]
fn real_app_toolbar_pixels_match_phosphor() {
    let expected = rasterized_icons(phosphor_only(), false, false);
    let mut app = support::start_with(|store| {
        let api = support::collection(store, "API");
        support::request(store, api.id, "Get", "http://example.invalid");
    });
    support::click(&mut app.harness, "GET Get");
    support::settle(&mut app.harness);
    app.harness
        .state_mut()
        .state
        .response
        .show(imposter_app::core::execution::Response {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: br#"{"key":1}"#.to_vec(),
            ..Default::default()
        });
    app.harness.run_steps(2);
    let atlas = app.harness.ctx.fonts_mut(|fonts| fonts.image());
    for icon in [
        Icon::Settings,
        Icon::More,
        Icon::Save,
        Icon::Filter,
        Icon::Copy,
    ] {
        let index = Icon::ALL
            .iter()
            .position(|candidate| *candidate == icon)
            .unwrap();
        let text = painted_icon(&app.harness, icon);
        assert_eq!(glyph_pixels(text, &atlas), expected[index], "{icon:?}");
    }
}

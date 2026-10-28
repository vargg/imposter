//! Measures how egui copes with large bodies; the numbers behind the viewer and editor
//! decisions in AGENTS.md. Re-run after egui upgrades:
//! `cargo run --release -p imposter-ui --example text_perf`.
//! Times egui passes (layout and shapes, no GPU) through kittest, plus highlighting and search.
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop
#![allow(clippy::unwrap_used)] // measurement program, not app code

use std::fmt::Write as _;
use std::time::Instant;

use egui_kittest::Harness;

fn json(bytes: usize) -> String {
    let mut out = String::from("[\n");
    let mut i = 0;
    while out.len() < bytes {
        write!(
            out,
            "  {{\n    \"id\": {i},\n    \"name\": \"item number {i}\",\n    \"active\": {},\n    \"score\": {}.5,\n    \"tags\": [\"alpha\", \"beta\", null]\n  }},\n",
            i % 2 == 0,
            i % 1000
        )
        .unwrap();
        i += 1;
    }
    out.push_str("  {}\n]\n");
    out
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    println!(
        "{label:<58} {:>9.1} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    value
}

fn frames(label: &str, harness: &mut Harness<'_>) {
    time(&format!("{label}: first pass"), || harness.step());
    let start = Instant::now();
    for _ in 0..20 {
        harness.step();
    }
    println!(
        "{:<58} {:>9.1} ms",
        format!("{label}: steady pass (avg of 20)"),
        start.elapsed().as_secs_f64() * 50.0
    );
}

/// Minimal JSON highlighter: strings, numbers, keywords, punctuation.
fn json_job(text: &str) -> egui::text::LayoutJob {
    use egui::{Color32, FontId, TextFormat};
    let font = FontId::monospace(13.0);
    let fmt = |color| TextFormat {
        font_id: font.clone(),
        color,
        ..TextFormat::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let color = match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                Color32::from_rgb(0xce, 0x91, 0x78)
            }
            b'0'..=b'9' | b'-' => {
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'-' | b'e' | b'E' | b'+')
                {
                    i += 1;
                }
                Color32::from_rgb(0xb5, 0xce, 0xa8)
            }
            b't' | b'f' | b'n' => {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                Color32::from_rgb(0x56, 0x9c, 0xd6)
            }
            _ => {
                while i < bytes.len()
                    && !matches!(bytes[i], b'"' | b'0'..=b'9' | b'-' | b't' | b'f' | b'n')
                {
                    i += 1;
                }
                Color32::from_rgb(0xd4, 0xd4, 0xd4)
            }
        };
        job.append(&text[start..i], 0.0, fmt(color));
    }
    job
}

fn main() {
    let big = json(5 * 1024 * 1024);
    let preview = &big[..big.floor_char_boundary(256 * 1024)];
    println!(
        "bodies: viewer {} B, preview {} B\n",
        big.len(),
        preview.len()
    );
    search_and_highlight(&big, preview);
    viewers(&big, preview);
    editors();
}

fn harness<'a>(app: impl FnMut(&mut egui::Ui) + 'a) -> Harness<'a> {
    Harness::builder()
        .with_size(egui::vec2(1000.0, 650.0))
        .with_accessibility_check(false)
        .build_ui(app)
}

fn search_and_highlight(big: &str, preview: &str) {
    // Search across the whole captured body.
    let hits = time("search 5 MiB for \"item number 4\" (all matches)", || {
        big.match_indices("item number 4").count()
    });
    println!("{:<58} {hits:>9}", "  matches");
    let lower = time(
        "case-insensitive search 5 MiB (lowercase copy + find)",
        || big.to_lowercase().match_indices("item number 4").count(),
    );
    assert_eq!(lower, hits);

    // Highlighting cost of the 256 KiB preview.
    time("hand-written JSON highlighter, 256 KiB", || {
        json_job(preview)
    });
    println!();
}

fn viewers(big: &str, preview: &str) {
    // Viewer: whole 5 MiB in one read-only TextEdit.
    let text = big.to_owned();
    let mut h = harness(move |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text.as_str())
                    .code_editor()
                    .desired_width(f32::INFINITY),
            );
        });
    });
    frames("viewer, 5 MiB read-only TextEdit", &mut h);

    // Viewer: 5 MiB as virtualized rows (only visible lines laid out).
    let lines: Vec<&str> = big.lines().collect();
    let line_count = lines.len();
    let mut h = harness(move |ui| {
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical().show_rows(ui, row, line_count, |ui, range| {
            for line in &lines[range] {
                ui.monospace(*line);
            }
        });
    });
    frames("viewer, 5 MiB virtualized rows", &mut h);

    // Highlighted preview: 256 KiB job laid out once and cached by egui.
    let job = json_job(preview);
    let mut h = harness(move |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(job.clone());
        });
    });
    frames("viewer, 256 KiB highlighted label", &mut h);

    // Viewer: per-line highlighting inside virtualized rows.
    let lines: Vec<String> = big.lines().map(str::to_owned).collect();
    let count = lines.len();
    let mut h = harness(move |ui| {
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical().show_rows(ui, row, count, |ui, range| {
            for line in &lines[range] {
                ui.label(json_job(line));
            }
        });
    });
    frames("viewer, 5 MiB virtualized rows, per-line highlight", &mut h);
    {
        use egui_kittest::kittest::Queryable;
        let shown = h
            .query_all_by(|node| node.value().is_some_and(|v| v.contains("item number")))
            .count()
            + h.query_all_by(|node| node.label().is_some_and(|v| v.contains("item number")))
                .count();
        println!(
            "{:<58} {shown:>9}",
            "  visible rows containing \"item number\""
        );
        assert!(shown > 5, "virtualized rows must render text");
    }
    println!();
}

fn editors() {
    // Editors: steady (unchanged) and typing (text changes every pass), plain and highlighted.
    for kib in [16, 64, 256, 1024] {
        for (highlight, typing) in [(false, false), (false, true), (true, true)] {
            let mut body = json(kib * 1024);
            let mut h = harness(move |ui| {
                if typing {
                    body.push('x');
                }
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                    let mut job = json_job(text.as_str());
                    job.wrap.max_width = wrap;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let edit = egui::TextEdit::multiline(&mut body)
                        .code_editor()
                        .desired_width(f32::INFINITY);
                    if highlight {
                        ui.add(edit.layouter(&mut layouter))
                    } else {
                        ui.add(edit)
                    };
                });
            });
            let label = format!(
                "editor {kib:>4} KiB{}{}",
                if highlight {
                    ", highlighted"
                } else {
                    ", plain"
                },
                if typing { ", typing" } else { ", steady" }
            );
            frames(&label, &mut h);
        }
    }
}

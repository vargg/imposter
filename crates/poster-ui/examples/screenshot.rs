//! Render the real app with sample data to PNG files, for looking at the layout without a
//! display: `cargo run -p poster-ui --example screenshot -- <output-dir>`.
#![allow(clippy::unwrap_used)] // developer tool, not app code

use std::time::Duration;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use poster_app::core::model::{BodyType, ParamItem, RequestTemplate};
use poster_app::paths::open_store;
use poster_app::store::collections;
use poster_ui::app::PosterApp;

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(poster_testkit::Server::start(|_, _| {
        poster_testkit::response(
            "200 OK",
            &[("Content-Type", "application/json"), ("X-Request-Id", "abc-123")],
            br#"{"users":[{"id":1,"name":"Ann","admin":true},{"id":2,"name":"Bo","admin":false}],"total":2,"next":null}"#,
        )
    }));
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open_store(dir.path()).unwrap();
        let url = format!("{}/users?page=1", server.url);
        store
            .call_blocking(move |db| {
                let api = collections::create_collection(
                    db,
                    "Example API",
                    "",
                    &poster_app::core::model::Variables::new(),
                )?;
                let users = collections::create_folder(db, api.id, None, "Users", "", 1)?;
                collections::create_folder(db, api.id, Some(users.id), "Admin", "", 0)?;
                let list = RequestTemplate {
                    name: "List users".into(),
                    url,
                    queries: vec![ParamItem::new("page", "1")],
                    headers: vec![ParamItem::new("Accept", "application/json")],
                    body_type: BodyType::None,
                    ..RequestTemplate::default()
                };
                collections::create_request(db, api.id, Some(users.id), &list)?;
                for (name, method, position) in [
                    ("Health", "GET", 0),
                    ("Create user", "POST", 2),
                    ("Delete user", "DELETE", 3),
                ] {
                    let template = RequestTemplate {
                        name: name.into(),
                        method: method.into(),
                        position,
                        ..RequestTemplate::default()
                    };
                    collections::create_request(db, api.id, None, &template)?;
                }
                Ok(())
            })
            .unwrap();
        store.close().unwrap();
    }
    for (label, size) in [
        ("narrow", egui::vec2(1000.0, 650.0)),
        ("wide", egui::vec2(1500.0, 850.0)),
    ] {
        let store = open_store(dir.path()).unwrap();
        let mut harness = Harness::builder()
            .with_size(size)
            .with_max_steps(64)
            .wgpu()
            .build_eframe(move |cc| PosterApp::new(&cc.egui_ctx, store).unwrap());
        let settle = |harness: &mut Harness<'_, PosterApp>| {
            for _ in 0..20 {
                harness.state().bridge().wait_idle(Duration::from_secs(10));
                harness.run_steps(2);
            }
        };
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-empty.png")))
            .unwrap();
        harness.get_by_label("GET List users").click();
        settle(&mut harness);
        harness.get_by_label("Send").click();
        settle(&mut harness);
        let image = harness.render().unwrap();
        let path = out.join(format!("{label}.png"));
        image.save(&path).unwrap();
        println!("{}", path.display());
        harness.get_by_label("New").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-new-menu.png")))
            .unwrap();
        harness.key_press(egui::Key::Escape);
        settle(&mut harness);
        harness.get_by_label("Hide Collections").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-sidebar-hidden.png")))
            .unwrap();
        harness.get_by_label("Layout").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-layout-menu.png")))
            .unwrap();
        harness.key_press(egui::Key::Escape);
        settle(&mut harness);
        harness.get_by_label("Show Collections").click();
        settle(&mut harness);
        harness.get_by_label("HTTP method").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-method-menu.png")))
            .unwrap();
        harness.get_by_label("Custom method…").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-custom-method.png")))
            .unwrap();
        harness.get_by_label("Variables").click();
        settle(&mut harness);
        harness
            .render()
            .unwrap()
            .save(out.join(format!("{label}-variables.png")))
            .unwrap();
    }
}

//! Event streams in the app: events show as they arrive while Send reads as Stop, Stop
//! keeps them, and a request's listening limit ends the stream on its Events tab.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // drives the app from outside its event loop

mod support;

use std::io::{Read as _, Write as _};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use imposter_app::core::model::{Options, RequestTemplate, Variables};
use imposter_app::core::sse::StreamEnd;
use imposter_app::store::collections;
use imposter_ui::app::ResponseTab;
use support::{click, click_button, settle, start_with};

/// A server whose every answer is two events, the connection then held open.
fn serve() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let _ = socket.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: price\ndata: {\"p\": 1}\n\nevent: price\ndata: {\"p\": 2}\n\n",
                );
                let _ = socket.flush();
                let _ = socket.read(&mut byte);
            });
        }
    });
    base
}

fn app(options: Options) -> support::App {
    let base = serve();
    start_with(move |store| {
        store
            .call_blocking(move |db| {
                let api = collections::create_collection(db, "API", "", &Variables::new())?;
                collections::create_request(
                    db,
                    api.id,
                    None,
                    &RequestTemplate {
                        name: "Prices".into(),
                        method: "GET".into(),
                        url: format!("{base}/prices"),
                        options,
                        ..RequestTemplate::default()
                    },
                )?;
                Ok(())
            })
            .unwrap();
    })
}

#[test]
fn events_show_as_they_come_and_stop_keeps_them() {
    let mut app = app(Options::default());
    click(&mut app.harness, "GET Prices");
    settle(&mut app.harness);
    let send = app.harness.get_by_label("Send").rect();
    click(&mut app.harness, "Send");
    // The stream stays open: wait for its events, not for the send to end.
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.harness.state().state.response.live_events.len() < 2 {
        assert!(Instant::now() < deadline, "no events arrived");
        app.harness.run_steps(1);
    }
    app.harness.run_steps(2);
    app.harness
        .get_by_label_contains("Receiving events… 2 so far");
    app.harness
        .get_by_label_contains("Event 2: price {\"p\": 2}");
    assert_eq!(
        app.harness
            .get_by_role_and_label(egui::Role::Button, "Stop")
            .rect(),
        send
    );

    click_button(&mut app.harness, "Stop");
    settle(&mut app.harness);
    let state = &app.harness.state().state;
    let stream = state
        .response
        .response
        .as_ref()
        .unwrap()
        .stream
        .as_ref()
        .unwrap();
    assert_eq!(stream.end, StreamEnd::Cancelled);
    assert_eq!(stream.events.len(), 2);
    assert_eq!(state.response.tab, ResponseTab::Events);
    app.harness.get_by_label("Stream stopped · 2 events");
}

#[test]
fn a_listening_limit_ends_the_stream_on_its_events_tab() {
    let mut app = app(Options {
        stream_events: Some(2),
        ..Options::default()
    });
    click(&mut app.harness, "GET Prices");
    settle(&mut app.harness);
    click(&mut app.harness, "Send");
    settle(&mut app.harness);
    app.harness
        .get_by_label("Stream stopped at its listening limit · 2 events");
    app.harness
        .get_by_label_contains("Event 1: price {\"p\": 1}");
    click(&mut app.harness, "Event 1: price {\"p\": 1}");
    app.harness.get_by_label_contains("price · +");
}

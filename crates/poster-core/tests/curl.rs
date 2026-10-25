//! curl commands in and out: browsers' "Copy as cURL" output, hand-written options, errors, and
//! that a generated command parses back to the same prepared request.
#![cfg(test)]

use poster_core::curl::{CurlError, parse, to_command};
use poster_core::model::{
    AuthKind, BodyType, FormItem, FormKind, ParamItem, RequestAuth, RequestTemplate,
};
use poster_core::prepare::prepare;
use poster_core::settings::AppSettings;

fn template(command: &str) -> RequestTemplate {
    let parsed = parse(command).unwrap();
    assert_eq!(parsed.warnings, Vec::<String>::new(), "{command}");
    parsed.template
}

fn headers(template: &RequestTemplate) -> Vec<(&str, &str)> {
    template
        .headers
        .iter()
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect()
}

#[test]
fn chrome_bash_json_post() {
    let t = template(
        "curl 'https://api.example.com/v1/items?x=1' \\\n  -H 'accept: application/json' \\\n  \
         -H 'content-type: application/json' \\\n  --data-raw '{\"name\":\"a b\",\"n\":1}'",
    );
    assert_eq!(t.method, "POST");
    assert_eq!(t.url, "https://api.example.com/v1/items?x=1");
    assert_eq!(
        headers(&t),
        [
            ("accept", "application/json"),
            ("content-type", "application/json")
        ]
    );
    assert_eq!(t.body_type, BodyType::Json);
    assert_eq!(t.body, br#"{"name":"a b","n":1}"#);
    assert_eq!(t.name, "/v1/items");
}

#[test]
fn ansi_c_quoting_as_chrome_writes_special_bodies() {
    let t = template(
        r"curl 'http://h/' -H 'content-type: text/plain' --data-raw $'it\'s\n\x41\u00e9\U0001F600'",
    );
    assert_eq!(t.body, "it's\nA\u{e9}\u{1F600}".as_bytes());
    assert_eq!(t.body_type, BodyType::Text);
}

#[test]
fn firefox_form_post() {
    let t =
        template("curl 'https://h/p' -X POST -H 'User-Agent: x' --data-raw 'a=1&b=two%20words'");
    assert_eq!(t.method, "POST");
    assert_eq!(t.body_type, BodyType::Urlencoded);
    assert_eq!(
        t.form_fields().unwrap(),
        [ParamItem::new("a", "1"), ParamItem::new("b", "two words")]
    );
}

#[test]
fn chrome_cmd_escapes() {
    let t = template(
        "curl ^\"https://h/p^\" ^\r\n  -H ^\"accept: */*^\" ^\r\n  --data-raw ^\"^{^\\^\"a^\\^\":1^}^\"",
    );
    assert_eq!(t.url, "https://h/p");
    assert_eq!(headers(&t), [("accept", "*/*")]);
    assert_eq!(t.body, br#"{"a":1}"#);
    assert_eq!(t.body_type, BodyType::Json);
}

#[test]
fn combined_short_options_and_raw_data_keep_curls_form_label() {
    let t = template("curl -sSL -XPUT -H'X: 1' http://h -d 'raw text'");
    assert_eq!(t.method, "PUT");
    assert_eq!(t.options.follow_redirects, Some(true));
    assert_eq!(t.body_type, BodyType::Text);
    assert_eq!(t.body, b"raw text");
    assert_eq!(
        headers(&t),
        [
            ("X", "1"),
            ("Content-Type", "application/x-www-form-urlencoded")
        ]
    );
}

#[test]
fn authentication() {
    let digest = template("curl --digest -u 'ann:p:w' http://h");
    assert_eq!(digest.auth, RequestAuth::digest("ann", "p:w"));
    assert_eq!(
        template("curl -u ann http://h").auth,
        RequestAuth::basic("ann", "")
    );
    let bearer = template("curl --oauth2-bearer t0k http://h");
    assert_eq!(bearer.auth.kind, Some(AuthKind::BearerToken));
}

#[test]
fn multipart_forms_drop_a_pasted_boundary() {
    let t = template(
        "curl -F 'note=hi' -F 'file=@/tmp/a.png;type=image/png;filename=b.png' \
         --form-string 'raw=@not-a-file' -H 'Content-Type: multipart/form-data; boundary=x' http://h",
    );
    assert_eq!(t.method, "POST");
    assert_eq!(t.body_type, BodyType::Multipart);
    assert_eq!(t.headers, []);
    let fields = t.multipart_fields().unwrap();
    assert_eq!(fields[0], FormItem::text("note", "hi"));
    assert_eq!(fields[1].kind, FormKind::File);
    assert_eq!(
        (
            fields[1].value.as_str(),
            fields[1].content_type.as_str(),
            fields[1].filename.as_str()
        ),
        ("/tmp/a.png", "image/png", "b.png")
    );
    assert_eq!(fields[2], FormItem::text("raw", "@not-a-file"));
}

#[test]
fn get_moves_data_into_the_query() {
    let t = template("curl -G http://h/s -d q=1 --data-urlencode 'w=a b'");
    assert_eq!(t.method, "GET");
    assert_eq!(t.url, "http://h/s?q=1&w=a+b");
    assert_eq!(t.body_type, BodyType::None);
}

#[test]
fn data_files_become_binary_bodies() {
    let t = template(
        "curl --data-binary @/tmp/body.bin -H 'Content-Type: application/octet-stream' http://h",
    );
    assert_eq!(t.body_type, BodyType::Binary);
    assert_eq!(t.body, b"/tmp/body.bin");
}

#[test]
fn cookies_headers_and_transport_options() {
    let t = template(
        "curl -b 'a=1; b=2' -A agent -e http://from -k -x http://proxy:8080 -m 5 \
         --cacert ca.pem --cert c.pem:pw --key k.pem -I http://h",
    );
    assert_eq!(t.method, "HEAD");
    assert_eq!(
        t.cookies,
        [ParamItem::new("a", "1"), ParamItem::new("b", "2")]
    );
    assert_eq!(
        headers(&t),
        [("User-Agent", "agent"), ("Referer", "http://from")]
    );
    let o = &t.options;
    assert_eq!(o.verify_ssl, Some(false));
    assert_eq!(o.proxy_url.as_deref(), Some("http://proxy:8080"));
    assert_eq!(o.timeout, Some(5.0));
    assert_eq!(o.ca_bundle.as_deref(), Some("ca.pem"));
    assert_eq!(o.certificate_path.as_deref(), Some("c.pem"));
    assert_eq!(o.password.as_deref(), Some("pw"));
    assert_eq!(o.key_file.as_deref(), Some("k.pem"));
}

#[test]
fn unsupported_options_are_warnings_not_errors() {
    let parsed = parse("curl --http2 -o out.txt --compressed http://h extra").unwrap();
    assert_eq!(
        parsed.warnings,
        [
            "--http2 is not supported and was ignored",
            "--output is not supported and was ignored",
            "extra: only the first URL is used",
        ]
    );
}

#[test]
fn errors_say_what_is_wrong() {
    assert_eq!(parse("wget http://h"), Err(CurlError::NotCurl));
    assert_eq!(parse("curl 'http://h"), Err(CurlError::UnclosedQuote));
    assert_eq!(
        parse("curl http://h -H"),
        Err(CurlError::MissingValue("-H".into()))
    );
    assert_eq!(parse("curl -s"), Err(CurlError::NoUrl));
    assert!(matches!(
        parse("curl -m abc http://h"),
        Err(CurlError::InvalidValue { .. })
    ));
}

#[test]
fn commands_are_quoted_for_a_posix_shell() {
    let request = RequestTemplate {
        method: "POST".into(),
        url: "http://h/it's".into(),
        headers: vec![ParamItem::new("X-Note", "a 'b'")],
        body: vec![b'o', b'k', 0xff],
        body_type: BodyType::Text,
        ..RequestTemplate::default()
    };
    let settings = AppSettings {
        follow_redirects: false,
        ..AppSettings::default()
    };
    let command = to_command(&prepare(&request, true).unwrap(), &settings);
    assert_eq!(
        command,
        "curl 'http://h/it'\\''s' \\\n  -H 'X-Note: a '\\''b'\\''' \\\n  \
         -H 'Content-Type: text/plain' \\\n  --data-raw $'ok\\xff'"
    );
}

#[test]
fn settings_become_options_and_parse_back() {
    let settings = AppSettings {
        follow_redirects: true,
        verify_ssl: false,
        proxy_url: "http://p:1".into(),
        ca_bundle: "ca.pem".into(),
        ..AppSettings::default()
    };
    let request = RequestTemplate {
        url: "http://h/".into(),
        body_type: BodyType::None,
        ..RequestTemplate::default()
    };
    let command = to_command(&prepare(&request, true).unwrap(), &settings);
    let options = template(&command).options;
    assert_eq!(options.follow_redirects, Some(true));
    assert_eq!(options.verify_ssl, Some(false));
    assert_eq!(options.proxy_url.as_deref(), Some("http://p:1"));
    assert_eq!(options.ca_bundle.as_deref(), Some("ca.pem"));
}

/// Generated commands parse back to a request that prepares identically.
#[test]
fn generated_commands_round_trip() {
    let base = RequestTemplate {
        url: "https://h/items?q=a%20b".into(),
        headers: vec![ParamItem::new("X-Trace", "on")],
        ..RequestTemplate::default()
    };
    let mut multipart = RequestTemplate {
        method: "PUT".into(),
        body_type: BodyType::Multipart,
        ..base.clone()
    };
    multipart.set_multipart_fields(&[
        FormItem::text("note", "it's"),
        FormItem {
            content_type: "text/csv".into(),
            filename: "x.csv".into(),
            ..FormItem::file("upload", "/tmp/a b.csv")
        },
    ]);
    let mut form = RequestTemplate {
        method: "POST".into(),
        body_type: BodyType::Urlencoded,
        ..base.clone()
    };
    form.set_form_fields(&[ParamItem::new("a", "1 2"), ParamItem::new("b", "&")]);
    let cases = [
        RequestTemplate {
            body_type: BodyType::None,
            ..base.clone()
        },
        RequestTemplate {
            method: "POST".into(),
            body: br#"{"a": [1, "x"]}"#.to_vec(),
            body_type: BodyType::Json,
            auth: RequestAuth::basic("u", "p w"),
            ..base.clone()
        },
        RequestTemplate {
            method: "PATCH".into(),
            body: b"line 1\nline 2".to_vec(),
            body_type: BodyType::Text,
            cookies: vec![ParamItem::new("s", "1")],
            auth: RequestAuth::bearer("t"),
            ..base.clone()
        },
        RequestTemplate {
            method: "DELETE".into(),
            body_type: BodyType::None,
            auth: RequestAuth::digest("u", "p"),
            ..base.clone()
        },
        RequestTemplate {
            method: "POST".into(),
            body: b"/tmp/body.bin".to_vec(),
            body_type: BodyType::Binary,
            ..base.clone()
        },
        multipart,
        form,
    ];
    let settings = AppSettings::default();
    for case in cases {
        let prepared = prepare(&case, true).unwrap();
        let command = to_command(&prepared, &settings);
        let back = template(&command);
        assert_eq!(prepare(&back, true).unwrap(), prepared, "{command}");
    }
}

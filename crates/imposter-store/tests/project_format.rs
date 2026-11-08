//! The collections folder's file format: every field written as a person reads it, the
//! same project read back, an edit confined to its own lines, names turned into file
//! names, and every malformed folder refused naming its file.
#![cfg(test)]
#![allow(clippy::disallowed_methods)] // reads and writes fixtures

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use imposter_core::capture::Capture;
use imposter_core::check::{Check, Op};
use imposter_core::model::{
    BodyType, EnvColor, FormItem, Options, ParamItem, RequestAuth, RequestTemplate, SavedValues,
    Scripts, Variables,
};
use imposter_core::select::Selector;
use imposter_store::files::format::{
    Collection, Environment, Folder, Globals, Project, Request, read, read_dir, render, slug,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/project")
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn request(id: &str, position: i64, template: RequestTemplate) -> Request {
    Request {
        id: id.to_owned(),
        template: RequestTemplate {
            position,
            ..template
        },
    }
}

/// One of everything a request, folder, collection and environment can hold.
fn sample() -> Project {
    let mut saved = SavedValues::default();
    saved.set("user", ["ann".to_owned(), "bob".to_owned()]);
    let mut login = RequestTemplate {
        name: "Log in".into(),
        description: "Exchanges the password for a token.".into(),
        method: "POST".into(),
        url: "{{base}}/login".into(),
        headers: vec![
            ParamItem::new("Accept", "application/json"),
            ParamItem {
                is_enabled: false,
                ..ParamItem::new("X-Debug", "1")
            },
        ],
        body_type: BodyType::Json,
        body: b"{\n  \"user\": \"{{user}}\",\n  \"password\": \"{{secret:password}}\"\n}".to_vec(),
        checks: vec![Check::new(Selector::Status, Op::Equals, "200")],
        captures: vec![Capture {
            secret: true,
            ..Capture::new("token", Selector::Json("$.token".into()))
        }],
        scripts: Scripts {
            before: String::new(),
            after: "test(\"has a token\", response.path(\"$.token\") != ());\nprint(\"ok\");"
                .into(),
        },
        ..RequestTemplate::default()
    };
    login.auth = RequestAuth::default();
    let mut upload = RequestTemplate {
        name: "Upload avatar".into(),
        method: "PUT".into(),
        url: "{{base}}/users/{{id}}/avatar?size=large&a-rather-long-query-parameter-name=and-a-long-value-that-goes-past-eighty-columns".into(),
        auth: RequestAuth::bearer("{{secret:token}}"),
        body_type: BodyType::Multipart,
        options: Options {
            timeout: Some(12.5),
            follow_redirects: Some(false),
            ..Options::default()
        },
        ..RequestTemplate::default()
    };
    upload.set_multipart_fields(&[
        FormItem::text("note", "hello"),
        FormItem {
            content_type: "image/png".into(),
            ..FormItem::file("avatar", "fixtures/avatar.png")
        },
    ]);
    let mut form = RequestTemplate {
        name: "Search".into(),
        method: "POST".into(),
        url: "{{base}}/search".into(),
        body_type: BodyType::Urlencoded,
        queries: vec![ParamItem::new("page", "1")],
        cookies: vec![ParamItem::new("session", "{{secret:session}}")],
        ..RequestTemplate::default()
    };
    form.set_form_fields(&[ParamItem::new("q", "shoes")]);
    let raw = RequestTemplate {
        name: "Raw bytes".into(),
        body_type: BodyType::Plain,
        body: vec![0xff, 0x00, 0x41],
        ..RequestTemplate::default()
    };
    let file = RequestTemplate {
        name: "Send file".into(),
        method: "POST".into(),
        body_type: BodyType::Binary,
        body: b"fixtures/data.bin".to_vec(),
        ..RequestTemplate::default()
    };
    let health = RequestTemplate {
        name: "Health".into(),
        url: "{{base}}/health".into(),
        body_type: BodyType::None,
        ..RequestTemplate::default()
    };
    Project {
        globals: Some(Globals {
            share_values: true,
            variables: vars(&[("region", "eu"), ("user_agent", "imposter-tests")]),
            saved_values: SavedValues::default(),
        }),
        environments: vec![
            Environment {
                id: "env-dev".into(),
                name: "Dev".into(),
                color: None,
                share_values: true,
                variables: vars(&[("base", "http://localhost:8080"), ("user", "ann")]),
                saved_values: saved.clone(),
            },
            Environment {
                id: "env-prod".into(),
                name: "Prod".into(),
                color: Some(EnvColor::Red),
                share_values: false,
                variables: vars(&[("base", "https://api.example.com"), ("user", "ops")]),
                saved_values: saved,
            },
        ],
        collections: vec![Collection {
            id: "col-shop".into(),
            name: "Shop API".into(),
            description: "The shop's public API.".into(),
            variables: vars(&[
                ("base", "http://localhost:8080"),
                ("flag", "true"),
                ("n", "1"),
            ]),
            saved_values: SavedValues::default(),
            scripts: Scripts {
                before: "request.set_header(\"x-run\", uuid());".into(),
                after: String::new(),
            },
            requests: vec![
                request("req-health", 0, health),
                request("req-login", 1, login),
            ],
            folders: vec![Folder {
                id: "fol-users".into(),
                name: "Users & Avatars".into(),
                position: 2,
                description: String::new(),
                scripts: Scripts::default(),
                requests: vec![
                    request("req-upload", 0, upload),
                    request("req-search", 1, form),
                    request("req-raw", 2, raw),
                    request("req-file", 3, file),
                ],
                folders: vec![Folder {
                    id: "fol-empty".into(),
                    name: "Empty".into(),
                    position: 4,
                    ..Folder::default()
                }],
            }],
        }],
    }
}

fn golden() -> BTreeMap<String, Vec<u8>> {
    read_dir(&fixtures()).unwrap()
}

#[test]
fn a_project_renders_to_the_golden_folder_and_reads_back() {
    let files = render(&sample());
    let expected = golden();
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in &files {
        assert_eq!(
            String::from_utf8_lossy(bytes),
            String::from_utf8_lossy(&expected[path]),
            "{path}"
        );
    }
    let mut back = read(&files).unwrap();
    let mut want = sample();
    // An environment that does not share its values keeps only the names on disk.
    want.environments[1].variables = vars(&[("base", ""), ("user", "")]);
    want.environments[1].saved_values = SavedValues::default();
    back.environments.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(back, want);
    let all = String::from_utf8(files.values().flatten().copied().collect()).unwrap_or_default();
    assert!(
        !all.contains("https://api.example.com"),
        "Prod's values stay local"
    );
}

fn changed_lines(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (path, bytes) in after {
        let old = before
            .get(path)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let new = String::from_utf8_lossy(bytes).into_owned();
        if old != new {
            let old: Vec<&str> = old.lines().collect();
            for (i, line) in new.lines().enumerate() {
                if old.get(i) != Some(&line) {
                    out.push(format!("{path}: {line}"));
                }
            }
        }
    }
    out
}

#[test]
fn an_edit_changes_only_its_own_lines() {
    let before = render(&sample());
    let mut project = sample();
    let login = &mut project.collections[0].requests[1].template;
    login.checks[0].expected = "201".into();
    let after = render(&project);
    assert_eq!(
        changed_lines(&before, &after),
        ["shop-api/log-in.yaml:   expected: \"201\""]
    );

    // A reorder changes positions, not file names.
    let mut project = sample();
    let requests = &mut project.collections[0].requests;
    requests[0].template.position = 1;
    requests[1].template.position = 0;
    let after = render(&project);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    assert!(
        changed_lines(&before, &after)
            .iter()
            .all(|l| l.ends_with("position: 0") || l.ends_with("position: 1"))
    );
}

#[test]
fn names_become_portable_file_names() {
    assert_eq!(slug("Shop API", "x"), "shop-api");
    assert_eq!(slug("  Users & Avatars!! ", "x"), "users-avatars");
    assert_eq!(slug("Пользователи", "request"), "request");
    assert_eq!(slug(&"a".repeat(80), "x").len(), 60);

    let mut project = sample();
    let collection = &mut project.collections[0];
    for (id, name) in [
        ("a", "Folder"),
        ("b", "Imposter"),
        ("c", "Health"),
        ("d", "health!"),
    ] {
        collection.requests.push(request(
            id,
            9,
            RequestTemplate {
                name: name.into(),
                ..RequestTemplate::default()
            },
        ));
    }
    project.collections.push(Collection {
        id: "col-env".into(),
        name: "Environments".into(),
        ..Collection::default()
    });
    let files = render(&project);
    for path in [
        "shop-api/folder-2.yaml",
        "shop-api/imposter-2.yaml",
        "shop-api/health-2.yaml",
        "shop-api/health-3.yaml",
        "environments-2/imposter.yaml",
    ] {
        assert!(files.contains_key(path), "{path} in {:?}", files.keys());
    }
    assert_eq!(read(&files).unwrap().collections.len(), 2);
}

/// One way to break a folder's files.
type Breaks = dyn Fn(&mut BTreeMap<String, Vec<u8>>);

#[test]
fn a_malformed_folder_is_refused_naming_the_file() {
    let good = render(&sample());
    let cases: Vec<(&str, Box<Breaks>)> = vec![
        (
            "shop-api/health.yaml",
            Box::new(|f| append(f, "shop-api/health.yaml", "extra: 1\n")),
        ),
        (
            "shop-api/log-in.yaml",
            Box::new(|f| replace(f, "shop-api/log-in.yaml", "req-login", "req-health")),
        ),
        (
            "shop-api/users-avatars/empty",
            Box::new(|f| {
                f.remove("shop-api/users-avatars/empty/folder.yaml");
                f.insert(
                    "shop-api/users-avatars/empty/x.yaml".into(),
                    b"id: z\nname: Z\n".to_vec(),
                );
            }),
        ),
        (
            "shop-api/imposter.yaml",
            Box::new(|f| replace(f, "shop-api/imposter.yaml", "imposter-project", "imposter")),
        ),
        (
            "shop-api/imposter.yaml",
            Box::new(|f| replace(f, "shop-api/imposter.yaml", "version: 1", "version: 2")),
        ),
        (
            "environments/dev.yaml",
            Box::new(|f| replace(f, "environments/dev.yaml", "name: Dev", "name: ''")),
        ),
        (
            "shop-api/health.yaml",
            Box::new(|f| {
                f.insert("shop-api/health.yaml".into(), b"id: [unclosed\n".to_vec());
            }),
        ),
    ];
    for (path, break_it) in cases {
        let mut files = good.clone();
        break_it(&mut files);
        let error = read(&files).unwrap_err();
        assert!(error.to_string().starts_with(path), "{path}: {error}");
    }
    let mut files = good.clone();
    append(&mut files, "globals.yaml", "extra: 1\n");
    let error = read(&files).unwrap_err();
    assert!(error.to_string().starts_with("globals.yaml"), "{error}");
    // Others' files beside the collections are not ours to read.
    let mut files = good;
    files.insert("README.yaml".into(), b"anything".to_vec());
    files.insert("docs/notes.yaml".into(), b"anything: [".to_vec());
    assert!(read(&files).is_ok());
}

fn append(files: &mut BTreeMap<String, Vec<u8>>, path: &str, text: &str) {
    files
        .get_mut(path)
        .unwrap()
        .extend_from_slice(text.as_bytes());
}

fn replace(files: &mut BTreeMap<String, Vec<u8>>, path: &str, from: &str, to: &str) {
    let text = String::from_utf8(files[path].clone()).unwrap();
    assert!(text.contains(from), "{path} has no {from:?}");
    files.insert(path.into(), text.replacen(from, to, 1).into_bytes());
}

#[test]
fn reading_a_directory_skips_hidden_entries_and_other_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/config.yaml"), "x").unwrap();
    std::fs::write(dir.path().join("README.md"), "x").unwrap();
    std::fs::create_dir_all(dir.path().join("api")).unwrap();
    std::fs::write(dir.path().join("api/imposter.yaml"), "x").unwrap();
    let files = read_dir(dir.path()).unwrap();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["api/imposter.yaml"]);
    assert!(read_dir(&dir.path().join("missing")).unwrap().is_empty());
}

#[test]
fn globals_that_are_not_shared_keep_only_their_names_in_the_file() {
    let mut project = sample();
    if let Some(globals) = &mut project.globals {
        globals.share_values = false;
    }
    let files = render(&project);
    let text = String::from_utf8(files["globals.yaml"].clone()).unwrap();
    assert_eq!(
        text,
        "share_values: false\nvariables:\n  region: \"\"\n  user_agent: \"\"\n"
    );
    let back = read(&files).unwrap().globals.unwrap();
    assert!(!back.share_values);
    assert_eq!(back.variables, vars(&[("region", ""), ("user_agent", "")]));
    assert!(
        read(&BTreeMap::new()).unwrap().globals.is_none(),
        "no file, no globals"
    );
}

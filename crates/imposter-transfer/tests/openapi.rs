//! OpenAPI import: one request per operation, generated once (see AGENTS.md).
#![cfg(test)]

use std::fmt::Write as _;
use std::path::Path;

use imposter_core::model::{
    BodyType, FormItem, FormKind, ParamItem, RequestAuth, RequestTemplate, Variables,
};
use imposter_transfer::native::NativeDocument;
use imposter_transfer::openapi::{OpenApiError, decode, is_openapi, parse_yaml};
use serde_json::{Value, json};

fn fixture() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/openapi/petstore.yaml");
    parse_yaml(&std::fs::read(path).unwrap()).unwrap()
}

fn vars(pairs: &[(&str, &str)]) -> Variables {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn row(name: &str, value: &str, enabled: bool) -> ParamItem {
    ParamItem {
        is_enabled: enabled,
        ..ParamItem::new(name, value)
    }
}

fn body(request: &RequestTemplate) -> String {
    String::from_utf8(request.body.clone()).unwrap()
}

fn request<'a>(document: &'a NativeDocument, name: &str) -> &'a RequestTemplate {
    document
        .requests()
        .into_iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no request {name:?}"))
}

/// A 3.1 document with one server and the given paths and components.
fn spec(paths: Value, components: Value) -> Value {
    let mut document = json!({
        "openapi": "3.1.0",
        "info": {"title": "T"},
        "servers": [{"url": "https://api.test"}],
    });
    document["paths"] = paths;
    document["components"] = components;
    document
}

fn single(paths: Value, components: Value) -> (RequestTemplate, Vec<String>, Variables) {
    let (document, warnings) = decode(&spec(paths, components)).unwrap();
    let requests = document.requests();
    assert_eq!(requests.len(), 1);
    (
        requests[0].clone(),
        warnings,
        document.collection.variables.clone(),
    )
}

#[test]
fn a_yaml_document_becomes_folders_requests_and_variables() {
    let (document, warnings) = decode(&fixture()).unwrap();
    let collection = &document.collection;
    assert_eq!(collection.name.as_str(), "Pet Store");
    assert_eq!(collection.description, "Pets and their owners.");
    assert_eq!(
        collection.variables,
        vars(&[
            ("baseUrl", "https://eu.pets.example/v1"),
            ("api_key", ""),
            ("petId", "42"),
            ("token", ""),
        ]),
        "server variables take their defaults; secrets start empty"
    );
    let folders: Vec<_> = collection
        .folders
        .iter()
        .map(|f| (f.name.as_str(), f.description.as_str(), f.position))
        .collect();
    assert_eq!(
        folders,
        [("store", "Orders", 0), ("pets", "Everything about pets", 1)],
        "declared tag order wins over the order of appearance"
    );
    let pets: Vec<_> = collection.folders[1]
        .requests
        .iter()
        .map(|r| (r.method.as_str(), r.name.as_str(), r.position))
        .collect();
    assert_eq!(
        pets,
        [
            ("GET", "List pets", 0),
            ("POST", "Create a pet", 1),
            ("GET", "showPetById", 2),
            ("DELETE", "Delete a pet", 3),
            ("PUT", "Upload a photo", 4),
        ]
    );
    let root: Vec<_> = collection
        .requests
        .iter()
        .map(|r| (r.name.as_str(), r.position))
        .collect();
    assert_eq!(root, [("Upload owner documents", 2), ("Health", 3)]);
    assert_eq!(
        warnings,
        [
            "Using server https://eu.pets.example/v1; to use another (http://localhost:8080/v1), change the baseUrl collection variable",
            "Schema #/components/schemas/Pet is recursive; example bodies include it once",
            "Path parameters without an example need collection variables before sending: ownerId",
        ]
    );
}

#[test]
fn parameters_land_in_their_rows_and_only_required_ones_are_enabled() {
    let (document, _) = decode(&fixture()).unwrap();
    let list = request(&document, "List pets");
    assert_eq!(list.url, "{{baseUrl}}/pets");
    assert_eq!(
        list.queries,
        [
            row("limit", "20", true),
            row("tags", "cat", false),
            row("tags", "dog", false),
        ]
    );
    assert_eq!(
        list.headers,
        [
            row("X-Trace", "", false),
            row("X-API-Key", "{{api_key}}", true)
        ],
        "Accept is not a parameter; the API key comes from the default security"
    );
    assert_eq!(list.body_type, BodyType::None);

    let show = request(&document, "showPetById");
    assert_eq!(show.url, "{{baseUrl}}/pets/{{petId}}");
    assert_eq!(show.description, "One pet.");

    let documents = request(&document, "Upload owner documents");
    assert_eq!(documents.url, "{{baseUrl}}/owners/{{ownerId}}/documents");
    assert_eq!(documents.cookies, [row("session", "abc", false)]);
}

#[test]
fn security_requirements_override_the_default() {
    let (document, _) = decode(&fixture()).unwrap();
    let delete = request(&document, "Delete a pet");
    assert_eq!(delete.auth, RequestAuth::bearer("{{token}}"));
    assert!(delete.headers.iter().all(|h| h.name != "X-API-Key"));
    assert_eq!(delete.description, "Deprecated.");
    let health = request(&document, "Health");
    assert_eq!(health.auth, RequestAuth::default());
    assert!(health.headers.is_empty(), "an empty requirement means none");
}

#[test]
fn bodies_follow_the_preferred_media_type() {
    let (document, _) = decode(&fixture()).unwrap();
    let create = request(&document, "Create a pet");
    assert_eq!(create.body_type, BodyType::Json);
    let created: Value = serde_json::from_str(&body(create)).unwrap();
    assert_eq!(
        created,
        json!({"name": "Rex", "kind": "cat", "born": "2024-01-01"}),
        "allOf merged, readOnly left out, the recursive parent included once"
    );
    assert!(body(create).contains("\n  \"name\""), "pretty-printed");

    let order = request(&document, "Place an order");
    assert_eq!(order.body_type, BodyType::Urlencoded);
    assert_eq!(
        order.form_fields().unwrap(),
        [row("petId", "7", true), row("quantity", "1", true)],
        "a form is preferred to XML; minimum is a sensible number"
    );

    let photo = request(&document, "Upload a photo");
    assert_eq!(photo.body_type, BodyType::Binary);
    assert!(photo.body.is_empty(), "the user picks the file");
    assert!(
        photo
            .headers
            .contains(&row("Content-Type", "image/png", true))
    );

    let documents = request(&document, "Upload owner documents");
    assert_eq!(documents.body_type, BodyType::Multipart);
    assert_eq!(
        documents.multipart_fields().unwrap(),
        [FormItem::text("note", "string"), FormItem::file("file", "")]
    );
}

#[test]
fn only_openapi_3_is_accepted() {
    assert!(matches!(
        decode(&json!({"swagger": "2.0", "paths": {}})),
        Err(OpenApiError::Swagger)
    ));
    for version in [json!("4.0.0"), json!("2.0"), json!(null), json!("3.10.0")] {
        let mut document = spec(json!({"/": {"get": {}}}), json!({}));
        document["openapi"] = version;
        assert!(matches!(decode(&document), Err(OpenApiError::Version(_))));
    }
    for version in [json!("3.0.0"), json!("3.1.1"), json!("3.2.0"), json!(3.0)] {
        let mut document = spec(json!({"/": {"get": {}}}), json!({}));
        document["openapi"] = version.clone();
        assert!(decode(&document).is_ok(), "{version}");
    }
    assert!(matches!(
        decode(&spec(json!({}), json!({}))),
        Err(OpenApiError::NoOperations)
    ));
    assert!(is_openapi(&json!({"openapi": "3.1.0"})));
    assert!(is_openapi(&json!({"swagger": "2.0"})));
    assert!(!is_openapi(&json!({"info": {}})));
}

#[test]
fn names_fall_back_to_the_operation_id_then_method_and_path() {
    let (document, _) = decode(&spec(
        json!({"/a": {
            "get": {"operationId": "getA"},
            "put": {"summary": "  "},
        }}),
        json!({}),
    ))
    .unwrap();
    let names: Vec<_> = document.requests().iter().map(|r| r.name.clone()).collect();
    assert_eq!(names, ["getA", "PUT /a"]);
    let mut untitled = spec(json!({"/": {"get": {}}}), json!({}));
    untitled["info"] = json!({});
    let (document, _) = decode(&untitled).unwrap();
    assert_eq!(document.collection.name.as_str(), "Imported API");
}

#[test]
fn servers_without_a_full_address_are_reported() {
    let mut document = spec(json!({"/": {"get": {}}}), json!({}));
    document.as_object_mut().unwrap().remove("servers");
    let (document, warnings) = decode(&document).unwrap();
    assert_eq!(document.collection.variables, vars(&[("baseUrl", "")]));
    assert_eq!(
        warnings,
        ["No server URL; set the baseUrl collection variable before sending"]
    );

    let mut relative = spec(json!({"/": {"get": {}}}), json!({}));
    relative["servers"] = json!([{"url": "/api/v3"}]);
    let (_, warnings) = decode(&relative).unwrap();
    assert_eq!(
        warnings,
        [
            "Server URL \"/api/v3\" is relative; put the full address in the baseUrl collection variable"
        ]
    );
}

#[test]
fn path_and_operation_servers_replace_the_base_url() {
    let (document, _) = decode(&spec(
        json!({"/a": {
            "servers": [{"url": "https://path.test/"}],
            "get": {"operationId": "path"},
            "put": {"operationId": "own", "servers": [{"url": "https://op.test"}]},
        }}),
        json!({}),
    ))
    .unwrap();
    assert_eq!(request(&document, "path").url, "https://path.test/a");
    assert_eq!(request(&document, "own").url, "https://op.test/a");
}

#[test]
fn query_values_follow_style_and_explode() {
    let (request, _, _) = single(
        json!({"/": {"get": {"parameters": [
            {"name": "ids", "in": "query", "required": true, "explode": false, "example": [1, 2]},
            {"name": "pipes", "in": "query", "required": true, "style": "pipeDelimited",
             "explode": false, "example": ["a", "b"]},
            {"name": "filter", "in": "query", "required": true, "style": "deepObject",
             "example": {"color": "red"}},
            {"name": "point", "in": "query", "required": true, "example": {"x": 1, "y": 2}},
            {"name": "plain", "in": "query", "required": true, "schema": {"enum": ["a", "b"]}},
            {"name": "empty", "in": "query", "required": true, "schema": {"type": "string"}},
            {"name": "", "in": "query"},
            {"name": "odd", "in": "body"},
        ]}}}),
        json!({}),
    );
    assert_eq!(
        request.queries,
        [
            row("ids", "1,2", true),
            row("pipes", "a|b", true),
            row("filter[color]", "red", true),
            row("x", "1", true),
            row("y", "2", true),
            row("plain", "a", true),
            row("empty", "", true),
        ]
    );
}

#[test]
fn parameter_examples_come_from_examples_and_content() {
    let (request, _, variables) = single(
        json!({"/items/{id}": {"get": {"parameters": [
            {"name": "id", "in": "path", "required": true,
             "examples": {"one": {"$ref": "#/components/examples/Id"}}},
            {"name": "X-Tags", "in": "header", "required": true, "example": ["a", "b"]},
            {"name": "q", "in": "query", "required": true,
             "content": {"application/json": {"example": {"k": 1}}}},
            {"name": "r", "in": "query", "required": true,
             "content": {"application/json": {"examples": {"x": {"value": "v"}}}}},
        ]}}}),
        json!({"examples": {"Id": {"value": 7}}}),
    );
    assert_eq!(variables.get("id").map(String::as_str), Some("7"));
    assert_eq!(request.headers, [row("X-Tags", "a,b", true)]);
    assert_eq!(
        request.queries,
        [row("k", "1", true), row("r", "v", true)],
        "content examples serialize like schema-typed ones"
    );
}

#[test]
fn operation_parameters_replace_path_level_ones() {
    let (request, _, _) = single(
        json!({"/": {
            "parameters": [{"name": "v", "in": "query", "example": "path"}],
            "get": {"parameters": [{"name": "v", "in": "query", "required": true, "example": "op"}]},
        }}),
        json!({}),
    );
    assert_eq!(request.queries, [row("v", "op", true)]);
}

#[test]
fn the_first_alternative_this_app_supports_is_chosen() {
    let schemes = json!({"securitySchemes": {
        "oauth": {"type": "oauth2", "flows": {}},
        "basic": {"type": "http", "scheme": "Basic"},
        "digest": {"type": "http", "scheme": "digest"},
        "query": {"type": "apiKey", "in": "query", "name": "key"},
        "cookie": {"type": "apiKey", "in": "cookie", "name": "sid"},
        "hoba": {"type": "http", "scheme": "hoba"},
        "mtls": {"type": "mutualTLS"},
        "ref": {"$ref": "#/components/securitySchemes/basic"},
    }});
    let (request, warnings, variables) = single(
        json!({"/": {"get": {"security": [{"oauth": []}, {"basic": [], "query": [], "cookie": []}]}}}),
        schemes.clone(),
    );
    assert_eq!(
        request.auth,
        RequestAuth::basic("{{username}}", "{{password}}")
    );
    assert_eq!(request.queries, [row("key", "{{query}}", true)]);
    assert_eq!(request.cookies, [row("sid", "{{cookie}}", true)]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        variables.keys().collect::<Vec<_>>(),
        ["baseUrl", "username", "password", "query", "cookie"]
    );

    let (request, warnings, _) = single(
        json!({"/": {"get": {"security": [{"oauth": [], "digest": [], "hoba": [], "mtls": [], "missing": []}]}}}),
        schemes.clone(),
    );
    assert_eq!(
        request.auth,
        RequestAuth::bearer("{{token}}"),
        "OAuth sends a pasted token"
    );
    assert_eq!(
        warnings,
        [
            "Security scheme \"oauth\": oauth2 flows are not supported; requests send the token collection variable as a bearer token",
            "GET /: only one authentication scheme is used (oauth, not digest)",
            "Security scheme \"hoba\": HTTP hoba authentication is not supported",
            "Security scheme \"mtls\": set the client certificate in Settings",
            "Security scheme \"missing\" is not defined",
        ]
    );

    let (request, _, _) = single(json!({"/": {"get": {"security": [{"ref": []}]}}}), schemes);
    assert_eq!(
        request.auth,
        RequestAuth::basic("{{username}}", "{{password}}"),
        "schemes may be references"
    );
}

#[test]
fn references_outside_the_document_or_missing_are_reported() {
    let (request, warnings, _) = single(
        json!({"/": {"post": {
            "parameters": [{"$ref": "common.yaml#/Limit"}, {"$ref": "#/components/parameters/Nope"}],
            "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Loop"}}}},
        }}}),
        json!({"schemas": {"Loop": {"$ref": "#/components/schemas/Loop"}}}),
    );
    assert_eq!(request.queries, []);
    assert_eq!(
        warnings,
        [
            "External reference common.yaml#/Limit is not followed; its values stay empty",
            "Reference #/components/parameters/Nope does not exist",
            "Schema #/components/schemas/Loop is recursive; example bodies include it once",
        ]
    );

    let (_, warnings) = decode(&spec(
        json!({"/": {"$ref": "#/components/pathItems/A"}, "/ok": {"get": {}}}),
        json!({"pathItems": {"A": {"$ref": "#/components/pathItems/A"}}}),
    ))
    .unwrap();
    assert_eq!(warnings, ["/: a reference chain is too long or loops"]);
}

#[test]
fn schemas_generate_typed_examples() {
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/vnd.api+json": {"schema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "format": "uuid"},
                "when": {"type": "string", "format": "date-time"},
                "count": {"type": "integer"},
                "ratio": {"type": ["number", "null"], "minimum": 0.5},
                "on": {"type": "boolean"},
                "tags": {"type": "array", "items": {"type": "string", "examples": ["x"]}},
                "kind": {"oneOf": [{"const": "a"}, {"const": "b"}]},
                "any": {},
                "nothing": {"type": "null"},
                "secret": {"$ref": "#/components/schemas/Secret"},
                "meta": {"properties": {"v": {"default": 1}}},
            },
        }}}}}}}),
        json!({"schemas": {"Secret": {"type": "string", "readOnly": true}}}),
    );
    assert_eq!(request.body_type, BodyType::Json);
    assert_eq!(
        request.headers,
        [row("Content-Type", "application/vnd.api+json", true)],
        "vendor JSON types keep their media type"
    );
    let value: Value = serde_json::from_str(&body(&request)).unwrap();
    assert_eq!(
        value,
        json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "when": "2024-01-01T00:00:00Z",
            "count": 0,
            "ratio": 0.5,
            "on": true,
            "tags": ["x"],
            "kind": "a",
            "any": null,
            "nothing": null,
            "meta": {"v": 1},
        })
    );
}

#[test]
fn media_examples_win_over_schemas() {
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/json": {
            "schema": {"type": "object"},
            "examples": {"first": {"value": {"from": "examples"}}},
        }}}}}}),
        json!({}),
    );
    assert_eq!(
        serde_json::from_str::<Value>(&body(&request)).unwrap(),
        json!({"from": "examples"})
    );
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/json": {
            "example": "{\"raw\": true}",
        }}}}}}),
        json!({}),
    );
    assert_eq!(
        body(&request),
        "{\"raw\": true}",
        "JSON in a string is kept"
    );
}

#[test]
fn text_bodies_keep_string_examples_and_their_media_type() {
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/xml": {
            "example": "<a/>",
        }}}}}}),
        json!({}),
    );
    assert_eq!(request.body_type, BodyType::Plain);
    assert_eq!(body(&request), "<a/>");
    assert_eq!(
        request.headers,
        [row("Content-Type", "application/xml", true)]
    );
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"text/plain": {"schema": {}}}}}}}),
        json!({}),
    );
    assert_eq!(request.body_type, BodyType::Plain);
    assert!(request.headers.is_empty(), "text/plain is the default");
}

#[test]
fn forms_without_fields_send_no_body() {
    for media in ["application/x-www-form-urlencoded", "multipart/form-data"] {
        let (request, _, _) = single(
            json!({"/": {"get": {"requestBody": {"content": {media: {
                "schema": {"type": "object", "additionalProperties": false},
            }}}}}}),
            json!({}),
        );
        assert_eq!(request.body_type, BodyType::None, "{media}");
        assert_eq!(request.body, b"");
    }
}

#[test]
fn multipart_files_come_from_binary_schemas_and_encodings() {
    let (request, _, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"multipart/form-data": {
            "schema": {"allOf": [{"$ref": "#/components/schemas/Upload"}], "properties": {
                "avatar": {"type": "string", "contentMediaType": "image/png"},
                "meta": {"type": "object", "properties": {"a": {"type": "integer"}}},
                "pdf": {"type": "string"},
            }},
            "encoding": {"pdf": {"contentType": "application/pdf"}},
        }}}}}}),
        json!({"schemas": {"Upload": {"properties": {
            "files": {"type": "array", "items": {"type": "string", "format": "binary"}},
        }}}}),
    );
    let fields = request.multipart_fields().unwrap();
    let kinds: Vec<_> = fields.iter().map(|f| (f.name.as_str(), f.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("files", FormKind::File),
            ("avatar", FormKind::File),
            ("meta", FormKind::Text),
            ("pdf", FormKind::File),
        ]
    );
    assert_eq!(fields[2].value, r#"{"a":0}"#, "objects are sent as JSON");
}

#[test]
fn deep_or_huge_schemas_are_cut_short_with_a_warning() {
    let mut schema = json!({"type": "string"});
    for _ in 0..20 {
        schema = json!({"type": "object", "properties": {"next": schema}});
    }
    let (request, warnings, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/json": {"schema": schema}}}}}}),
        json!({}),
    );
    assert_eq!(
        warnings,
        ["POST /: the example body was cut short; the schema is too large or deep"]
    );
    assert!(body(&request).matches("next").count() <= 11);

    let wide: serde_json::Map<String, Value> = (0..100)
        .map(|i| {
            (
                format!("p{i}"),
                json!({"$ref": "#/components/schemas/Wide"}),
            )
        })
        .collect();
    let (request, warnings, _) = single(
        json!({"/": {"post": {"requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Root"}}}}}}}),
        json!({"schemas": {
            "Root": {"properties": wide},
            "Wide": {"properties": (0..100)
                .map(|i| (format!("q{i}"), json!({"type": "integer"})))
                .collect::<serde_json::Map<String, Value>>()},
        }}),
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(request.body.len() < 200_000);
}

#[test]
fn version_3_2_operations_are_imported_and_bad_methods_skipped() {
    let (document, warnings) = decode(&spec(
        json!({"/search": {
            "query": {"operationId": "search"},
            "additionalOperations": {"LINK": {"operationId": "link"}, "BAD METHOD": {}},
        }}),
        json!({}),
    ))
    .unwrap();
    let methods: Vec<_> = document
        .requests()
        .iter()
        .map(|r| r.method.clone())
        .collect();
    assert_eq!(methods, ["QUERY", "LINK"]);
    assert_eq!(
        warnings,
        ["BAD METHOD /search: \"BAD METHOD\" is not an HTTP method; skipped"]
    );
}

#[test]
fn callbacks_and_webhooks_are_reported() {
    let mut document = spec(
        json!({"/": {"post": {"callbacks": {"done": {}}}}}),
        json!({}),
    );
    document["webhooks"] = json!({"newPet": {}});
    let (_, warnings) = decode(&document).unwrap();
    assert_eq!(
        warnings,
        [
            "POST /: callbacks are not imported",
            "Webhooks are not imported"
        ]
    );
}

#[test]
fn reserved_headers_and_undeclared_path_names() {
    let (request, warnings, variables) = single(
        json!({"/a/{x}/{y}": {"get": {"parameters": [
            {"name": "Content-Type", "in": "header"},
            {"name": "authorization", "in": "header"},
            {"name": "y", "in": "path", "example": "why"},
        ]}}}),
        json!({}),
    );
    assert_eq!(request.headers, []);
    assert_eq!(variables.get("y").map(String::as_str), Some("why"));
    assert!(!variables.contains_key("x"), "an empty value would hide it");
    assert_eq!(
        warnings,
        ["Path parameters without an example need collection variables before sending: x"]
    );
}

#[test]
fn yaml_anchors_work_and_alias_bombs_are_refused() {
    let yaml = br#"
openapi: 3.1.0
info: {title: Anchors}
servers: [{url: "https://h"}]
x-shared: &limit {name: limit, in: query, required: true, example: 5}
paths:
  /a:
    get:
      parameters: [*limit]
    post:
      parameters: [*limit]
"#;
    let (document, _) = decode(&parse_yaml(yaml).unwrap()).unwrap();
    assert!(
        document
            .requests()
            .iter()
            .all(|r| r.queries == [row("limit", "5", true)])
    );

    let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
    for level in 1..12 {
        let previous = char::from(b'a' + level - 1);
        let name = char::from(b'a' + level);
        let refs = vec![format!("*{previous}"); 10].join(", ");
        writeln!(bomb, "{name}: &{name} [{refs}]").unwrap();
    }
    assert!(matches!(
        parse_yaml(bomb.as_bytes()),
        Err(OpenApiError::Yaml(_))
    ));
    assert!(matches!(
        parse_yaml(b"a: [unclosed"),
        Err(OpenApiError::Yaml(_))
    ));
}

#[test]
fn yaml_numbers_as_keys_and_versions_read_as_json() {
    let value = parse_yaml(b"openapi: 3.0\nresponses:\n  200: {description: ok}\n").unwrap();
    assert_eq!(value["responses"]["200"]["description"], "ok");
    assert!(value["openapi"].is_number());
}

#[test]
fn operations_check_for_their_lowest_documented_success() {
    use imposter_core::check::{Check, Op};
    use imposter_core::select::Selector;
    let status = |expected: &str| vec![Check::new(Selector::Status, Op::Equals, expected)];
    let responses = |codes: Value| {
        single(json!({"/x": {"get": {"responses": codes}}}), json!({}))
            .0
            .checks
    };
    assert_eq!(
        responses(json!({"404": {}, "201": {}, "200": {}, "default": {}})),
        status("200")
    );
    assert_eq!(responses(json!({"2XX": {}, "400": {}})), status("2xx"));
    assert_eq!(responses(json!({"2XX": {}, "204": {}})), status("204"));
    assert_eq!(
        responses(json!({"default": {}, "500": {}})),
        [],
        "no documented success, no guess"
    );
    let (template, _, _) = single(json!({"/x": {"get": {}}}), json!({}));
    assert_eq!(template.checks, []);
}

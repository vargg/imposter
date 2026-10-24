"""Generate golden fixtures from the Python reference app (~/dev/pystman).

Run from the Python checkout so its environment is used:

    cd ~/dev/pystman && uv run --frozen python ~/dev/poster/tests/fixtures/python/generate.py

Outputs (relative to tests/fixtures/):
- native/*.poster.json: documents encoded by the Python native codec.
- parity/decode.json: native documents and whether Python accepts them.
- parity/prepare.json: request preparation (URL, headers, body) or the error raised.
- parity/resolve.json: variable resolution of request templates or the error raised.
- parity/settings.json: application settings resolved against request options.
"""

import base64
import json
from pathlib import Path

from poster.adapters import native
from poster.adapters.http import prepare
from poster.dto.request import FormItem, Options, ParamItem, RequestAuth, RequestDTO, RequestTemplate
from poster.dto.settings import ApplicationSettings
from poster.dto.transfer import NativeDocument, TransferCollection, TransferEnvironment, TransferFolder
from poster.services.transfer import validate_templates
from poster.services.variable import VariableService

ROOT = Path(__file__).resolve().parent.parent


def items(*pairs, cls=ParamItem):
    return [cls(**pair) if isinstance(pair, dict) else cls(name=pair[0], value=pair[1]) for pair in pairs]


def form(*entries, cls=ParamItem) -> bytes:
    return json.dumps([item.model_dump() for item in items(*entries, cls=cls)]).encode()


def full_document() -> NativeDocument:
    requests = [
        RequestTemplate(
            name="Get users",
            description="List with ünïcode ✓",
            method="GET",
            url="https://{{host}}/users?page=1",
            queries=items(("page", "1"), {"name": "debug", "value": "", "is_enabled": False}, ("tag", "a"), ("tag", "b")),
            headers=items(("Accept", "application/json"), ("X-Dup", "1"), ("X-Dup", "2"), ("X-Empty", "")),
            cookies=items(("session", "{{token}}")),
            body_type="none",
        ),
        RequestTemplate(
            position=1,
            name="Create (json)",
            method="POST",
            url="https://{{host}}/users",
            auth=RequestAuth.bearer_token_auth("{{token}}"),
            body=b'{"name": "Ann", "emoji": "\xf0\x9f\x98\x80"}',
            body_type="json",
            options=Options(timeout=2.5, follow_redirects=False, download_limit=0, proxy_url=""),
        ),
        RequestTemplate(
            position=2,
            name="Plain",
            method="put",
            url="http://localhost:8080/text",
            auth=RequestAuth.basic_auth("user", "p@ss:word"),
            body=b"line 1\nline 2\r\n",
            body_type="plain",
        ),
        RequestTemplate(
            position=3,
            name="Text",
            method="PROPFIND",
            url="http://localhost/dav",
            auth=RequestAuth.digest_auth("dig", "est"),
            body=b"text body",
            body_type="text",
            options=Options(
                verify_ssl=False,
                attach_cookies=False,
                ca_bundle="/etc/ca.pem",
                certificate_path="{{cert}}",
                key_file="/keys/client.key",
                password="secret",
            ),
        ),
        RequestTemplate(
            position=4,
            name="Form data",
            method="POST",
            url="http://localhost/form",
            body=form(("a", "1"), ("a", "2"), {"name": "off", "value": "x", "is_enabled": False}),
            body_type="form-data",
        ),
        RequestTemplate(
            position=5,
            name="Urlencoded",
            method="POST",
            url="http://localhost/urlencoded",
            body=form(("q", "a b&c"), ("empty", "")),
            body_type="urlencoded",
        ),
        RequestTemplate(
            position=6,
            name="Multipart",
            method="POST",
            url="http://localhost/upload",
            body=form(
                {"name": "field", "value": "text value"},
                {"name": "json", "value": "{}", "content_type": "application/json"},
                {"name": "file", "value": "/tmp/{{file}}", "kind": "file", "filename": "custom.bin"},
                {"name": "off", "value": "/x", "kind": "file", "is_enabled": False},
                cls=FormItem,
            ),
            body_type="multipart",
        ),
        RequestTemplate(
            position=7,
            name="Binary",
            method="POST",
            url="http://localhost/binary",
            body=b"/data/{{file}}",
            body_type="binary",
        ),
        RequestTemplate(
            position=8,
            name="Leftover bytes",
            method="DELETE",
            url="http://localhost/x",
            body=bytes([0xFB, 0xFF, 0x00, 0x3E, 0x3F]),
            body_type="none",
        ),
    ]
    nested = TransferFolder(
        name="Nested",
        description="Inner folder",
        position=1,
        requests=[requests[4], requests[5]],
        folders=[TransferFolder(name="Deep", requests=[requests[6]])],
    )
    folder = TransferFolder(name="Folder", position=2, requests=[requests[2], requests[3]], folders=[nested])
    return NativeDocument(
        collection=TransferCollection(
            name="Full collection",
            description="Every field the native format carries",
            variables={"host": "api.example.com", "zeta": "last", "alpha": "first", "empty": ""},
            folders=[folder, TransferFolder(name="Empty folder")],
            requests=[requests[0], requests[1], requests[7], requests[8]],
        ),
        environments=[
            TransferEnvironment(name="Dev", variables={"host": "dev.local", "token": "dev-token"}),
            TransferEnvironment(name="Empty env"),
        ],
        globals={"token": "global-token", "file": "upload.bin"},
    )


def encode_document(document: NativeDocument) -> bytes:
    return native.encode(document)


def decode_cases() -> list[dict]:
    """Documents and whether Python's decode plus pre-import validation accepts them."""
    base = json.loads(native.encode(NativeDocument(collection=TransferCollection(name="C"))))

    def with_request(request: dict) -> dict:
        data = json.loads(json.dumps(base))
        data["collection"]["requests"] = [request]
        return data

    def b64(value: bytes) -> str:
        return base64.urlsafe_b64encode(value).decode()

    cases = {
        "minimal": base,
        "empty request object uses defaults": with_request({}),
        "unknown top-level field": {**base, "extra": 1},
        "unknown collection field": {**base, "collection": {**base["collection"], "extra": 1}},
        "unknown request field": with_request({"bogus": 1}),
        "unknown auth field": with_request({"auth": {"type": None, "oauth": None}}),
        "unknown option": with_request({"options": {"retries": 3}}),
        "unknown param field": with_request({"headers": [{"name": "a", "value": "b", "note": ""}]}),
        "version 2": {**base, "version": 2},
        "other format": {**base, "format": "postman"},
        "missing format uses default": {k: v for k, v in base.items() if k != "format"},
        "missing collection": {k: v for k, v in base.items() if k != "collection"},
        "empty collection name": {**base, "collection": {**base["collection"], "name": ""}},
        "empty folder name": {**base, "collection": {**base["collection"], "folders": [{"name": ""}]}},
        "empty environment name": {**base, "environments": [{"name": ""}]},
        "custom method token": with_request({"method": "PROPFIND"}),
        "lowercase method": with_request({"method": "get"}),
        "method with space": with_request({"method": "GE T"}),
        "empty method": with_request({"method": ""}),
        "non-ascii method": with_request({"method": "GÉT"}),
        "auth type without config": with_request({"auth": {"type": "basic"}}),
        "auth with config": with_request({"auth": {"type": "basic", "basic": {"username": "u", "password": "p"}}}),
        "unknown auth type": with_request({"auth": {"type": "oauth2"}}),
        "unknown body type": with_request({"body_type": "graphql"}),
        "form body not an array": with_request({"body_type": "urlencoded", "body": b64(b'{"a": 1}')}),
        "form body bad item": with_request({"body_type": "form-data", "body": b64(b'[{"name": "a"}]')}),
        "form body item with kind": with_request({"body_type": "urlencoded", "body": b64(b'[{"name": "a", "value": "b", "kind": "file"}]')}),
        "multipart item with kind": with_request({"body_type": "multipart", "body": b64(b'[{"name": "a", "value": "b", "kind": "file"}]')}),
        "multipart bad kind": with_request({"body_type": "multipart", "body": b64(b'[{"name": "a", "value": "b", "kind": "url"}]')}),
        "form body not json": with_request({"body_type": "multipart", "body": b64(b"not json")}),
        "empty form body": with_request({"body_type": "multipart", "body": ""}),
        "standard base64 alphabet": with_request({"body": "+/8APj8="}),
        "unpadded base64": with_request({"body": "-_8APj8"}),
        "invalid base64": with_request({"body": "a b"}),
        "zero timeout": with_request({"options": {"timeout": 0}}),
        "negative download limit": with_request({"options": {"download_limit": -1}}),
        "integer timeout": with_request({"options": {"timeout": 5}}),
        "null param value": with_request({"headers": [{"name": "a", "value": None}]}),
        "param without is_enabled": with_request({"headers": [{"name": "a", "value": "b"}]}),
        "duplicate variables keep the last": None,  # JSON text case, added below
    }
    out = []
    for name, data in cases.items():
        text = json.dumps(data, ensure_ascii=False) if data is not None else (
            '{"format": "poster", "version": 1, "collection": {"name": "C", "variables": {"a": "1", "a": "2"}}}'
        )
        try:
            document = native.decode(text.encode())
            validate_templates(document)
            accepted, detail = True, json.loads(native.encode(document))
        except Exception as error:  # noqa: BLE001 - the fixture records any rejection
            accepted, detail = False, type(error).__name__
        out.append({"name": name, "document": text, "accepted": accepted, "result": detail})
    return out


def dump(template: RequestTemplate) -> dict:
    return json.loads(template.model_dump_json())


def prepare_cases() -> list[dict]:
    def case(label, settings=None, **fields):
        return label, RequestTemplate(**fields), settings or {}

    cases = [
        case("url query kept when no rows", url="http://h/p?a=1&b=&c&a=2"),
        case("rows replace url query", url="http://h/p?ignored=1", queries=items(("a", "1"), ("b", "2"))),
        case(
            "disabled and nameless rows skipped",
            url="http://h/p?ignored=1",
            queries=items({"name": "x", "value": "1", "is_enabled": False}, ("", "v"), ("keep", "")),
        ),
        case(
            "all rows disabled still replace url query",
            url="http://h/p?ignored=1",
            queries=items({"name": "x", "value": "1", "is_enabled": False}),
        ),
        case("query encoding", url="http://h/p", queries=items(("a b", "~*!'()/é?&=+"), ("ключ", "значение"))),
        case("url query decoding", url="http://h/p?q=a+b%20c&bad=%zz&utf=%C3%A9&lat=%ff;x"),
        case("fragment dropped", url="http://h/p?a=1#frag"),
        case("scheme lowercased", url="HTTP://Example.COM:8080/Path"),
        case("path kept verbatim", url="http://h/a b/ü"),
        case("no path", url="http://h"),
        case("userinfo kept", url="http://u:p@h/x"),
        case("ipv6 host", url="http://[::1]:8080/x"),
        case("not http", url="ftp://h/x"),
        case("relative", url="/x"),
        case("missing host", url="http:///x"),
        case("host and port without scheme", url="localhost:8080/x"),
        case("empty url", url=""),
        case(
            "headers in order with duplicates and empties",
            url="http://h",
            headers=items(("A", "1"), ("B", "2"), ("A", "3"), ("Empty", ""), ("", "nameless"), {"name": "Off", "value": "x", "is_enabled": False}),
        ),
        case("auth conflicts with authorization header", url="http://h", auth=RequestAuth.bearer_token_auth("t"), headers=items(("authorization", "x"))),
        case("disabled authorization header does not conflict", url="http://h", auth=RequestAuth.bearer_token_auth("t"), headers=items({"name": "Authorization", "value": "x", "is_enabled": False})),
        case("cookies appended", url="http://h", cookies=items(("a", "1"), {"name": "b", "value": "2", "is_enabled": False}, ("c", ""))),
        case("cookies conflict with cookie header", url="http://h", cookies=items(("a", "1")), headers=items(("COOKIE", "x"))),
        case("cookies ignored when not attached", {"attach_cookies": False}, url="http://h", cookies=items(("a", "1")), headers=items(("Cookie", "x"))),
        case("json body adds content type", url="http://h", body=b'{"a": 1}', body_type="json"),
        case("json body keeps explicit content type", url="http://h", body=b"{}", body_type="json", headers=items(("content-TYPE", "application/vnd+json"))),
        case("plain body", url="http://h", body=b"hi", body_type="plain"),
        case("text body", url="http://h", body=b"hi", body_type="text"),
        case("none body drops bytes", url="http://h", body=b"leftover", body_type="none"),
        case("empty json body", url="http://h", body=b"", body_type="json"),
        case(
            "urlencoded body",
            url="http://h",
            body=form(("a", "1"), ("a", "x y"), ("", "nameless"), {"name": "off", "value": "1", "is_enabled": False}),
            body_type="urlencoded",
        ),
        case("form-data body", url="http://h", body=form(("k", "v/é")), body_type="form-data"),
        case("empty urlencoded body", url="http://h", body=b"", body_type="urlencoded"),
        case("multipart left for transport", url="http://h", body=form(("k", "v"), cls=FormItem), body_type="multipart"),
        case("binary left for transport", url="http://h", body=b"/path/file.bin", body_type="binary"),
    ]
    out = []
    for name, template, settings in cases:
        request = RequestDTO(**template.model_dump())
        try:
            url, headers, body = prepare(request, ApplicationSettings(**settings))
            expected = {"url": url, "headers": [list(pair) for pair in headers], "body": base64.urlsafe_b64encode(body).decode()}
        except Exception as error:  # noqa: BLE001
            expected = {"error": str(error)}
        out.append({"name": name, "request": dump(template), "settings": settings, "expected": expected})
    return out


def resolve_cases() -> list[dict]:
    values = {"host": "example.com", "token": "t0k", "empty": "", "path": "/files", "spaced name": "s"}

    def case(label, **fields):
        return label, RequestTemplate(**fields)

    cases = [
        case("url and headers", url="https://{{host}}/x?t={{ token }}", headers=items(("X-{{token}}", "{{host}}"))),
        case("empty value counts", url="http://h/{{empty}}"),
        case("missing names are sorted", url="http://{{zeta}}/{{alpha}}", headers=items(("a", "{{beta}}"))),
        case("whitespace inside braces", url="http://h/{{  spaced name  }}"),
        case("no recursion into values", url="http://h/{{host}}", queries=items(("v", "{{{{host}}}}"))),
        case("triple braces", url="http://h/{{{host}}}"),
        case("newline breaks a placeholder", url="http://h", headers=items(("a", "{{ho\nst}} {{host}}"))),
        case("unclosed", url="http://h/{{host"),
        case("empty braces", url="http://h/{{}}"),
        case("auth fields", url="http://h", auth=RequestAuth.basic_auth("{{token}}", "{{empty}}")),
        case("options paths", url="http://h", options=Options(proxy_url="http://{{host}}:3128", ca_bundle="{{path}}/ca.pem", timeout=3.0)),
        case("cookies", url="http://h", cookies=items(("{{token}}", "{{host}}"))),
        case("json body", url="http://h", body=b'{"host": "{{host}}"}', body_type="json"),
        case("plain body", url="http://h", body="ü {{token}}".encode(), body_type="plain"),
        case("none body is not resolved", url="http://h", body=b"{{missing}}", body_type="none"),
        case("binary path is resolved", url="http://h", body=b"{{path}}/a.bin", body_type="binary"),
        case("form items resolved", url="http://h", body=form(("{{token}}", "{{host}}")), body_type="urlencoded"),
        case(
            "multipart items resolved",
            url="http://h",
            body=form({"name": "f", "value": "{{path}}/x", "kind": "file", "filename": "{{token}}.bin", "content_type": "{{empty}}text/plain"}, cls=FormItem),
            body_type="multipart",
        ),
        case("name and description untouched", name="{{missing}}", description="{{missing}}", url="http://h"),
        case("missing in body", url="http://h", body=b"{{nope}}", body_type="text"),
    ]
    out = []
    for name, template in cases:
        try:
            resolved = VariableService._resolve(RequestDTO(**template.model_dump()), values)  # noqa: SLF001
            expected = {"request": dump(resolved.template())}
        except Exception as error:  # noqa: BLE001
            expected = {"error": str(error)}
        out.append({"name": name, "request": dump(template), "values": values, "expected": expected})
    return out


def settings_cases() -> list[dict]:
    overrides = [
        {},
        {"timeout": 1.5, "follow_redirects": False, "verify_ssl": False, "attach_cookies": False},
        {"proxy_url": "", "ca_bundle": "", "certificate_path": "", "key_file": "", "password": ""},
        {"proxy_url": "http://proxy:3128", "download_limit": 0},
    ]
    base = ApplicationSettings(proxy_url="http://default:1", ca_bundle="/default/ca.pem", download_limit=10)
    return [
        {
            "base": json.loads(base.model_dump_json()),
            "options": json.loads(Options(**case).model_dump_json()),
            "expected": json.loads(base.resolve(Options(**case)).model_dump_json()),
        }
        for case in overrides
    ]


def write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    print(f"wrote {path.relative_to(ROOT)} ({len(data)} bytes)")


def pretty(value) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()


def main() -> None:
    write(ROOT / "native" / "full.poster.json", encode_document(full_document()))
    write(ROOT / "native" / "minimal.poster.json", encode_document(NativeDocument(collection=TransferCollection(name="Minimal"))))
    write(ROOT / "parity" / "decode.json", pretty(decode_cases()))
    write(ROOT / "parity" / "prepare.json", pretty(prepare_cases()))
    write(ROOT / "parity" / "resolve.json", pretty(resolve_cases()))
    write(ROOT / "parity" / "settings.json", pretty(settings_cases()))


if __name__ == "__main__":
    main()

# Poster agent manual

Poster is a local desktop HTTP API client written in Rust (egui/eframe). It is a rewrite of the
Python/PyQt6 prototype at `~/dev/pystman`; that app stays the reference implementation, and its
`AGENTS.md` is the behavioral spec. Port behaviors, not code structure.

`PLAN.md` holds the tech stack, behavioral spec, persistence rules and the staged roadmap.

## Workspace

```
crates/
  poster-core/      pure domain: DTOs, settings, variables, URL/query/header logic. No IO.
  poster-http/      transport: reqwest client, TLS, auth, uploads, capped streaming, cancellation
  poster-store/     SQLite: schema, migrations, backup, repositories, DB actor thread
  poster-transfer/  native .poster.json v1 + Postman v2.1 conversion and validation
  poster-app/       services: executor, saver, history retention, task registry
  poster-ui/        egui widgets and screens (library, tested with egui_kittest)
  poster/           binary: main(), eframe setup, logging, paths
```

Dependency direction: `core` <- `http`, `store`, `transfer` <- `app` <- `ui` <- `poster`.
Never add a dependency against this direction. `poster-core` must stay free of IO.

Internal crates and shared third-party versions are declared once in the root
`[workspace.dependencies]`; crates use `dep.workspace = true`.

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p poster
```

A stage (see `PLAN.md`) is done only when fmt, clippy and tests are green, plus the coverage floor
from stage 2 on (85% lines). Each stage is its own commit.

## Rules

- Toolchain is pinned in `rust-toolchain.toml`; `Cargo.lock` is committed. Use `--locked` in CI.
- Lints live in the root `Cargo.toml` (`[workspace.lints]`, clippy pedantic). Add allows there with
  a reason, not with `#[allow]` scattered through the code.
- No `unsafe` in our code.
- Errors: `thiserror` in library crates, `anyhow` only in the `poster` binary.
- The UI thread never blocks or awaits. Background work goes through the app handle and comes back
  over a channel drained each frame.
- Never log credentials or request/response bodies.
- The app's data lives in `~/.poster/poster.db` (override: `POSTER_DATA_DIR`). Never touch the
  Python app's database files.

## Stage 0 decisions

Record the outcome of each open question from `PLAN.md` here once settled.

- egui vs Iced: _pending_
- Cross-name header order on the wire: _pending_
- Legacy encrypted PEM keys: _pending_ (default: PKCS#8 only)
- Large body editor responsiveness: _pending_

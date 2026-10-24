# Releasing Poster

Packages are built by `cargo-packager` from the `[package.metadata.packager]` section of
`crates/poster/Cargo.toml`. The release workflow (`.github/workflows/release.yml`) builds them
on each OS, smoke-tests every packaged binary, and attaches them to a **draft** GitHub release.
Nothing is published until someone reviews the draft and publishes it.

| OS | Packages | Smoke-tested binary |
|---|---|---|
| macOS (arm64) | `.app` inside a `.dmg` | `Poster.app/Contents/MacOS/poster` |
| Linux (x86_64, glibc of Ubuntu 22.04 or newer) | `.deb`, `.AppImage` | the installed `.deb` and the AppImage, under Xvfb |
| Windows (x86_64) | NSIS installer (per user, no admin) | the silently installed `poster.exe` |

## Cutting a release

1. Set the version in the root `Cargo.toml` (`[workspace.package] version`) and run
   `cargo check` so `Cargo.lock` follows.
2. Run `scripts/check.sh`, then the manual walkthrough in [validation.md](validation.md) on a
   local package (below), and record the result there.
3. Commit, tag `vX.Y.Z`, push the tag.
4. Wait for the workflow; open the draft release, check the assets, publish it.

## Building locally

```sh
cargo install cargo-packager --locked --version 0.11.8
CI=true cargo packager -p poster --release --formats app,dmg   # macOS
cargo packager -p poster --release --formats deb,appimage      # Linux
cargo packager -p poster --release --formats nsis              # Windows
python3 scripts/smoke.py target/packages/Poster.app/Contents/MacOS/poster
```

`CI=true` skips the Finder styling of the DMG window, which needs automation permission.
Packages land in `target/packages/`.

## The smoke test

`poster --smoke-test <url>` starts the real app (window, database, migrations, HTTP, history),
sends `GET <url>` as an unsaved draft, closes through the normal close sequence, and exits 0
only if a response arrived. `scripts/smoke.py` runs it against a loopback server with a
throwaway `POSTER_DATA_DIR` and also checks that the server saw the request and that history
recorded it. CI runs it on every push (Linux, debug build); the release workflow runs it on
every package.

## Signing

Without the secrets below, packages are **unsigned**. macOS then requires right-click → Open
the first time, and Windows SmartScreen warns.

**macOS signing and notarization.** Repository secrets read by the release workflow:

| Secret | Meaning |
|---|---|
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)`; written into the packager config on the runner |
| `APPLE_CERTIFICATE` | the Developer ID certificate and key as a base64-encoded `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | notarization with an app-specific password |

cargo-packager also accepts an App Store Connect API key for notarization (`APPLE_API_KEY`,
`APPLE_API_ISSUER`, `APPLE_API_KEY_PATH`); the workflow would need those added to its `env`.

**Windows signing (optional).** Not set up. It needs a code-signing certificate in the
runner's certificate store and `certificate-thumbprint` plus `timestamp-url` under
`[package.metadata.packager.windows]`.

**Before the first signed release**, confirm the bundle identifier (`dev.poster.app`): macOS
identifies the app by it (Gatekeeper approvals, notarization), so it should not change later.

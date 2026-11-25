# CEF spike: the web wrapper on Chromium

A small standalone Tauri 3 alpha app that opens one https site in a window rendered by
Chromium (CEF, through `tauri-runtime-cef`), with the Kitchen web wrapper's hooks. It sits
outside the template and the template workspace. It is meant to be dropped into the template once the
Tauri 3 port (`feat/tauri3`) lands, as the `web_engine = chromium` engine.

Background: `chromium-engine-assessment.md` (project files). This spike checks it against
the published crates.

| Crate | Version (exact pin) |
|---|---|
| `tauri` | `=3.0.0-alpha.6` (no `wry` feature: the runtime is chosen with `Builder::runtime`) |
| `tauri-runtime-cef` | `=3.0.0-alpha.7` (pins `cef`/`cef-dll-sys` `=154.5.0+154.0.34`, Chromium 154.0.8037.98) |
| `tauri-build` | `=3.0.0-alpha.4` |
| `tauri-cli` (to bundle) | `3.0.0-alpha.6`, which uses `tauri-bundler` `3.0.0-alpha.5` |

## What was verified here (Linux x86_64 container)

- `cargo build`, `cargo test` (18 tests: the spike's policy and config, plus desktop-core's
  `url`, `pin` and `proxy` tests, compiled in) and `cargo clippy --all-targets -- -D warnings`
  all pass on Linux.
- `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` passes with the
  stub C toolchain. The macOS CEF distribution was downloaded through the agent proxy
  (`cef-builds.spotifycdn.com`: allowed). See "Type-checking macOS on Linux".
- It runs under Xvfb (as root, so with `--no-sandbox`). The window opens and mute is applied.
  **The pinned self-signed route works end to end.** A local `https://localhost:8443` with a
  self-signed certificate loaded through the loopback proxy, and the trust-on-first-use pin
  written to `pin.txt` equals `sha256(DER)` of the server certificate. Direct
  `https://example.com` was cancelled in this container, because the agent proxy re-signs
  TLS with a CA that Chromium's own store does not hold. That is the
  `CertificateErrorPolicy::Cancel` path working as intended, not a bug.
- **Not verified:** anything on a real Mac (signing, notarization, TCC prompts, the helper
  apps launching, deep links through CEF's NSApplication). These come first on Apple Silicon.

## Hooks: how they map

| Hook | Kitchen today (`src-tauri/src/wrapper/`, `desktop-core/src/wrapper/`) | This spike on `tauri-runtime-cef` | Status |
|---|---|---|---|
| Window and start URL | `WebviewWindowBuilder` on `tauri::Wry` | `WebviewWindowBuilder` on `CefRuntime` (`tauri::Builder::<CefRuntime>::new().runtime(Cef::default()…)`). The URL comes from `--url` / `KITCHEN_CEF_URL` (https only). | Works. The template's wrapper hard-codes `tauri::Wry` (`Menu<tauri::Wry>`, `AppHandle`), so it has to become generic over `R: Runtime` or use `CefRuntime`. |
| Navigation policy | `on_navigation` → `wurl::decide` | The same `on_navigation` and the same `desktop_core::wrapper::url::decide`: the same origin stays, other http(s) origins go to the default browser (`open`/`xdg-open`), `file:`/custom schemes/`*.localhost` are refused. Through the proxy, a link with the site's absolute URL is redirected onto the proxy, and loopback URLs never go to a browser. | Works (`policy.rs` tests). The runtime's request handler calls Tauri's `NavigationHandler`. |
| Downloads | `on_download` (`DownloadEvent`) | `on_download`: `Requested` sets `destination` to `~/Downloads/<safe name>` (deduplicated `name (1).ext`), `Finished` is logged. Because the destination is changed, CEF shows no save dialog (the runtime shows one only when it is left unchanged). | Works (`cef_impl/client/download.rs`). |
| Cookie and profile persistence | WebKit persistent data store, `data_directory(<app data>/wrapper)` | `Cef::root_cache_path(~/Library/Application Support/<id>/cef)` and `persist_session_cookies(true)`: a Chromium profile that keeps sign-ins, including session cookies, across restarts. Not `~/Library/Caches`, which macOS may purge. Proxy mode keeps a fixed port (`proxy::preferred_port(id)`), so cookies on `127.0.0.1:<port>` survive. | Works. Cookies are encrypted at rest with Chromium's `SecretStorage` (macOS Keychain by default). |
| Mute | `_setPageMuted:` (private WKWebView API) and a page script | `WebviewCefExt::with_cef_webview(… browser().host().set_audio_muted(1))`, from `--muted` and from an "Audio › Mute Audio" menu item (⌘⇧M). It covers the whole browser, including iframes and WebRTC. | Works, with no private API. Cleaner than today. |
| Media permission decisions | WKUIDelegate `requestMediaCapturePermission` and `desktop_core::wrapper::perm` | `on_permission_request`: Camera, Microphone and DisplayCapture are allowed only when the asking page is on the window's origin and `--allow-media camera,microphone,screen` allows it. Otherwise they are denied. Geolocation, notifications and MIDI are denied. `Cef::default_content_setting` also blocks notifications, geolocation, USB, serial and HID. | Works in code (`policy.rs` tests). The handler gets no requesting origin, so the spike uses `webview.url()` (the top frame). An iframe from another origin asking would be judged by the top frame. Needs `perm.rs` (which depends on `WrapperConfig`) ported or a runtime API that passes the origin. |
| Pinned self-signed site (`insecureTls`) | macOS: an added `didReceiveAuthenticationChallenge`. Others: the rustls loopback proxy | **The loopback proxy only.** With `--insecure-tls` (trust on first use) or `--pin <sha256>`, `desktop_core::wrapper::proxy::Proxy` listens on 127.0.0.1, the window opens `proxy.auth_url(path)`, and the proxy talks TLS to the site through `PinVerifier`. The pin is saved next to the profile (`pin.txt`). The window itself uses `CertificateErrorPolicy::Cancel`, so no interstitial can be clicked through. `127.0.0.1` is a secure context to Chromium, so `getUserMedia` still works. | Works end to end (tested above). The runtime has no per-certificate callback (`ChromeInterstitial` or `Cancel` only), so the proxy is the route. |
| User-Agent override | per-site rule, proxy header | `--user-agent`: `Cef::user_agent` in direct mode, `Upstream::user_agent` through the proxy. | Works. |
| Key capture, deep links | `CGEventTap`, `tauri-plugin-deep-link` | Not in the spike. The CGEventTap is engine-neutral. Deep links need plugin 3 alpha, and CEF's NSApplication must forward `open-url`. Test that first on a Mac. | Open |

### Reusing desktop-core's wrapper code

`desktop-core` **cannot be a path dependency**. Its `Cargo.toml` is a cargo-generate (Liquid)
template (`{% if web_wrapper %}`, `{{app_name}}`) and inherits `version.workspace = true`
etc. from the template workspace, so cargo cannot parse it from the template source tree. The
spike compiles the three engine-neutral files straight from the template with `#[path]`
(`src/wrapper.rs`), so there is still only one copy:

- `template/desktop-core/src/wrapper/url.rs`: the origin guard
- `template/desktop-core/src/wrapper/pin.rs`: `PinStore`, `PinVerifier`, fingerprints
- `template/desktop-core/src/wrapper/proxy.rs`: the loopback proxy

They need `rustls` (ring), `webpki-roots`, `sha2`, `ring` and `log`, plus a
`crate::net::check_host` (`src/net.rs` is a stub that allows every host. In the template it is
the network policy). Once the template is on Tauri 3, the engine goes into `src-tauri` and uses
`desktop_core::wrapper::*` normally. None of this is needed then.

## Build and run on a Mac (Apple Silicon)

Prerequisites: Xcode command line tools, `cmake` and `ninja` (`brew install cmake ninja`), and
Rust with `aarch64-apple-darwin`.

```sh
cd spikes/cef
cargo install tauri-cli --version 3.0.0-alpha.6 --locked   # the 3.x CLI: the 2.x one cannot package CEF
export CEF_PATH="$HOME/Library/Caches/tauri-cef"          # one CEF download shared by all builds (the CLI's default too)

# Dev: the CLI builds, bundles a .app (CEF helpers included) and launches it. Pass the
# settings through the environment (how the dev runner forwards app arguments differs).
KITCHEN_CEF_URL=https://your.site.example/ KITCHEN_CEF_MEDIA=camera,microphone cargo tauri dev

# A self-signed device (PiKVM, NAS): pin on first use, or give the fingerprint.
KITCHEN_CEF_URL=https://pikvm.local/ KITCHEN_CEF_INSECURE_TLS=1 cargo tauri dev
KITCHEN_CEF_URL=https://pikvm.local/ KITCHEN_CEF_PIN=AB:CD:… cargo tauri dev

# Release bundle (.app and .dmg in target/release/bundle/macos and …/dmg).
cargo tauri build --bundles app,dmg
open "target/release/bundle/macos/Kitchen CEF Spike.app" --args --url https://your.site.example/
```

`cargo run` on its own does not work on macOS. CEF launches its renderer and GPU processes
from the `… Helper (…).app` bundles inside `Contents/Frameworks`, so the app must run from a
bundle. `cargo tauri dev` does that (`run_dev_in_app_bundle` in the CLI). The first build
downloads the CEF minimal distribution (132 MB, about 340 MB extracted) into `CEF_PATH` and
builds `libcef_dll_wrapper` with cmake.

Flags on the executable: `--url`, `--pin <sha256>`, `--insecure-tls`, `--muted`,
`--allow-media camera,microphone,screen`, `--user-agent`. Their environment equivalents are
`KITCHEN_CEF_URL`, `KITCHEN_CEF_PIN`, `KITCHEN_CEF_INSECURE_TLS=1`, `KITCHEN_CEF_MUTED=1`,
`KITCHEN_CEF_MEDIA` and `KITCHEN_CEF_USER_AGENT`. `RUST_LOG=debug` gives more logging. The profile is in
`~/Library/Application Support/dev.kitchen.cef-spike/` (`cef/` and `pin.txt`). Delete it to
start clean or to forget the pin.

On Linux the same `cargo build && ./target/debug/kitchen-cef-spike --url …` runs a bare
executable. The build script copies `libcef.so` and its resources next to it. As root, add
`--no-sandbox`.

## How the CEF helpers are bundled and signed (3.x CLI)

From the `tauri-cli` 3.0.0-alpha.6 and `tauri-bundler` 3.0.0-alpha.5 sources:

1. **Detection.** The CLI picks the CEF runtime because `tauri-runtime-cef` is a dependency
   (`Runtime::detect`). Nothing has to be set in `tauri.conf.json`. `bundle.cef.embed` (default
   `true`) ships the framework, and `false` loads a shared runtime from
   `TAURI_CEF_LIBRARY_PATH`.
2. **Framework.** `Chromium Embedded Framework.framework` is copied whole from the distribution
   under `CEF_PATH/<version>/cef_macos_aarch64` into `Contents/Frameworks/`. All 220 `.lproj`
   locale folders come along (see the size notes below).
3. **Helpers.** The bundler compiles a small `tauri-cef-helper` crate it carries
   (`cef-helper/`) with cargo, for the bundle's target, against the same resolved `cef` and
   `cef-dll-sys` sources and the same `CEF_PATH`. A universal build is `lipo`ed. It writes
   five copies as `Contents/Frameworks/<Product> Helper.app`, `… Helper (GPU).app`,
   `(Renderer)`, `(Plugin)` and `(Alerts)`. Each one's `Info.plist` has `LSUIElement=1`,
   `LSMinimumSystemVersion 11.0` and `CFBundleIdentifier <identifier>.helper`. On macOS the
   subprocesses run these helpers, not the main executable. On Windows and Linux CEF
   relaunches the main executable with `--type=…`, and `#[cef_entry_point]` routes that launch.
4. **Signing.** Inside out: helpers and framework first (otherwise codesign fails with "In
   subcomponent: … Helper (Renderer).app"), then the main binary, then the `.app`. The
   identity is `bundle.macOS.signingIdentity` or `APPLE_SIGNING_IDENTITY` (or
   `APPLE_CERTIFICATE` + `APPLE_CERTIFICATE_PASSWORD` in CI). The hardened runtime is on
   (`hardenedRuntime: true`).
5. **Entitlements.** One entitlements file is used for **every** signed item, main app and
   helpers alike. The CLI merges its CEF set into ours: `com.apple.security.cs.allow-jit`,
   `allow-unsigned-executable-memory`, `disable-library-validation`. Our `Entitlements.plist`
   adds `device.camera`, `device.audio-input` and `network.client`/`server`. Because all
   helpers get the union, the renderer also gets camera and microphone entitlements. That is
   broader than Chrome's per-helper split, and acceptable for now. Changing it needs a bundler patch.
6. **Notarization** runs when `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` (or the API key
   variables) are set, and the ticket is stapled unless `skipStapling`. It uploads about
   150 MB, so expect it to take longer than it does today.
7. **Privacy strings.** `Info.plist` here (merged by the CLI into the main app) has
   `NSCameraUsageDescription` and `NSMicrophoneUsageDescription`. The bundler writes **no**
   usage strings into the helper plists. TCC normally charges the responsible process (the
   main app), so this should be enough, but confirm it on a Mac. If the prompt names the helper or
   crashes, the helper plists need the keys (a bundler patch, or a post-bundle `plutil` step
   before signing, which means signing ourselves).

## Bundle size

Measured from the CEF 154.0.34 `macosarm64_minimal` distribution the build downloads:

| Part | Unpacked | gzip -6 |
|---|---|---|
| `Chromium Embedded Framework` (the arm64 dylib) | 223 MB | |
| `Resources/` (paks, ICU, V8 snapshot, 220 `.lproj` locale folders = 50 MB) | 83 MB | |
| `Libraries/` (SwiftShader Vulkan, `libcef_sandbox`) | 17 MB | |
| **Framework total** | **322 MB** | **145 MB** |
| Framework without the non-English `.lproj` folders | about 273 MB | about 132 MB |

Estimated, since nothing could be built for macOS here: the main executable is about 10 MB
(Tauri, rustls, CEF bindings, release with LTO and stripped), and the five helper copies
are about 2 MB each (they are not hardlinked in the bundle).

- **`.app`: about 340 MB unpacked.** Today's WKWebView build is about 10 MB.
- **`.dmg` (UDZO, zlib): about 150 MB**, or about 137 MB if the locales are trimmed to `en`.
- **Updater `.app.tar.gz`: about 150 MB** for every update, with no deltas.

The assessment estimated 250–300 MB unpacked and a 100–130 MB DMG. The measured
framework is larger (322 MB), so plan for about 150 MB. The 3.x bundler copies every locale.
Trimming to `en.lproj` needs a bundler option or patch (none exists in alpha.5). Linux for
comparison: `libcef.so` alone is 1.4 GB unstripped in the minimal distribution (326 MB
`tar.bz2`).

## Type-checking macOS on Linux

```sh
export CARGO_TARGET_DIR=/home/user/wt/target-cef CARGO_INCREMENTAL=0 CEF_PATH=/home/user/wt/cef-dist
export CC_aarch64_apple_darwin=/home/user/wt/macstub/cc CXX_aarch64_apple_darwin=/home/user/wt/macstub/cc
export AR_aarch64_apple_darwin=/home/user/wt/macstub/ar
export CMAKE_TOOLCHAIN_FILE_aarch64_apple_darwin=$PWD/typecheck/macos-stub.cmake
cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings
```

`cef-dll-sys`'s build script downloads the macOS distribution and builds `libcef_dll_wrapper`
with cmake and ninja. With only `CC`/`AR` stubs, everything compiles to empty objects, but the
final step runs the **host** `/usr/bin/ranlib` on the stub's empty `.a` and fails
(`FAILED: libcef_dll_wrapper/libcef_dll_wrapper.a`).
`typecheck/macos-stub.cmake` sets `CMAKE_RANLIB=/bin/true` (and the stub compilers) for that
one case. It is never used on a Mac. If a configure already failed, delete
`$CARGO_TARGET_DIR/aarch64-apple-darwin/debug/build/cef-dll-sys-*/out` first, because cmake
keeps the cached ranlib.

Disk use: the two CEF distributions take about 1.9 GB under `CEF_PATH` (Linux 1.5 GB, macOS
337 MB), and the Linux build copies another 1.4 GB `libcef.so` into the target directory. The
spike sets `[profile.dev] debug = 0` to keep the target directory at about 3.5 GB.

## Files

- `src/main.rs`: `#[cef_entry_point] main`, stderr logging
- `src/engine.rs`: the hooks on `tauri-runtime-cef` (the only engine-specific code)
- `src/policy.rs`: engine-free decisions (navigation, media, download names), tested
- `src/config.rs`: flags and environment, tested
- `src/wrapper.rs`, `src/net.rs`: desktop-core's `url`, `pin`, `proxy` by `#[path]`, and the
  `check_host` stub
- `tauri.conf.json`, `Entitlements.plist`, `Info.plist`: bundle settings for the 3.x CLI
- `typecheck/macos-stub.cmake`: only for the Linux type-check of the macOS target

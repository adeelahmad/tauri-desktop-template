//! Feature tests for the app: they start the real, built app and drive its window
//! through [tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) (WebDriver).
//!
//! The tests live in `tests/` and are `#[ignore]`d, so `cargo test --workspace` only
//! compiles them. Run them with:
//!
//! ```text
//! kitchen test --feature
//! ```
//!
//! which builds the app with its frontend (`cargo tauri build --debug --no-bundle`,
//! `KITCHEN_ENV=test`), then runs `cargo test -p e2e -- --ignored` with `APP_BINARY` set.
//! By hand, the same:
//!
//! ```text
//! cargo install tauri-driver --locked
//! sudo apt install webkit2gtk-driver        # Linux: WebKitWebDriver
//!                                           # Windows: msedgedriver matching Edge, on PATH
//! KITCHEN_ENV=test cargo tauri build --debug --no-bundle
//! APP_BINARY=target/debug/<app> cargo test -p e2e -- --ignored --nocapture
//! ```
//!
//! On Linux without a display, wrap the last line in `xvfb-run -a`. macOS has no
//! WebDriver for WKWebView, so there a test skips itself; `kitchen test --feature` runs
//! `<app> --self-test` there instead.
//!
//! A test is a few lines with the small client in `tests/webdriver/mod.rs`:
//!
//! ```text
//! let app = webdriver::App::launch()?;          // None (skip) when the driver is missing
//! app.find("textarea")?.type_text("hello")?;
//! app.find_xpath("//button[normalize-space()='Add text']")?.click()?;
//! ```

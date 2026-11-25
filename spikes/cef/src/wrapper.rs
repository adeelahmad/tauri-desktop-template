//! desktop-core's engine-neutral wrapper policy, compiled straight from the template's source.
//!
//! `desktop-core` cannot be a path dependency: its `Cargo.toml` is a cargo-generate (Liquid)
//! template and inherits `workspace = true` keys from the template workspace. These three
//! files have no Liquid and no `desktop_core`-only dependencies, so the spike builds them with
//! `#[path]` instead (they stay one copy). `proxy.rs` calls `crate::net::check_host`, which
//! [`crate::net`] provides.

// The spike uses only part of each module's API; the rest is exercised by desktop-core.
#![allow(dead_code)]

#[path = "../../../template/desktop-core/src/wrapper/url.rs"]
pub mod url;

#[path = "../../../template/desktop-core/src/wrapper/pin.rs"]
pub mod pin;

#[path = "../../../template/desktop-core/src/wrapper/proxy.rs"]
pub mod proxy;

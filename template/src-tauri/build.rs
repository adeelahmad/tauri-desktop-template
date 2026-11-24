//! Build script: Tauri's own build step, plus the layered app config.
//!
//! The files in `../config/` are merged for this build's environment and target OS (see
//! `src/config_layers.rs`, which this script includes) and written to
//! `$OUT_DIR/app_config.json`; `src/app_config.rs` compiles that in.

#[path = "src/config_layers.rs"]
mod config_layers;
{% if supply_chain %}#[path = "../desktop-core/src/notices.rs"]
#[allow(dead_code)]
mod notices;
{% endif %}
fn main() {
    let chosen = std::env::var("KITCHEN_ENV").ok().filter(|e| !e.trim().is_empty()).or_else(|| std::env::var("RUN_MODE").ok());
    let env = config_layers::env_name(chosen.as_deref(), &std::env::var("PROFILE").unwrap_or_default());
    let platform = config_layers::platform_name(&std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default());
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("../config");
    println!("cargo:rerun-if-env-changed=KITCHEN_ENV");
    println!("cargo:rerun-if-env-changed=RUN_MODE");
    println!("cargo:rerun-if-changed={}", dir.display());
    for path in config_layers::layer_paths(&dir, &env, &platform) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if !dir.join("environments").join(format!("{env}.json")).exists() && !["development", "production", "test"].contains(&env.as_str()) {
        println!(
            "cargo:warning=KITCHEN_ENV={env}: there is no config/environments/{env}.json, so only settings.json and the platform apply"
        );
    }
    let config = config_layers::load(&dir, &env, &platform).unwrap_or_else(|e| panic!("config/: {e}"));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("cargo sets it")).join("app_config.json");
    std::fs::write(&out, config.to_string()).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

{% if supply_chain %}    third_party_notices();
{% endif %}    tauri_build::build()
}{% if supply_chain %}

/// Write the licence notices for what this build links. A failure (no `cargo` on the
/// PATH, an offline machine with a stale lockfile) gives a warning and a short notice
/// pointing at `cargo about` / `make audit`, never a failed build.
fn third_party_notices() {
    use std::path::Path;
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it");
    let root = Path::new(&manifest_dir).join("..");
    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets it")).to_path_buf();
    println!("cargo:rerun-if-changed={}", root.join("Cargo.lock").display());
    let package = std::env::var("CARGO_PKG_NAME").expect("cargo sets it");
    // The features this build enables, so the list matches what is linked.
    let features: Vec<String> = std::env::vars()
        .filter_map(|(k, _)| k.strip_prefix("CARGO_FEATURE_").map(|f| f.to_ascii_lowercase().replace('_', "-")))
        .filter(|f| f != "default")
        .collect();
    let target = std::env::var("TARGET").unwrap_or_default();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let metadata = |with_features: bool| {
        let mut cmd = std::process::Command::new(&cargo);
        cmd.args(["metadata", "--format-version", "1", "--offline", "--manifest-path"]).arg(Path::new(&manifest_dir).join("Cargo.toml"));
        if !target.is_empty() {
            cmd.args(["--filter-platform", &target]);
        }
        if with_features {
            cmd.arg("--no-default-features");
            if !features.is_empty() {
                cmd.args(["--features", &features.join(",")]);
            }
        }
        // The outer build's lock on the target folder must not be waited for.
        cmd.env_remove("CARGO_TARGET_DIR").env("CARGO_TARGET_DIR", out.join("metadata-target"));
        cmd.output().ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
    };
    let {% if frontend == "react" %}mut {% endif %}packages = match metadata(true).or_else(|| metadata(false)) {
        Some(m) => notices::rust_packages(&m, &package),
        None => {
            println!("cargo:warning=licence notices: `cargo metadata` failed; the About screen will say so");
            Vec::new()
        }
    };
{%- if frontend == "react" %}
    let ui = root.join("ui-react");
    let lock = ui.join("package-lock.json");
    println!("cargo:rerun-if-changed={}", lock.display());
    if let Some(lock) = std::fs::read_to_string(&lock).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        packages.extend(notices::npm_packages(&lock, &ui));
    }
{%- endif %}
    let text = if packages.is_empty() {
        "# Third-party notices\n\nThe list could not be made for this build. Run `cargo about generate` or `make audit` to see the licences.\n".to_string()
    } else {
        notices::render("{{app_name}}", &packages)
    };
    std::fs::write(out.join("THIRD_PARTY_NOTICES.md"), text).expect("write the notices");
    std::fs::write(out.join("third_party.json"), notices::summary(&packages).to_string()).expect("write the notices");
}{% endif %}

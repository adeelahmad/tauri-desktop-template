//! Build script: Tauri's own build step, plus the layered app config.
//!
//! The files in `../config/` are merged for this build's environment and target OS (see
//! `src/config_layers.rs`, which this script includes) and written to
//! `$OUT_DIR/app_config.json`; `src/app_config.rs` compiles that in.

#[path = "src/config_layers.rs"]
mod config_layers;

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

    tauri_build::build()
}

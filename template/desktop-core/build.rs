fn main() {
    // The target triple picks the right Postgres extension bundle at run time.
    println!("cargo:rustc-env=APP_TARGET={}", std::env::var("TARGET").unwrap_or_default());
{% if i18n %}    if std::env::var_os("CARGO_FEATURE_I18N").is_some() {
        locales();
    }
{% endif %}}
{% if i18n %}
/// Embed every `locales/<code>/app.ftl` (feature `i18n`): writes `$OUT_DIR/locales.rs`, the
/// list `desktop_core::i18n::LOCALES` reads, with `en` (the source) first.
fn locales() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut found: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().join("app.ftl")))
                .filter(|(code, file)| file.is_file() && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|(code, _)| (code != "en", code.clone()));
    let mut out = String::from("&[\n");
    for (code, file) in &found {
        println!("cargo:rerun-if-changed={}", file.display());
        out += &format!("    ({code:?}, include_str!({:?})),\n", file);
    }
    out += "]\n";
    let target = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("locales.rs");
    std::fs::write(target, out).expect("write locales.rs");
}
{% endif %}
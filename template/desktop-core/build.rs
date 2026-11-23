fn main() {
    // The target triple picks the right Postgres extension bundle at run time.
    println!("cargo:rustc-env=APP_TARGET={}", std::env::var("TARGET").unwrap_or_default());
}

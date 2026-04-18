fn main() {
    println!("cargo:rerun-if-changed=ui/assets/knotreels.png");
    println!("cargo:rerun-if-changed=ui/theme.slint");
    let icons_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/icons/lucide");
    if let Ok(entries) = std::fs::read_dir(&icons_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "svg") {
                let rel = p.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap();
                println!("cargo:rerun-if-changed={}", rel.display());
            }
        }
    }
    // `with_debug_info(true)` is required for the `i-slint-backend-testing`
    // `ElementHandle` API used by `tests/ui_floating_controls.rs` — without it
    // element-id lookups return nothing and every interaction test panics.
    // Minor binary-size hit; acceptable trade for regression coverage.
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile");
}

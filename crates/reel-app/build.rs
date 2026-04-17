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
    slint_build::compile("ui/app.slint").expect("slint compile");
}

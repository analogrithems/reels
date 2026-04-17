fn main() {
    println!("cargo:rerun-if-changed=ui/assets/knotreels.png");
    println!("cargo:rerun-if-changed=ui/theme.slint");
    for name in [
        "grip-horizontal",
        "skip-back",
        "skip-forward",
        "arrow-left",
        "arrow-right",
        "play",
        "pause",
        "chevron-down",
        "repeat",
        "volume-2",
        "volume-x",
        "maximize",
        "minimize",
        "chevrons-right",
    ] {
        println!("cargo:rerun-if-changed=ui/icons/lucide/{name}.svg");
    }
    slint_build::compile("ui/app.slint").expect("slint compile");
}

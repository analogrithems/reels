fn main() {
    println!("cargo:rerun-if-changed=ui/assets/knotreels.png");
    println!("cargo:rerun-if-changed=ui/theme.slint");
    slint_build::compile("ui/app.slint").expect("slint compile");
}

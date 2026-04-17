fn main() {
    println!("cargo:rerun-if-changed=ui/assets/knotreels.png");
    slint_build::compile("ui/app.slint").expect("slint compile");
}

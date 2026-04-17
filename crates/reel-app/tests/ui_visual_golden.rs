//! Phase 3 — visual regression for the main [`AppWindow`](reel_app::AppWindow).
//!
//! Renders with Slint’s headless [`MinimalSoftwareWindow`] (software renderer, no GPU, no `DISPLAY`).
//! Compares the RGBA framebuffer to a committed PNG.
//!
//! **Refresh the golden image** (after intentional UI changes):
//! ```sh
//! UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden
//! ```
//! Then commit `tests/golden/app_window_default.png`.

use std::path::PathBuf;
use std::rc::Rc;

use reel_app::AppWindow;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, WindowSize};

struct HeadlessSoftwarePlatform(Rc<MinimalSoftwareWindow>);

impl Platform for HeadlessSoftwarePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn golden_png_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/app_window_default.png")
}

#[test]
fn app_window_matches_golden_png() {
    let msw = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(HeadlessSoftwarePlatform(msw.clone())))
        .expect("set_platform: headless software renderer");

    let app = AppWindow::new().expect("AppWindow::new");
    app.window()
        .set_size(WindowSize::Physical(PhysicalSize::new(400, 320)));
    app.show().expect("show window");

    let shot = app
        .window()
        .take_snapshot()
        .expect("Window::take_snapshot — requires renderer-software + window size");

    let w = shot.width();
    let h = shot.height();
    let rgba = shot.as_bytes();

    let golden = golden_png_path();
    if std::env::var("UPDATE_UI_GOLDENS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        std::fs::create_dir_all(golden.parent().expect("golden path has parent"))
            .expect("create tests/golden");
        let img =
            image::RgbaImage::from_raw(w, h, rgba.to_vec()).expect("snapshot buffer → RgbaImage");
        img.save(&golden).expect("write golden PNG");
        eprintln!("Wrote {}", golden.display());
        return;
    }

    assert!(
        golden.is_file(),
        "missing golden PNG at {}\n\
         Generate it with:\n  UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden",
        golden.display()
    );

    let expected = image::open(&golden)
        .expect("read golden PNG")
        .to_rgba8();
    assert_eq!(w, expected.width(), "width mismatch");
    assert_eq!(h, expected.height(), "height mismatch");
    assert_eq!(
        rgba,
        expected.as_raw(),
        "pixel mismatch — if the UI change is intentional, run:\n  UPDATE_UI_GOLDENS=1 cargo test -p reel-app --test ui_visual_golden"
    );
}

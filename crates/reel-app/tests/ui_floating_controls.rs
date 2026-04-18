//! Interaction regression for the floating transport chrome.
//!
//! Guards the hit-targets on the play / step / skip / loop / mute / fullscreen
//! buttons so a future layout refactor can't silently collapse them to 0×0
//! again — the original bug was that [`TouchArea`] inside a `HorizontalBox`
//! had no preferred size, rendered its child icon, but received zero clicks.
//!
//! Uses [`i_slint_backend_testing`]’s [`ElementHandle`] harness: finds each
//! named `*-ta` TouchArea by element-id, synthesises a real left-button click
//! through the event loop, and asserts the wired callback fired.
//!
//! Run with:
//! ```sh
//! cargo test -p reel-app --test ui_floating_controls
//! ```
//!
//! Kept in a dedicated integration binary so the `i-slint-backend-testing`
//! platform init doesn’t collide with the software-rendered visual golden.

use std::cell::Cell;
use std::rc::Rc;

use i_slint_backend_testing::ElementHandle;
use reel_app::AppWindow;
use slint::platform::PointerEventButton;
use slint::ComponentHandle;

/// Click every labelled TouchArea on the floating transport once and assert
/// the matching callback fired exactly once. If any of these fails, the
/// TouchArea’s hit rect has almost certainly collapsed again.
#[test]
fn floating_transport_buttons_receive_clicks() {
    i_slint_backend_testing::init_integration_test_with_system_time();

    slint::spawn_local(async move {
        let app = AppWindow::new().expect("AppWindow::new");

        // Prime the state so the `enabled: root.media-ready` gates let
        // clicks through and duration-dependent seeks have something to
        // clamp against.
        app.set_media_ready(true);
        app.set_duration_ms(10_000.0);
        app.set_playhead_ms(0.0);
        app.set_is_playing(false);
        app.set_volume_percent(85.0);

        // Counters: one per callback we care about. Rc<Cell<u32>> so the
        // closures captured by `on_*` can mutate a shared count and the
        // outer `#[test]` body can read it back after each click.
        let play_pause_hits = Rc::new(Cell::new(0u32));
        let seek_hits = Rc::new(Cell::new(0u32));
        let rewind_hits = Rc::new(Cell::new(0u32));
        let forward_hits = Rc::new(Cell::new(0u32));
        let toggle_loop_hits = Rc::new(Cell::new(0u32));
        let volume_hits = Rc::new(Cell::new(0u32));
        let fullscreen_hits = Rc::new(Cell::new(0u32));

        {
            let c = play_pause_hits.clone();
            app.on_play_pause(move || c.set(c.get() + 1));
        }
        {
            let c = seek_hits.clone();
            app.on_seek_timeline(move |_| c.set(c.get() + 1));
        }
        {
            let c = rewind_hits.clone();
            app.on_transport_rewind(move || c.set(c.get() + 1));
        }
        {
            let c = forward_hits.clone();
            app.on_transport_forward(move || c.set(c.get() + 1));
        }
        {
            let c = toggle_loop_hits.clone();
            app.on_view_toggle_loop(move || c.set(c.get() + 1));
        }
        {
            let c = volume_hits.clone();
            app.on_volume_changed(move |_| c.set(c.get() + 1));
        }
        {
            let c = fullscreen_hits.clone();
            app.on_view_toggle_fullscreen(move || c.set(c.get() + 1));
        }

        app.show().expect("show AppWindow");

        // Sized snug (not full-screen) so the software layout actually
        // produces a non-zero rect for every hit target — the integration
        // backend lays out to the window’s current size.
        app.window()
            .set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(
                1280, 720,
            )));

        // Each helper finds the single element with the given id, clicks it
        // once, and panics with a clear message if the element is missing or
        // duplicated (indicating the .slint structure drifted).
        async fn click_one(app: &AppWindow, id: &str) {
            let mut it = ElementHandle::find_by_element_id(app, id);
            let el = it
                .next()
                .unwrap_or_else(|| panic!("element-id `{id}` not found in AppWindow"));
            assert!(
                it.next().is_none(),
                "element-id `{id}` is not unique — rename one or refine the selector",
            );
            el.single_click(PointerEventButton::Left).await;
        }

        click_one(&app, "AppWindow::play-ta").await;
        assert_eq!(
            play_pause_hits.get(),
            1,
            "play-ta click did not fire play-pause",
        );

        click_one(&app, "AppWindow::rw-ta").await;
        assert_eq!(
            rewind_hits.get(),
            1,
            "rw-ta click did not fire transport-rewind",
        );

        click_one(&app, "AppWindow::ff-ta").await;
        assert_eq!(
            forward_hits.get(),
            1,
            "ff-ta click did not fire transport-forward",
        );

        click_one(&app, "AppWindow::skip-start-ta").await;
        click_one(&app, "AppWindow::skip-end-ta").await;
        assert_eq!(
            seek_hits.get(),
            2,
            "skip-start-ta + skip-end-ta clicks did not fire seek-timeline twice",
        );

        click_one(&app, "AppWindow::loop-ta").await;
        assert_eq!(
            toggle_loop_hits.get(),
            1,
            "loop-ta click did not fire view-toggle-loop",
        );

        click_one(&app, "AppWindow::mute-ta").await;
        assert_eq!(
            volume_hits.get(),
            1,
            "mute-ta click did not fire volume-changed",
        );

        click_one(&app, "AppWindow::fs-ta").await;
        assert_eq!(
            fullscreen_hits.get(),
            1,
            "fs-ta click did not fire view-toggle-fullscreen",
        );

        slint::quit_event_loop().expect("quit event loop");
    })
    .expect("spawn_local");

    slint::run_event_loop().expect("run_event_loop");
}

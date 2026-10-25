//! Headless snapshots of the demo app. No window is opened (iced_test Simulator, tiny-skia).
//! Set `FB_WRITE_EVIDENCE=1` to copy the PNGs to `evidence/`.

use fb_ui::Mode;
use flockbook::snapshot::{distinct_colors, render, write_png, Options};
use flockbook::state::Screen;

fn check(screen: Screen, mode: Mode, name: &str) {
    let img = render(Options { screen, mode, size: (1280, 852) });
    assert_eq!((img.width, img.height), (2560, 1704));
    let colors = distinct_colors(&img);
    assert!(colors > 40, "{name}: only {colors} distinct colors, frame looks blank");
    // Sidebar and content are one tone, as in Swift (SidebarView.swift:74, "sidebar and content are one tone").
    let px = |x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    };
    assert_eq!(px(100, 1200), px(560, 1200), "{name}: sidebar and content differ in tone");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    write_png(&img, &dir.join(format!("{name}.png"))).unwrap();
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn conversation_light() {
    check(Screen::Conversation, Mode::Light, "conversation-light");
}
#[test]
fn conversation_dark() {
    check(Screen::Conversation, Mode::Dark, "conversation-dark");
}
#[test]
fn overview_light() {
    check(Screen::Overview, Mode::Light, "overview-light");
}
#[test]
fn overview_dark() {
    check(Screen::Overview, Mode::Dark, "overview-dark");
}

#[test]
fn light_and_dark_differ() {
    let a = render(Options { screen: Screen::Overview, mode: Mode::Light, size: (640, 400) });
    let b = render(Options { screen: Screen::Overview, mode: Mode::Dark, size: (640, 400) });
    assert_ne!(a.rgba, b.rgba);
}

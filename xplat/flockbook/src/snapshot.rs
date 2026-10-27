//! Headless rendering: one frame, offscreen, through `iced_test::Simulator` on the tiny-skia (CPU) renderer.
//! No window, no Dock icon, no focus change. Output is RGBA at 2x scale.
//! Under window glass (`state.glass.on`) the frame is drawn over a stand-in wallpaper (the mocks' `--wall`), so the
//! translucent surfaces show. There is no blur: the real backdrop is the OS's.

use crate::state::Screen;
use crate::{App, Message};
use fb_ui::Mode;
use iced::{Settings, Size};
use iced_test::Simulator;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub screen: Screen,
    pub mode: Mode,
    /// Logical size in points. The PNG is `size * 2`.
    pub size: (u32, u32),
}

impl Default for Options {
    fn default() -> Self {
        Options { screen: Screen::Conversation, mode: Mode::Light, size: (1280, 852) }
    }
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Renders an app that the caller built (any source, any state).
pub fn render_app(app: &App, size: (u32, u32)) -> Image {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    let view = if app.state.glass.on { iced::widget::stack![wall(app.state.mode), app.view()].into() } else { app.view() };
    // With the title bar hidden (macOS) the system draws the traffic lights over the content; draw them here too, so a
    // shot shows what they would cover.
    let view = if app.state.chrome.title_bar == crate::chrome::TitleBar::Hidden { iced::widget::stack![view, traffic_lights()].into() } else { view };
    let mut sim: Simulator<'_, Message> = Simulator::with_size(settings, Size::new(size.0 as f32, size.1 as f32), view);
    let snap = sim.snapshot(&app.theme()).expect("snapshot");
    // `Snapshot` keeps its pixels private; it can only write a PNG, so go through a temp file.
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("fb-snap-{}-{}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    snap.matches_image(dir.join("frame")).expect("write snapshot");
    let png_path = std::fs::read_dir(&dir).expect("snapshot dir").map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|e| e == "png")).expect("png written");
    let img = read_png(&png_path);
    let _ = std::fs::remove_dir_all(&dir);
    img
}

/// The mocks' `--wall` (DESIGN.md section 9): three soft color spots on a plain ground.
pub fn wall_svg(mode: Mode) -> String {
    let (a, b, c, ground) = if mode.is_dark() { ("#2b3a58", "#3b2c4c", "#1f3d44", "#12141a") } else { ("#b9cce6", "#dccfe0", "#e9ddd0", "#dde3eb") };
    let spot = |id: &str, color: &str, cx: f32, cy: f32, rx: f32, ry: f32| {
        format!(
            r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{cx}" cy="{cy}" r="{rx}" gradientTransform="translate({cx} {cy}) scale(1 {s}) translate(-{cx} -{cy})"><stop offset="0" stop-color="{color}"/><stop offset="0.7" stop-color="{color}" stop-opacity="0"/></radialGradient>"#,
            s = ry / rx
        )
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" preserveAspectRatio="none"><defs>{}{}{}</defs><rect width="100" height="100" fill="{ground}"/><rect width="100" height="100" fill="url(#a)"/><rect width="100" height="100" fill="url(#b)"/><rect width="100" height="100" fill="url(#c)"/></svg>"#,
        spot("a", a, 12.0, 18.0, 60.0, 70.0),
        spot("b", b, 88.0, 85.0, 55.0, 65.0),
        spot("c", c, 70.0, 10.0, 50.0, 60.0),
    )
}

/// The macOS window buttons: three 12 pt circles from x = 20, 8 pt apart, centred in the 52 pt top row.
fn traffic_lights<'a>() -> iced::Element<'a, Message> {
    use iced::widget::{container, row, Space};
    let light = |r: u8, g: u8, b: u8| -> iced::Element<'a, Message> {
        container(Space::new().width(12).height(12))
            .style(move |_| container::Style { background: Some(iced::Color::from_rgb8(r, g, b).into()), border: iced::Border { radius: 6.0.into(), ..Default::default() }, ..Default::default() })
            .into()
    };
    let top = (fb_ui::theme::TOPROW_H - 12.0) / 2.0;
    container(row![light(255, 95, 87), light(254, 188, 46), light(40, 200, 64)].spacing(8)).padding(iced::Padding { top, left: 20.0, right: 0.0, bottom: 0.0 }).into()
}

fn wall<'a>(mode: Mode) -> iced::Element<'a, Message> {
    let handle = iced::widget::svg::Handle::from_memory(wall_svg(mode).into_bytes());
    iced::widget::svg(handle).width(iced::Length::Fill).height(iced::Length::Fill).content_fit(iced::ContentFit::Fill).into()
}

fn read_png(path: &Path) -> Image {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("open png")));
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut buf).expect("png frame");
    buf.truncate(info.buffer_size());
    Image { width: info.width, height: info.height, rgba: buf }
}

/// Renders the demo app for `opts`.
pub fn render(opts: Options) -> Image {
    let app = App::demo(opts.mode, opts.screen);
    render_app(&app, opts.size)
}

pub fn write_png(img: &Image, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(&img.rgba).map_err(std::io::Error::other)?;
    Ok(())
}

/// Count of distinct colors, a cheap "is this more than a blank frame" measure.
pub fn distinct_colors(img: &Image) -> usize {
    let mut set = std::collections::HashSet::new();
    for p in img.rgba.chunks_exact(4) {
        set.insert([p[0], p[1], p[2], p[3]]);
        if set.len() > 4096 {
            break;
        }
    }
    set.len()
}

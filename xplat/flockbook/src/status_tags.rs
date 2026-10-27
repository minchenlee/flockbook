//! Read-only agent status tags above the conversation's input area.
use crate::{style, App, Message};
use crate::settings::SettingsMsg;
use fb_ui::icons::{icon, Icon, SIZE_INLINE};
use fb_ui::theme::*;
use herdr_core::StatusTag;
use iced::widget::{button, column, container, responsive, row, scrollable, Space};
use iced::{Alignment, Element, Length};
use std::time::{Duration, Instant};

fn tag(t: Tokens, tag: &StatusTag) -> Element<'static, Message> {
    let color = if tag.warn { t.blocked } else if tag.kind == "model" { t.label } else { t.label_2 };
    let mut line = row![if tag.kind == "model" { HEADLINE.text(tag.text.clone()).size(11) } else { CAPTION.text(tag.text.clone()) }.color(color)].spacing(5).align_y(Alignment::Center);
    if let Some(percent) = tag.percent.filter(|p| p.is_finite()) {
        let percent = percent.clamp(0.0, 100.0);
        let bar = row![container(Space::new()).width(Length::Fixed(28.0 * percent as f32 / 100.0)).height(4).style(style::fill(color, 2.0)), Space::new().width(Length::Fill)];
        line = line.push(container(bar).width(28).height(4).style(style::fill(t.fill_3, 2.0)));
    }
    container(line).padding([2, 7]).style(style::fill(t.fill_1, 10.0)).into()
}

/// `widths` are conservative estimates; overflow remains available through the expand control.
fn fitting(widths: &[f32], available: f32) -> usize {
    let mut used = 28.0;
    widths.iter().take_while(|&&w| { used += w + 6.0; used <= available }).count()
}

/// The status line files change at most every few seconds; one read a second matches the 800 ms refresh.
const TAGS_TTL: Duration = Duration::from_secs(1);

fn tags_for(app: &App, pane: &herdr_core::Pane) -> Vec<StatusTag> {
    let cache = &app.state.settings.status_tags;
    if let Some((at, v)) = cache.borrow().get(&pane.id) {
        if at.elapsed() < TAGS_TTL {
            return v.clone();
        }
    }
    let v = app.source.status_tags(pane);
    let mut cache = cache.borrow_mut();
    cache.retain(|_, (at, _)| at.elapsed() < TAGS_TTL);
    cache.insert(pane.id.clone(), (Instant::now(), v.clone()));
    v
}

pub fn view(app: &App) -> Element<'_, Message> {
    let Some(pane) = app.state.selected_pane() else { return Space::new().into() };
    let tags = tags_for(app, pane);
    if tags.is_empty() { return Space::new().into(); }
    let t = app.tokens();
    let pane_id = pane.id.clone();
    let open = app.state.settings.expanded_status.contains(&pane_id);
    responsive(move |size| {
        let widths: Vec<f32> = tags.iter().map(|t| t.text.chars().count() as f32 * 12.0 + 20.0 + if t.percent.is_some() { 33.0 } else { 0.0 }).collect();
        let shown = fitting(&widths, size.width);
        let toggle = button(icon(if open { Icon::ChevronDown } else { Icon::ChevronRight }, SIZE_INLINE, t.label_2))
            .padding(4).on_press(Message::Settings(SettingsMsg::ToggleStatus(pane_id.clone())));
        let content: Element<'_, Message> = if open {
            // Each tag may wrap in a narrow pane; the expanded area has its own height cap.
            let mut all = column![].spacing(6).width(Length::Fill);
            for item in &tags { all = all.push(tag(t, item)); }
            row![container(scrollable(all)).max_height(160.0).width(Length::Fill), toggle].spacing(6).into()
        } else {
            let mut line = row![].spacing(6).align_y(Alignment::Center);
            for item in tags.iter().take(shown) { line = line.push(tag(t, item)); }
            line = line.push(Space::new().width(Length::Fill));
            if shown < tags.len() { line = line.push(toggle); }
            line.into()
        };
        container(content).padding([4, 14]).width(Length::Fill).max_width(app.state.settings.column_width()).into()
    }).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collapse_reserves_expand_control_even_when_no_tags_fit() {
        assert_eq!(fitting(&[100.0, 100.0], 250.0), 2);
        assert_eq!(fitting(&[100.0, 100.0], 180.0), 1);
        assert_eq!(fitting(&[100.0], 50.0), 0);
    }

    /// Demo data that counts status line reads.
    struct Counting(crate::source::DemoSource, std::rc::Rc<std::cell::Cell<usize>>);
    impl crate::source::DataSource for Counting {
        fn status_tags(&self, _: &herdr_core::Pane) -> Vec<StatusTag> {
            self.1.set(self.1.get() + 1);
            vec![StatusTag { kind: "model".into(), text: "Model".into(), spoken: "Model".into(), percent: None, warn: false }]
        }
        fn label(&self) -> String { self.0.label() }
        fn workspaces(&self) -> Vec<herdr_core::Workspace> { self.0.workspaces() }
        fn connected(&self) -> bool { self.0.connected() }
        fn conversation(&self, p: &herdr_core::Pane) -> Vec<herdr_core::ConversationItem> { self.0.conversation(p) }
    }

    #[test]
    fn views_in_a_row_read_the_status_line_once() {
        let reads = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut app = App::new(Box::new(Counting(crate::source::DemoSource::new(), reads.clone())), crate::Mode::Light, crate::Screen::Conversation);
        app.refresh();
        for _ in 0..5 {
            let _ = view(&app);
        }
        assert_eq!(reads.get(), 1);
    }
}

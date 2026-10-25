//! Drawing of the approval card and the question card.

use super::flow::Flow;
use super::keys::catch;
use super::{Approval, Card, CardKey, CardsMsg, OTHER_ID};
use crate::composer::CARD_MAX_W;
use crate::{composer, style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_CARD};
use fb_ui::theme::*;
use iced::font::Weight;
use iced::keyboard::{self, key::Named, Key};
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Alignment, Background, Border, Color, Element, Length};

const MEDIUM: TextStyle = TextStyle { size: 13.0, line: 1.3, weight: Weight::Medium, mono: false };
/// The scrolled part of the question card (title and options) stops growing here and scrolls.
const MAX_BODY_H: f32 = 330.0;

/// Widget id of the approval card's title row (tests click it: it is not a button).
pub const HEAD_ID: &str = "cards-head";

fn msg(m: CardsMsg) -> Message {
    Message::Cards(m)
}

/// Key presses the question card reacts to (same rules as Swift `QuestionCard.key`). The approval card takes no keys.
/// `first_step`: Esc at the first question is not ours. `field_focused`: a text field (composer, "Other") had the focus
/// when the key came.
pub fn map_key(first_step: bool, e: &keyboard::Event, field_focused: bool) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = e else { return None };
    let chord = modifiers.command() || modifiers.control();
    // Like Swift: any modifier but Shift means the key is not ours.
    if chord || modifiers.alt() {
        return None;
    }
    let k = match key.as_ref() {
        Key::Named(Named::Enter) => CardKey::Enter { chord: false },
        // In the "Other" field the field owns Esc (it lets go itself), digits, Space and Down.
        Key::Named(Named::ArrowUp) => CardKey::Up,
        _ if field_focused => return None,
        Key::Named(Named::Escape) if !first_step => CardKey::Escape,
        Key::Named(Named::ArrowDown) => CardKey::Down,
        Key::Named(Named::Space) => CardKey::Space,
        Key::Character(c) => match c.parse::<usize>() {
            Ok(n @ 1..=9) if !modifiers.shift() => CardKey::Digit(n),
            _ => return None,
        },
        _ => return None,
    };
    Some(msg(CardsMsg::Key(k)))
}

/// Wraps the question card so it takes its keys. Every other card is returned as is: no keys.
fn keyed<'a>(app: &App, card: &Card, inner: Element<'a, Message>) -> Element<'a, Message> {
    let covered = app.state.settings.open || app.state.shortcuts.open;
    let Card::Question { key, .. } = card else { return inner };
    let first_step = app.state.cards.flow().is_none_or(|f| f.step == 0);
    catch(inner, key.clone(), true, move |e, field| if covered { None } else { map_key(first_step, e, field) }).into()
}

fn slot<'a>(inner: Element<'a, Message>) -> Element<'a, Message> {
    container(inner).padding([SP_8, SP_24]).width(Length::Fill).align_x(Alignment::Center).into()
}

/// The bottom of the conversation column: the composer, with the approval card above it, or the question card in its place.
pub fn bottom(app: &App) -> Element<'_, Message> {
    column![crate::settings::status_tags::view(app), bottom_body(app)].into()
}

fn bottom_body(app: &App) -> Element<'_, Message> {
    let Some(card) = &app.state.cards.card else { return composer::view(app) };
    let t = app.tokens();
    let body: Element<Message> = match card {
        Card::Approval(a) => column![keyed(app, card, slot(approval(app, a))), composer::view(app)].into(),
        Card::Question { can_send, agent, .. } => keyed(app, card, slot(question(app, agent, *can_send))),
        Card::Sent { agent } => {
            let line = container(CALLOUT.text(format!("Sent. {agent} is continuing.")).color(t.label_2)).padding([SP_12, 0.0]).center_x(Length::Fill);
            column![slot(line.into()), composer::view(app)].into()
        }
    };
    body
}

/// The card alone (no composer), for the Needs you detail. `None` when nothing is asked.
pub fn detail(app: &App) -> Option<Element<'_, Message>> {
    let t = app.tokens();
    let card = app.state.cards.card.as_ref()?;
    let body: Element<Message> = match card {
        Card::Approval(a) => approval(app, a),
        Card::Question { can_send, agent, .. } => question(app, agent, *can_send),
        Card::Sent { agent } => container(CALLOUT.text(format!("Sent. {agent} is continuing.")).color(t.label_2)).padding([SP_12, 0.0]).into(),
    };
    Some(keyed(app, card, body))
}

fn card_box<'a>(t: Tokens, inner: Element<'a, Message>) -> Element<'a, Message> {
    container(inner).padding(SP_16).width(Length::Fill).max_width(CARD_MAX_W).style(style::float_e2(t, R_CARD)).into()
}

/// A pill button: `primary` = accent fill (Allow, Send), else the quiet fill.
fn pill<'a>(t: Tokens, id: &'static str, label: &'a str, primary: bool, on_press: Option<Message>) -> Element<'a, Message> {
    let enabled = on_press.is_some();
    let fg = match (primary, enabled) {
        (true, true) => t.on_accent,
        (_, false) => t.label_3,
        _ => t.label,
    };
    let b = button(container(MEDIUM.text(label).color(fg)).center_y(Length::Fixed(30.0)).padding([0, 16]))
        .padding(0)
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg = match (primary, enabled) {
                (_, false) => t.fill_2,
                (true, true) => if hot { Color { a: 0.88, ..t.accent } } else { t.accent },
                (false, true) => if hot { t.fill_3 } else { t.fill_2 },
            };
            button::Style { background: Some(Background::Color(bg)), border: Border { radius: 15.0.into(), ..Default::default() }, ..Default::default() }
        });
    container(b).id(id).into()
}

/// A little keycap (hint line).
fn keycap<'a>(t: Tokens, s: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(CAPTION.text(s).color(t.label_2)).padding([1, 6]).style(move |_| container::Style {
        background: Some(Background::Color(t.fill_2)),
        border: Border { radius: 4.0.into(), ..Default::default() },
        ..Default::default()
    }).into()
}

/// The check or radio mark of a row.
fn mark<'a>(t: Tokens, on: bool, square: bool) -> Element<'a, Message> {
    let radius = if square { 5.0 } else { 9.0 };
    let inner: Element<Message> = if on { icon(Icon::Check, 12.0, t.on_accent) } else { Space::new().into() };
    container(inner)
        .center(Length::Fixed(18.0))
        .style(move |_| container::Style {
            background: on.then_some(Background::Color(t.accent)),
            border: Border { color: if on { t.accent } else { t.sep_strong }, width: 1.0, radius: radius.into() },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------- approval

/// "Run a command" -> "Wants to run a command".
fn wants(title: &str) -> String {
    let t = title.trim();
    let mut c = t.chars();
    match c.next() {
        Some(f) if f.is_uppercase() && !t.ends_with('?') => format!("Wants to {}{}", f.to_lowercase(), c.as_str()),
        _ => t.to_string(),
    }
}

fn approval<'a>(app: &'a App, a: &'a Approval) -> Element<'a, Message> {
    let t = app.tokens();
    let st = &app.state.cards;
    let tile = container(text("!").size(17).font(HEADLINE.font()).color(t.on_accent)).center(Length::Fixed(28.0)).style(style::fill(t.blocked, 8.0));
    let title = if a.title.trim().is_empty() { "Wants to run a command".to_string() } else { wants(&a.title) };
    let head = container(row![tile, HEADLINE.text(title).color(t.label)].spacing(SP_12).align_y(Alignment::Center)).id(HEAD_ID);
    // The description is part of what the prompt shows, so the card shows it too: nothing the owner approves is hidden.
    let mut lines = column![MONO.text(a.command.as_str()).size(app.state.settings.reading_size().code()).color(t.label)].spacing(SP_4);
    if !a.detail.trim().is_empty() {
        lines = lines.push(CAPTION.text(a.detail.as_str()).color(t.label_2));
    }
    // Bound the command body; Allow/Deny remain outside this scroll area.
    let chip = container(container(scrollable(lines).spacing(0)).max_height(approval_body_height(app.state.conversation.viewport.height)))
        .padding([SP_8, SP_12]).width(Length::Fill).style(style::fill(t.fill_1, 8.0));

    let go = a.can_act && !st.busy;
    let mut foot = row![].spacing(SP_12).align_y(Alignment::Center);
    let mut always_row: Option<Element<'a, Message>> = None;
    if let Some(al) = &a.always {
        let on = st.always;
        always_row = Some(
            container(
                button(row![mark(t, on, true), CAPTION.text(al.label.as_str()).color(t.label_2)].spacing(SP_8).align_y(Alignment::Center))
                    .padding(0)
                    .on_press_maybe(go.then_some(msg(CardsMsg::Always(!on))))
                    .style(|_, _| button::Style::default()),
            )
            .id("cards-always")
            .into(),
        );
    }
    foot = foot.push(Space::new().width(Length::Fill));
    if a.unbound {
        foot = foot.push(CAPTION.text(crate::actions::PROMPT_MISMATCH).color(t.blocked));
    } else if a.stale {
        foot = foot.push(CAPTION.text(crate::actions::PROMPT_CHANGED).color(t.blocked));
    } else if !a.can_act {
        foot = foot.push(CAPTION.text("Answer in the terminal: input is off.").color(t.label_2));
    }
    foot = foot.push(pill(t, "cards-deny", "Deny", false, go.then_some(msg(CardsMsg::Deny)))).push(pill(t, "cards-allow", "Allow", true, go.then_some(msg(CardsMsg::Allow))));

    let mut body = column![head, chip].spacing(SP_12);
    if let Some(r) = always_row {
        body = body.push(r);
    }
    if let Some(p) = &st.problem {
        body = body.push(CAPTION.text(p.as_str()).color(t.blocked));
    }
    card_box(t, body.push(foot).into())
}

// ---------------------------------------------------------------- question

fn option_row<'a>(t: Tokens, flow: &'a Flow, i: usize, can: bool) -> Element<'a, Message> {
    let o = &flow.question().options[i];
    let on = flow.picked(i);
    let multi = flow.question().multi_select;
    let mut title = row![MEDIUM.text(o.label.as_str()).color(t.label)].spacing(SP_8).align_y(Alignment::Center);
    if o.recommended {
        title = title.push(
            container(CAPTION.text("Recommended").color(t.accent)).padding([1, 6]).style(style::fill(t.accent_soft, 9.0)),
        );
    }
    let mut text_col = column![title].spacing(2);
    if !o.detail.is_empty() {
        text_col = text_col.push(BODY.text(o.detail.as_str()).color(t.label_2));
    }
    let cursor = flow.moved && flow.cursor == i;
    let b = button(row![container(mark(t, on, multi)).padding([1, 0]), text_col].spacing(SP_12))
        .padding([SP_8, SP_12])
        .width(Length::Fill)
        .on_press_maybe(can.then_some(msg(CardsMsg::Pick(i))))
        .style(move |_, status| {
            let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let bg = if on { Some(t.accent_soft) } else if hot || cursor { Some(t.fill_1) } else { None };
            button::Style { background: bg.map(Background::Color), text_color: t.label, ..Default::default() }
        });
    container(b).id(format!("cards-opt-{i}")).into()
}

fn other_row<'a>(app: &'a App, flow: &'a Flow, can: bool) -> Element<'a, Message> {
    let t = app.tokens();
    let st = &app.state.cards;
    let value = flow.other[flow.step].as_str();
    let on = !flow.other_text().is_empty();
    let placeholder = "Other: type your own answer";
    let field: Element<Message> = if can && (st.other_active || !value.is_empty()) {
        text_input(placeholder, value)
            .id(OTHER_ID)
            .on_input(|s| msg(CardsMsg::OtherInput(s)))
            .font(BODY.font())
            .size(BODY.size)
            .padding([4, 8])
            .style(move |_, status| text_input::Style {
                background: Background::Color(t.fill_1),
                border: Border {
                    color: if matches!(status, text_input::Status::Focused { .. }) { t.accent } else { t.sep },
                    width: if matches!(status, text_input::Status::Focused { .. }) { 1.5 } else { 1.0 },
                    radius: 6.0.into(),
                },
                icon: t.label_2,
                placeholder: t.label_3,
                value: t.label,
                selection: Color { a: 0.28, ..t.accent },
            })
            .into()
    } else {
        // Looks like the field; a click turns it into the real field with the keyboard.
        container(
            button(container(BODY.text(placeholder).color(t.label_3)).padding([4, 8]).width(Length::Fill).style(style::fill(t.fill_1, 6.0)))
                .padding(0)
                .width(Length::Fill)
                .on_press_maybe(can.then_some(msg(CardsMsg::ActivateOther)))
                .style(|_, _| button::Style::default()),
        )
        .id("cards-other-open")
        .into()
    };
    container(row![container(mark(t, on, flow.question().multi_select)).padding([5, 0]), field].spacing(SP_12).align_y(Alignment::Center))
        .padding([SP_8, SP_12])
        .style(move |_| container::Style { background: on.then_some(Background::Color(t.accent_soft)), ..Default::default() })
        .into()
}

fn question<'a>(app: &'a App, agent: &'a str, can_send: bool) -> Element<'a, Message> {
    let t = app.tokens();
    let st = &app.state.cards;
    // `recompute` keeps a flow for every question card; without one there is nothing to draw.
    let Some(flow) = st.flow() else { return Space::new().into() };
    let qq = flow.question();
    let can = can_send && !st.busy;

    let mut head = row![icon(Icon::CircleHelp, SIZE_CARD, t.blocked), row![HEADLINE.text(agent).color(t.label), BODY.text("asks you").color(t.label_2)].spacing(4)]
        .spacing(SP_8)
        .align_y(Alignment::Center);
    head = head.push(Space::new().width(Length::Fill));
    if !flow.header().is_empty() {
        head = head.push(container(CAPTION.text(flow.header()).color(t.label_2)).padding([2, 8]).style(style::fill(t.fill_2, 10.0)));
    }
    if flow.total() > 1 {
        head = head.push(CALLOUT.text(format!("{} of {}", flow.step + 1, flow.total())).color(t.label_2));
    }

    let mut title = column![TITLE_3.text(qq.text.as_str()).size(app.state.settings.reading_size().heading(3)).color(t.label)].spacing(SP_4);
    if qq.multi_select {
        title = title.push(BODY.text("Choose any.").color(t.label_2));
    }
    let mut group = column![].width(Length::Fill);
    for i in 0..qq.options.len() {
        group = group.push(option_row(t, flow, i, can)).push(container(Space::new()).height(Length::Fixed(1.0)).width(Length::Fill).style(style::fill(t.sep, 0.0)));
    }
    group = group.push(other_row(app, flow, can));
    let group = container(group).width(Length::Fill).clip(true).style(move |_| container::Style {
        background: Some(Background::Color(t.fill_1)),
        border: Border { color: t.sep, width: 1.0, radius: 10.0.into() },
        ..Default::default()
    });
    let body = scrollable(column![title, group].spacing(SP_12)).spacing(0);

    // Footer: key hints (or why nothing can be sent) and the buttons.
    let n = qq.options.len();
    let hint: Element<Message> = if let Some(p) = &st.problem {
        CAPTION.text(p.as_str()).color(t.blocked).into()
    } else if !can_send {
        let why = if flow.unread() > 0 { "Answer in the terminal: this card reads one question at a time." } else { "Answer in the terminal: input is off." };
        CAPTION.text(why).color(t.label_2).into()
    } else {
        let range = if n > 1 { row![keycap(t, "1"), CAPTION.text("\u{2013}").color(t.label_3), keycap(t, n.min(9).to_string())] } else { row![keycap(t, "1")] };
        row![
            range.spacing(SP_4).align_y(Alignment::Center),
            CAPTION.text(if qq.multi_select { "toggle" } else { "choose" }).color(t.label_2),
            keycap(t, "\u{21A9}"),
            CAPTION.text(if flow.is_last() { "send" } else { "next" }).color(t.label_2),
        ]
        .spacing(SP_4)
        .align_y(Alignment::Center)
        .into()
    };
    let mut foot = row![hint, Space::new().width(Length::Fill)].spacing(SP_8).align_y(Alignment::Center);
    if flow.step > 0 {
        foot = foot.push(pill(t, "cards-back", "Back", false, (!st.busy).then_some(msg(CardsMsg::Back))));
    }
    let label = if flow.is_last() { if flow.total() > 1 { "Send Answers" } else { "Send" } } else { "Next" };
    let go = can && flow.valid() && (!flow.is_last() || can_send);
    foot = foot.push(pill(t, "cards-send", label, true, go.then_some(msg(CardsMsg::Submit))));

    let inner = column![
        head,
        container(body).max_height(question_body_height(app.state.conversation.viewport.height)),
        foot,
    ]
    .spacing(SP_16);
    card_box(t, inner.into())
}

/// Reserve space for the top row, composer, header and action buttons in a short window.
fn approval_body_height(viewport: f32) -> f32 { (viewport * 0.3).clamp(48.0, 180.0) }
fn question_body_height(viewport: f32) -> f32 { (viewport * 0.5).clamp(80.0, MAX_BODY_H) }

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn prompt_bodies_are_bounded_and_shrink_in_short_windows() {
        assert_eq!(approval_body_height(10000.0), 180.0);
        assert!(approval_body_height(300.0) < approval_body_height(800.0));
        assert_eq!(question_body_height(10000.0), MAX_BODY_H);
    }
}

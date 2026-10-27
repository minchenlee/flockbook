//! Flockbook (iced): a reading view for Herdr agents. Unofficial; not affiliated with Herdr, Inc.
//!
//! # Contract for module builders
//!
//! ```text
//! App { state: State, source: Box<dyn DataSource> }
//! App::update(Message) -> Task<Message>        // routes to the module updates below
//! App::view() -> Element<Message>              // sidebar | (toprow / main area)
//! ```
//!
//! Layout: sidebar (fixed `SIDEBAR_W`) | right column = top row (`TOPROW_H` = 52) + main area. The main area shows
//! `state.screen`: Conversation (conversation + composer), Overview, NeedsYou or Search (stubs for step 3).
//!
//! Each module file owns ONE area and exposes exactly this shape:
//!
//! ```text
//! pub struct XState { .. }            // own state, `Default`; registered as ONE field in state.rs
//! pub enum XMsg { .. }                // own messages; wrapped as `Message::X(XMsg)`
//! pub fn view(app: &App) -> Element<'_, Message>
//! pub fn update(app: &mut App, msg: XMsg) -> Task<Message>
//! ```
//!
//! | file              | owns                                  | state field          | Message variant      |
//! |-------------------|---------------------------------------|----------------------|----------------------|
//! | `sidebar.rs`      | sidebar (nav, workspace rows)         | `state.sidebar`      | `Message::Sidebar`   |
//! | `toprow.rs`       | top row (tabs strip, actions)         | `state.toprow`       | `Message::TopRow`    |
//! | `overview.rs`     | Overview, NeedsYou, Search screens    | `state.overview`     | `Message::Overview`  |
//! | `conversation.rs` | conversation list                     | `state.conversation` | `Message::Conversation` |
//! | `terminal.rs`     | live terminal sheet (Show terminal)   | `state.terminal`     | `Message::Terminal`  |
//! | `composer.rs`     | composer and approval/question cards  | `state.composer`     | `Message::Composer`  |
//!
//! Shared and cross-cutting things live in `state.rs` (State, Screen, selection, helpers), `source.rs` (DataSource),
//! `style.rs` (container styles), `demo.rs` (demo data), `snapshot.rs` (headless render). Cross-module actions
//! (navigate, select a workspace or tab) are plain `Message` variants handled in `App::update`; a module sends them
//! instead of editing another module's state.
//!
//! Design tokens, fonts, icons and status symbols come from `fb-ui`. Never hard-code a color: use `Tokens`.

pub mod access;
pub mod actions;
pub mod cards;
pub mod chrome;
pub mod composer;
pub mod conversation;
pub mod demo;
pub mod glass;
pub mod links;
pub mod needs_you;
pub mod overview;
pub mod search;
pub mod settings;
pub mod shortcuts;
pub mod sidebar;
pub mod snapshot;
pub mod source;
pub mod state;
pub mod style;
pub mod terminal;
pub mod toprow;
pub mod workspace_actions;
pub mod pane_layout;

use fb_ui::theme::Tokens;
use fb_ui::Mode;
use iced::widget::{column, container, row, Space};
use iced::{Element, Length, Task, Theme};
use source::DataSource;
use state::{Screen, State};

pub use state::Screen as AppScreen;

#[derive(Debug, Clone)]
pub enum Message {
    /// Cross-cutting actions, handled in `App::update`.
    Navigate(Screen),
    SelectWorkspace(String),
    SelectTab { workspace: String, tab: String },
    SelectPane(String),
    SetMode(Mode),
    /// Polls the source. Fired by the subscription in `main.rs`.
    Tick,
    Noop,
    Workspace(workspace_actions::WorkspaceMsg),
    PaneConversation { pane: String, message: conversation::ConversationMsg },
    // One wrapper per module.
    Sidebar(sidebar::SidebarMsg),
    TopRow(toprow::TopRowMsg),
    Overview(overview::OverviewMsg),
    NeedsYou(needs_you::NeedsYouMsg),
    Conversation(conversation::ConversationMsg),
    Composer(composer::ComposerMsg),
    Cards(cards::CardsMsg),
    Search(search::SearchMsg),
    Terminal(terminal::TerminalMsg),
    Glass(glass::GlassMsg),
    Chrome(chrome::ChromeMsg),
    Settings(settings::SettingsMsg),
    Shortcut(shortcuts::Action),
}

pub struct App {
    pub state: State,
    pub source: Box<dyn DataSource>,
    /// Opens clicked links. A recorder by default; `main.rs` installs the system opener for a real window.
    pub opener: Box<dyn links::LinkOpener>,
}

impl App {
    pub fn new(source: Box<dyn DataSource>, mode: Mode, screen: Screen) -> App {
        let mut state = State::new(mode, screen);
        state.session_label = source.label();
        let mut app = App { state, source, opener: Box::new(links::RecordingOpener::default()) };
        app.refresh();
        app
    }

    /// An app on the demo data with Quarry / ranking selected (what the snapshots show).
    pub fn demo(mode: Mode, screen: Screen) -> App {
        let demo = demo::DemoSource::new();
        let (ws, tab) = demo.default_selection();
        let mut app = App::new(Box::new(demo), mode, screen);
        app.state.select_tab(&ws, &tab);
        app.state.overview.clock = Some(overview::Clock::demo());
        app.refresh();
        // The demo's blocked panes show their cards; the actions only record (no socket).
        cards::install_demo(&mut app);
        // Demo terminals are canned frames: no process, no socket.
        terminal::install_demo(&mut app);
        app
    }

    pub fn tokens(&self) -> Tokens {
        Tokens::of(self.state.mode)
    }

    /// Tokens for text on the glass bars (sidebar, top row, Needs you list): the stronger labels under window glass.
    pub fn bar_tokens(&self) -> Tokens {
        if self.state.glass.on {
            self.tokens().on_glass()
        } else {
            self.tokens()
        }
    }

    /// The color under the main area: `bg`, or the almost opaque `content` tint under window glass.
    pub fn ground(&self) -> iced::Color {
        let t = self.tokens();
        if self.state.glass.on {
            t.content
        } else {
            t.bg
        }
    }

    pub fn theme(&self) -> Theme {
        self.tokens().iced_theme()
    }

    pub fn title(&self) -> String {
        match (self.state.selected_workspace(), self.state.selected_tab()) {
            (Some(w), Some(t)) => format!("{} \u{2014} {}", w.label, t.label),
            _ => "Flockbook".to_string(),
        }
    }

    /// Pulls the tree and the selected conversation from the source.
    pub fn refresh(&mut self) {
        // On Needs you the cards work on the selected request's pane.
        needs_you::retarget(self);
        let changed = self.state.reload(self.source.as_ref());
        let pane = self.state.selected_pane().map(|p| p.id.clone());
        pane_layout::switch_reader(&mut self.state, pane.as_deref());
        self.state.composer.switch_pane(pane.as_deref());
        // Rows are rebuilt only when the conversation changed or another pane is shown.
        if changed || self.state.conversation.pane != pane {
            conversation::sync_markdown(self);
        }
        cards::on_refresh(self);
        pane_layout::refresh_other_readers(self);
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        if self.state.settings.open && settings::blocks_underlying(&message) { return Task::none(); }
        composer::sync_permissions(self);
        // The sheet is a modal: the composer and the cards below it get no input and send nothing while it shows.
        if self.state.terminal.is_open() && terminal::blocks_underlying(&message) {
            return Task::none();
        }
        if workspace_actions::modal(self) && terminal::blocks_underlying(&message) {
            return Task::none();
        }
        let was_open = self.state.terminal.is_open();
        let mut task = self.route(message);
        // The sheet just opened: the widget that had the keyboard (the composer) lets go of it.
        if !was_open && self.state.terminal.is_open() {
            task = Task::batch([task, iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())]);
        }
        // The terminal sheet follows the screen and the pane: leaving either releases it.
        terminal::after_update(self);
        composer::sync_permissions(self);
        pane_layout::release_hidden_pictures(&mut self.state);
        match glass::sync(self) {
            Some(g) => Task::batch([task, g]),
            None => task,
        }
    }

    fn route(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(screen) => {
                self.state.screen = screen;
                needs_you::sync_target(self);
                Task::none()
            }
            Message::SelectWorkspace(id) => {
                self.state.select_workspace(&id);
                self.state.screen = Screen::Conversation;
                self.refresh();
                Task::none()
            }
            Message::SelectTab { workspace, tab } => {
                self.state.select_tab(&workspace, &tab);
                self.state.screen = Screen::Conversation;
                self.refresh();
                Task::none()
            }
            Message::SelectPane(id) => {
                self.state.selected_pane = Some(id);
                self.refresh();
                Task::none()
            }
            Message::SetMode(mode) => {
                self.state.mode = mode;
                Task::none()
            }
            Message::Tick => {
                self.refresh();
                Task::none()
            }
            Message::Noop => Task::none(),
            Message::Workspace(m) => workspace_actions::update(self, m),
            Message::PaneConversation { pane, message } => pane_layout::update_reader(self, pane, message),
            Message::Sidebar(m) => sidebar::update(self, m),
            Message::TopRow(m) => toprow::update(self, m),
            Message::Overview(m) => overview::update(self, m),
            Message::NeedsYou(m) => needs_you::update(self, m),
            Message::Conversation(m @ conversation::ConversationMsg::EarlierLoaded { .. }) => {
                let conversation::ConversationMsg::EarlierLoaded { ref pane, .. } = m else { unreachable!() };
                pane_layout::update_reader(self, pane.clone(), m)
            }
            Message::Conversation(m) => conversation::update(self, m),
            Message::Composer(m) => composer::update(self, m),
            Message::Cards(m) => cards::update(self, m),
            Message::Search(m) => search::update(self, m),
            Message::Terminal(m) => terminal::update(self, m),
            Message::Glass(m) => glass::update(self, m),
            Message::Chrome(m) => chrome::update(self, m),
            Message::Settings(m) => settings::update(self, m),
            Message::Shortcut(a) => shortcuts::run(self, a),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let t = self.tokens();
        let ground = self.ground();
        let main: Element<Message> = match self.state.screen {
            Screen::Conversation => pane_layout::view(self),
            Screen::Overview => style::with_top_fade(ground, overview::view(self)),
            Screen::NeedsYou => needs_you::view(self),
            Screen::Search => overview::view(self),
        };
        // Settings is a page of the window (Swift `SettingsPage`): it takes the detail column and leaves the screen it was
        // opened from in place, hidden and inert, for Back. No tab row while it shows: it is not about a workspace.
        let on_settings = self.state.settings.open;
        let main = iced::widget::stack![settings::hide(main, on_settings), settings::page(self)];
        let bar: Element<Message> = if on_settings { Space::new().width(Length::Fill).into() } else { toprow::view(self) };
        let sidebar = chrome::sidebar(self);
        let toggle = chrome::sidebar_toggle(self);
        let covered = self.state.shortcuts.open;
        if self.state.glass.on {
            // Window glass: each surface paints one tint over a clear root, so translucent layers never stack.
            let main = container(main).width(Length::Fill).height(Length::Fill);
            let main = if needs_you::paints_own_ground(self) && !on_settings { main } else { main.style(style::fill(ground, 0.0)) };
            let right = column![container(bar).width(Length::Fill).height(Length::Fixed(fb_ui::theme::TOPROW_H)).style(style::fill(t.glass_bar, 0.0)), main];
            let body = row![sidebar, container(right).width(Length::Fill).height(Length::Fill)];
            let body: Element<Message> = match toggle { Some(toggle) => iced::widget::stack![body, toggle].into(), None => body.into() };
            return container(iced::widget::stack![settings::cover(iced::widget::stack![body, settings::hide(toprow::menu_layer(self), on_settings), settings::hide(workspace_actions::layer(self), on_settings), settings::hide(terminal::layer(self), on_settings)], covered), shortcuts::layer(self)]).width(Length::Fill).height(Length::Fill).into();
        }
        let right = column![container(bar).height(Length::Fixed(fb_ui::theme::TOPROW_H)), container(main).width(Length::Fill).height(Length::Fill)];
        let body = row![sidebar, container(right).width(Length::Fill).height(Length::Fill).style(style::content(t))];
        let body: Element<Message> = match toggle { Some(toggle) => iced::widget::stack![body, toggle].into(), None => body.into() };
        container(iced::widget::stack![settings::cover(iced::widget::stack![body, settings::hide(toprow::menu_layer(self), on_settings), settings::hide(workspace_actions::layer(self), on_settings), settings::hide(terminal::layer(self), on_settings)], covered), shortcuts::layer(self)]).width(Length::Fill).height(Length::Fill).style(style::content(t)).into()
    }
}

/// A placeholder body for a screen that is not built yet.
pub fn stub<'a>(app: &App, title: &'a str) -> Element<'a, Message> {
    let t = app.tokens();
    container(column![fb_ui::theme::TITLE_3.text(title).color(t.label), Space::new().height(4), fb_ui::theme::BODY.text("Not built yet.").color(t.label_2)]
        .align_x(iced::Alignment::Center))
    .center(Length::Fill)
    .into()
}


pub mod pictures;
pub mod queued;

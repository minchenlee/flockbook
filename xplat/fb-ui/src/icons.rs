//! Lucide v0.460.0 icons (ISC, see `icons/LICENSE`), the set from DESIGN.md section 5.
//!
//! The SVG files are embedded. One stroke width (1.75) is patched in at load time and the stroke is recolored with
//! the svg tint, so one cached handle per icon serves every color. Sizes: 14 (inline), 16 (default), 20 (cards).

use iced::widget::svg;
use iced::{Color, Element, Length};
use std::cell::RefCell;
use std::collections::HashMap;

pub const SIZE_INLINE: f32 = 14.0;
pub const SIZE_DEFAULT: f32 = 16.0;
pub const SIZE_CARD: f32 = 20.0;
/// Stroke width in the 24-grid. Lucide's default is 2.
pub const STROKE: f32 = 1.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    SquarePen,
    Inbox,
    LayoutGrid,
    Search,
    PanelLeft,
    Folder,
    ListFilter,
    Plus,
    X,
    SquareTerminal,
    FileText,
    Pencil,
    Globe,
    Mic,
    ArrowUp,
    ChevronLeft,
    ChevronUp,
    ChevronRight,
    ChevronDown,
    ChevronsUpDown,
    GitBranch,
    Smartphone,
    Hammer,
    Check,
    FileDiff,
    Shield,
    Ellipsis,
    Columns2,
    Rows2,
    GripVertical,
    Sparkles,
    CircleUserRound,
    Code,
    Eye,
    BookOpen,
    PenTool,
    CircleHelp,
    Activity,
    ArrowsCollapse,
    ArrowsExpand,
    TextCursorInput,
    ExternalLink,
    Layers,
    Keyboard,
    Settings2,
}

impl Icon {
    pub const ALL: [Icon; 45] = [
        Icon::SquarePen,
        Icon::Inbox,
        Icon::LayoutGrid,
        Icon::Search,
        Icon::PanelLeft,
        Icon::Folder,
        Icon::ListFilter,
        Icon::Plus,
        Icon::X,
        Icon::SquareTerminal,
        Icon::FileText,
        Icon::Pencil,
        Icon::Globe,
        Icon::Mic,
        Icon::ArrowUp,
        Icon::ChevronLeft,
        Icon::ChevronUp,
        Icon::ChevronRight,
        Icon::ChevronDown,
        Icon::ChevronsUpDown,
        Icon::GitBranch,
        Icon::Smartphone,
        Icon::Hammer,
        Icon::Check,
        Icon::FileDiff,
        Icon::Shield,
        Icon::Ellipsis,
        Icon::Columns2,
        Icon::Rows2,
        Icon::GripVertical,
        Icon::Sparkles,
        Icon::CircleUserRound,
        Icon::Code,
        Icon::Eye,
        Icon::BookOpen,
        Icon::PenTool,
        Icon::CircleHelp,
        Icon::Activity,
        Icon::ArrowsCollapse,
        Icon::ArrowsExpand,
        Icon::TextCursorInput,
        Icon::ExternalLink,
        Icon::Layers,
        Icon::Keyboard,
        Icon::Settings2,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Icon::SquarePen => "square-pen",
            Icon::Inbox => "inbox",
            Icon::LayoutGrid => "layout-grid",
            Icon::Search => "search",
            Icon::PanelLeft => "panel-left",
            Icon::Folder => "folder",
            Icon::ListFilter => "list-filter",
            Icon::Plus => "plus",
            Icon::X => "x",
            Icon::SquareTerminal => "square-terminal",
            Icon::FileText => "file-text",
            Icon::Pencil => "pencil",
            Icon::Globe => "globe",
            Icon::Mic => "mic",
            Icon::ArrowUp => "arrow-up",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronsUpDown => "chevrons-up-down",
            Icon::GitBranch => "git-branch",
            Icon::Smartphone => "smartphone",
            Icon::Hammer => "hammer",
            Icon::Check => "check",
            Icon::FileDiff => "file-diff",
            Icon::Shield => "shield",
            Icon::Ellipsis => "ellipsis",
            Icon::Columns2 => "columns-2",
            Icon::Rows2 => "rows-2",
            Icon::GripVertical => "grip-vertical",
            Icon::Sparkles => "sparkles",
            Icon::CircleUserRound => "circle-user-round",
            Icon::Code => "code",
            Icon::Eye => "eye",
            Icon::BookOpen => "book-open",
            Icon::PenTool => "pen-tool",
            Icon::CircleHelp => "circle-help",
            Icon::Activity => "activity",
            Icon::ArrowsCollapse => "arrows-collapse",
            Icon::ArrowsExpand => "arrows-expand",
            Icon::TextCursorInput => "text-cursor-input",
            Icon::ExternalLink => "external-link",
            Icon::Layers => "layers",
            Icon::Keyboard => "keyboard",
            Icon::Settings2 => "settings-2",
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            Icon::SquarePen => include_str!("../icons/square-pen.svg"),
            Icon::Inbox => include_str!("../icons/inbox.svg"),
            Icon::LayoutGrid => include_str!("../icons/layout-grid.svg"),
            Icon::Search => include_str!("../icons/search.svg"),
            Icon::PanelLeft => include_str!("../icons/panel-left.svg"),
            Icon::Folder => include_str!("../icons/folder.svg"),
            Icon::ListFilter => include_str!("../icons/list-filter.svg"),
            Icon::Plus => include_str!("../icons/plus.svg"),
            Icon::X => include_str!("../icons/x.svg"),
            Icon::SquareTerminal => include_str!("../icons/square-terminal.svg"),
            Icon::FileText => include_str!("../icons/file-text.svg"),
            Icon::Pencil => include_str!("../icons/pencil.svg"),
            Icon::Globe => include_str!("../icons/globe.svg"),
            Icon::Mic => include_str!("../icons/mic.svg"),
            Icon::ArrowUp => include_str!("../icons/arrow-up.svg"),
            Icon::ChevronLeft => include_str!("../icons/chevron-left.svg"),
            Icon::ChevronUp => include_str!("../icons/chevron-up.svg"),
            Icon::ChevronRight => include_str!("../icons/chevron-right.svg"),
            Icon::ChevronDown => include_str!("../icons/chevron-down.svg"),
            Icon::ChevronsUpDown => include_str!("../icons/chevrons-up-down.svg"),
            Icon::GitBranch => include_str!("../icons/git-branch.svg"),
            Icon::Smartphone => include_str!("../icons/smartphone.svg"),
            Icon::Hammer => include_str!("../icons/hammer.svg"),
            Icon::Check => include_str!("../icons/check.svg"),
            Icon::FileDiff => include_str!("../icons/file-diff.svg"),
            Icon::Shield => include_str!("../icons/shield.svg"),
            Icon::Ellipsis => include_str!("../icons/ellipsis.svg"),
            Icon::Columns2 => include_str!("../icons/columns-2.svg"),
            Icon::Rows2 => include_str!("../icons/rows-2.svg"),
            Icon::GripVertical => include_str!("../icons/grip-vertical.svg"),
            Icon::Sparkles => include_str!("../icons/sparkles.svg"),
            Icon::CircleUserRound => include_str!("../icons/circle-user-round.svg"),
            Icon::Code => include_str!("../icons/code.svg"),
            Icon::Eye => include_str!("../icons/eye.svg"),
            Icon::BookOpen => include_str!("../icons/book-open.svg"),
            Icon::PenTool => include_str!("../icons/pen-tool.svg"),
            Icon::CircleHelp => include_str!("../icons/circle-help.svg"),
            Icon::Activity => include_str!("../icons/activity.svg"),
            Icon::ArrowsCollapse => include_str!("../icons/arrows-collapse.svg"),
            Icon::ArrowsExpand => include_str!("../icons/arrows-expand.svg"),
            Icon::TextCursorInput => include_str!("../icons/text-cursor-input.svg"),
            Icon::ExternalLink => include_str!("../icons/external-link.svg"),
            Icon::Layers => include_str!("../icons/layers.svg"),
            Icon::Keyboard => include_str!("../icons/keyboard.svg"),
            Icon::Settings2 => include_str!("../icons/settings-2.svg"),
        }
    }
}

/// The SVG text with stroke 1.75 and a black stroke (the tint recolors it).
pub fn patched_source(icon: Icon) -> String {
    icon.source().replace("currentColor", "#000000").replace("stroke-width=\"2\"", &format!("stroke-width=\"{STROKE}\""))
}

thread_local! {
    static HANDLES: RefCell<HashMap<Icon, svg::Handle>> = RefCell::new(HashMap::new());
}

pub fn handle(icon: Icon) -> svg::Handle {
    HANDLES.with(|h| {
        h.borrow_mut().entry(icon).or_insert_with(|| svg::Handle::from_memory(patched_source(icon).into_bytes())).clone()
    })
}

/// An icon at `size` points in `color` (alpha is kept).
pub fn icon<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    // The SVG tint takes the color without its alpha, so a translucent label (secondary, tertiary) drew solid black or
    // white; the alpha goes to the widget's opacity instead.
    svg(handle(icon))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .opacity(color.a)
        .style(move |_, _| svg::Style { color: Some(Color { a: 1.0, ..color }) })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_lucide_svg_with_stroke_1_75() {
        for i in Icon::ALL {
            let s = patched_source(i);
            assert!(s.contains("lucide"), "{}", i.name());
            assert!(s.contains("stroke-width=\"1.75\""), "{}", i.name());
            assert!(!s.contains("currentColor"), "{}", i.name());
        }
    }
    #[test]
    fn names_are_unique() {
        let mut n: Vec<_> = Icon::ALL.iter().map(|i| i.name()).collect();
        n.sort();
        n.dedup();
        assert_eq!(n.len(), Icon::ALL.len());
    }
}

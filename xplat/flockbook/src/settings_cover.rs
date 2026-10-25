//! Keeps the reader's widget state/focus while a settings sheet covers it, without delivering input underneath.
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{mouse, overlay, renderer, Clipboard, Shell, Widget};
use iced::{Element, Event, Length, Rectangle, Size, Vector};

pub fn cover<'a, M: 'a>(content: impl Into<Element<'a, M>>, covered: bool) -> Element<'a, M> {
    Element::new(Cover { content: content.into(), covered })
}
struct Cover<'a, M> { content: Element<'a, M>, covered: bool }
impl<M> Widget<M, iced::Theme, iced::Renderer> for Cover<'_, M> {
    fn size(&self) -> Size<Length> { self.content.as_widget().size() }
    fn size_hint(&self) -> Size<Length> { self.content.as_widget().size_hint() }
    fn children(&self) -> Vec<Tree> { vec![Tree::new(&self.content)] }
    fn diff(&self, tree: &mut Tree) { tree.diff_children(std::slice::from_ref(&self.content)); }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(&self, tree: &Tree, renderer: &mut iced::Renderer, theme: &iced::Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, if self.covered { mouse::Cursor::Unavailable } else { cursor }, viewport);
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        if !self.covered { self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation); }
    }
    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &iced::Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, M>, viewport: &Rectangle) {
        if self.covered { return; }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }
    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &iced::Renderer) -> mouse::Interaction {
        if self.covered { mouse::Interaction::None } else { self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer) }
    }
    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &iced::Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        if self.covered { None } else { self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation) }
    }
}

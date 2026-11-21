//! Small shared pieces: icons and icon buttons.

use gpui::{
    div, prelude::*, px, svg, App, ClickEvent, Div, ElementId, Hsla, SharedString, Stateful, Window,
};

use crate::theme::ActiveTheme as _;

pub fn icon(name: &str, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(16.))
        .flex_none()
        .text_color(color)
}

pub fn icon_sized(name: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A square, borderless button holding one icon.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: &str,
    color: Hsla,
    cx: &App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = cx.theme().hover;
    div()
        .id(id.into())
        .w(px(22.))
        .h(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(name, color))
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx)
        })
}

pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .px_2()
            .py(px(3.))
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .rounded(px(4.))
            .text_size(px(12.))
            .text_color(t.fg)
            .child(self.0.clone())
    }
}

pub fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

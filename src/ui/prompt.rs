//! A themed confirmation dialog, used where the OS has no native one (Linux).

use gpui::{
    div, prelude::*, px, App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    PromptButton, PromptHandle, PromptLevel, PromptResponse, RenderablePromptHandle, Window,
};

use crate::theme::{ActiveTheme as _, ThemeState};

pub fn themed_prompt(
    _level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let view = cx.new(|cx| ThemedPrompt {
        message: message.to_string(),
        detail: detail.map(str::to_string),
        actions: actions.iter().map(|a| a.label().to_string()).collect(),
        focus: cx.focus_handle(),
    });
    handle.with_view(view, window, cx)
}

struct ThemedPrompt {
    message: String,
    detail: Option<String>,
    actions: Vec<String>,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for ThemedPrompt {}

impl Focusable for ThemedPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ThemedPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let font = cx.global::<ThemeState>().ui_font.clone();
        let cancel_ix = self
            .actions
            .iter()
            .position(|a| a == "Cancel")
            .unwrap_or(self.actions.len().saturating_sub(1));
        let buttons = self.actions.iter().enumerate().rev().map(|(ix, label)| {
            let primary = ix == 0;
            let (bg, fg, hover) = if primary {
                (t.accent, t.on_accent, t.accent_hover)
            } else {
                (t.input, t.fg, t.hover)
            };
            div()
                .id(ix)
                .px(px(14.))
                .h(px(28.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if primary { t.accent } else { t.border })
                .bg(bg)
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label.clone())
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(ix))))
        });
        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.35))
            .font_family(font)
            .text_size(px(13.))
            .child(
                div()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(move |_, e: &KeyDownEvent, _, cx| {
                        match e.keystroke.key.as_str() {
                            "escape" => cx.emit(PromptResponse(cancel_ix)),
                            "enter" => cx.emit(PromptResponse(0)),
                            _ => {}
                        }
                    }))
                    .w(px(360.))
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(8.))
                    .shadow_lg()
                    .text_color(t.fg)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.message.clone()),
                    )
                    .children(
                        self.detail
                            .clone()
                            .map(|d| div().text_color(t.muted).text_size(px(12.)).child(d)),
                    )
                    .child(
                        div()
                            .mt(px(8.))
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .children(buttons),
                    ),
            )
    }
}

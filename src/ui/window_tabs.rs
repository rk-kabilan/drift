//! macOS window tabs. With "Prefer tabs when opening documents: Always",
//! new windows join the current one as native tabs, but the native tab bar sits
//! in the title bar, which Drift draws over. So, as in Zed, Drift draws the
//! tabs itself below its title bar whenever the window has more than one,
//! styled like the tab bar of Finder and Terminal.

use gpui::{div, prelude::*, px, AnyWindowHandle, Hsla, MouseButton, SharedString, Window};

use super::workspace::{CloseWindow, NewWindow, Workspace};
use crate::theme::{ActiveTheme as _, Appearance};

/// The windows of this window's tab group, this one included, in tab order;
/// `None` when the window is on its own.
fn tabs(window: &Window) -> Option<Vec<(AnyWindowHandle, SharedString)>> {
    let tabs = window.tabbed_windows()?;
    (tabs.len() > 1).then(|| tabs.into_iter().map(|t| (t.handle, t.title)).collect())
}

/// Selects the tab `step` places after (or before) this window's tab.
pub(super) fn select_tab(step: isize, window: &Window, cx: &mut gpui::App) {
    let Some(tabs) = tabs(window) else {
        return;
    };
    let id = window.window_handle().window_id();
    let Some(ix) = tabs.iter().position(|(h, _)| h.window_id() == id) else {
        return;
    };
    let n = tabs.len() as isize;
    let next = tabs[(ix as isize + step).rem_euclid(n) as usize].0;
    activate(next, cx);
}

/// Activating a tabbed window selects its tab. Deferred: another window can
/// only be updated once this one's update has finished.
fn activate(handle: AnyWindowHandle, cx: &mut gpui::App) {
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
    });
}

/// Closes a tab's window, asking about its unsaved edits first.
fn close(handle: AnyWindowHandle, cx: &mut gpui::App) {
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, cx| {
                window.dispatch_action(Box::new(CloseWindow), cx)
            })
            .ok();
    });
}

fn shade(c: Hsla, by: f32) -> Hsla {
    Hsla {
        l: (c.l + by).clamp(0., 1.),
        ..c
    }
}

impl Workspace {
    pub(super) fn render_window_tabs(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let tabs = tabs(window)?;
        let this_window = window.window_handle().window_id();
        // A background window raises no tab, as on macOS.
        let window_active = window.is_window_active();
        let t = cx.theme();
        let dark = t.appearance == Appearance::Dark;
        // The selected tab has the title bar's colour; the bar is a step darker,
        // so it reads as sunken below it.
        let selected = t.chrome;
        let (bar, hover, divider) = if dark {
            (
                shade(selected, -0.04),
                shade(selected, -0.015),
                shade(selected, 0.08),
            )
        } else {
            (
                shade(selected, -0.05),
                shade(selected, -0.085),
                shade(selected, -0.14),
            )
        };
        let (fg, muted, faint) = (t.fg, t.muted, t.faint);
        let close_hover = shade(hover, if dark { 0.08 } else { -0.07 });
        let ix_selected = tabs.iter().position(|(h, _)| h.window_id() == this_window);

        let x_icon = move |color: Hsla| super::widgets::icon_sized("close", 10., color);
        Some(
            div()
                .id("window-tabs")
                .h(px(28.))
                .flex_none()
                .flex()
                .bg(bar)
                .border_t(px(0.5))
                .border_b(px(0.5))
                .border_color(divider)
                .children(
                    tabs.into_iter()
                        .enumerate()
                        .map(move |(ix, (handle, title))| {
                            let is_selected = Some(ix) == ix_selected;
                            let raised = is_selected && window_active;
                            // Dividers sit between tabs, but not beside the raised one.
                            let divider_left = ix > 0
                                && !raised
                                && !(window_active && ix_selected == Some(ix - 1));
                            let text = if raised {
                                fg
                            } else if window_active {
                                muted
                            } else {
                                faint
                            };
                            let group = SharedString::from(format!("window-tab-{ix}"));
                            let close_button = div()
                                .id(("window-tab-close", ix))
                                .absolute()
                                .left(px(6.))
                                .top(px(6.))
                                .size(px(16.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(!is_selected, |d| {
                                    d.invisible().group_hover(group.clone(), |s| s.visible())
                                })
                                .hover(move |s| s.bg(close_hover))
                                .child(x_icon(muted))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    close(handle, cx);
                                });
                            div()
                                .id(("window-tab", ix))
                                .group(group)
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .px(px(26.))
                                .when(divider_left, |d| d.border_l(px(0.5)).border_color(divider))
                                .when(raised, |d| d.bg(selected))
                                .when(!is_selected, |d| d.hover(move |s| s.bg(hover)))
                                .text_size(px(12.))
                                .text_color(text)
                                .when(raised, |d| d.font_weight(gpui::FontWeight::MEDIUM))
                                .child(div().truncate().child(title))
                                .child(close_button)
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    if !is_selected {
                                        activate(handle, cx);
                                    }
                                })
                                .on_mouse_down(MouseButton::Middle, move |_, _, cx| {
                                    close(handle, cx)
                                })
                        }),
                )
                .child(
                    div()
                        .id("window-tab-new")
                        .flex_none()
                        .w(px(30.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_l(px(0.5))
                        .border_color(divider)
                        .hover(move |s| s.bg(hover))
                        .child(super::widgets::icon_sized("plus", 12., muted))
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(NewWindow), cx)),
                ),
        )
    }
}

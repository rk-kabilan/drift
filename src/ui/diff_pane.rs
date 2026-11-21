//! The diff pane: header with layout toggle and actions, column labels, and
//! the virtualised list of diff rows.

use std::ops::Range;

use gpui::{
    div, pattern_slash, prelude::*, px, uniform_list, Context, FontStyle, HighlightStyle, Hsla,
    SharedString, StyledText, Window,
};

use super::sidebar::{file_badge, status_color};
use super::widgets::{icon, icon_button, tooltip};
use super::workspace::{CursorLine, DiffState, MergeDirection, Workspace};
use crate::diff::{Layout, LineKind, Row, Side};
use crate::syntax::{self, Lang};
use crate::theme::{ActiveTheme as _, Theme, ThemeState};

pub const ROW_HEIGHT: f32 = 20.;

impl Workspace {
    pub(super) fn render_diff_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut pane = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(t.editor)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.));

        let Some(diff) = self.diff() else {
            let msg = if self.staged.is_empty() && self.unstaged.is_empty() {
                "Nothing to commit. The working tree is clean."
            } else {
                "Select a file to see its changes, or press Cmd+P to find one."
            };
            let msg = msg.replace("Cmd", super::workspace::secondary_key());
            return pane.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.muted)
                    .child(msg),
            );
        };

        pane = pane.child(self.render_diff_header(&t, cx));
        pane = pane.child(
            div()
                .h(px(24.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(14.))
                .text_size(px(12.))
                .text_color(t.muted)
                .border_b_1()
                .border_color(t.border)
                .children(breadcrumbs(&diff.change.path, &t)),
        );
        if self.layout == Layout::SideBySide {
            pane = pane.child(
                div()
                    .flex_none()
                    .flex()
                    .text_size(px(11.))
                    .text_color(t.muted)
                    .border_b_1()
                    .border_color(t.border)
                    .child(column_head("commit", &diff.left_label, &t))
                    .child(
                        div()
                            .w(px(GUTTER))
                            .flex_none()
                            .h_full()
                            .border_l_1()
                            .border_r_1()
                            .border_color(t.border),
                    )
                    .child(column_head("file", &diff.right_label, &t)),
            );
        }

        if diff.file.is_empty() {
            let note = if diff.empty {
                "This file is empty."
            } else {
                "No text changes (mode or line-ending changes only)."
            };
            return pane.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.muted)
                    .child(note),
            );
        }

        let count = diff.rows.len();
        let scroll = self
            .active_tab()
            .map(|t| t.scroll.clone())
            .unwrap_or_default();
        if self.char_width.is_none() {
            let mono = cx.global::<ThemeState>().mono_font.clone();
            let ts = window.text_system();
            let font_id = ts.resolve_font(&gpui::font(mono));
            self.char_width = ts
                .advance(font_id, px(CODE_SIZE), 'm')
                .ok()
                .map(|s| f32::from(s.width));
        }
        // Keep the horizontal scroll in range after a resize or layout change.
        let max = self.max_h_scroll();
        if let Some(tab) = self.active.and_then(|a| self.tabs.get_mut(a)) {
            tab.h_scroll = tab.h_scroll.min(max);
        }
        let width = self.diff_width.clone();
        pane.child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .flex()
                .flex_col()
                .child(self.render_rows(count, scroll, cx))
                .children(self.render_scroll_indicators(&t))
                .child(
                    gpui::canvas(
                        move |bounds, _, _| width.set(f32::from(bounds.size.width)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
        )
    }

    /// Width available to code text on one side, in pixels.
    fn code_width(&self) -> f32 {
        let w = self.diff_width.get();
        match self.layout {
            Layout::SideBySide => (w - GUTTER) / 2. - 48. - 16. - 12.,
            Layout::Inline => w - 88. - 16. - 12. - 56.,
        }
    }

    /// How far the code can scroll sideways: enough to show the end of the longest line.
    fn max_h_scroll(&self) -> f32 {
        let Some(d) = self.diff() else { return 0. };
        let char_width = self.char_width.unwrap_or(CODE_SIZE * 0.6);
        (d.file.max_cols as f32 * char_width + 24. - self.code_width()).max(0.)
    }

    fn render_rows(
        &self,
        count: usize,
        scroll: gpui::UniformListScrollHandle,
        cx: &mut Context<Self>,
    ) -> gpui::UniformList {
        let mut list = uniform_list(
            "diff-rows",
            count,
            cx.processor(|this, range: Range<usize>, _window, cx| {
                let state = cx.global::<ThemeState>();
                let t = state.theme.clone();
                let mono: SharedString = state.mono_font.clone().into();
                let ui: SharedString = state.ui_font.clone().into();
                let arrows = this.merge_directions();
                let Some(diff) = this.diff() else {
                    return vec![];
                };
                let fonts = (mono, ui);
                let offset = this.active_tab().map_or(0., |t| t.h_scroll);
                range
                    .filter_map(|ix| diff.rows.get(ix).map(|row| (ix, row)))
                    .map(|(ix, row)| render_row(ix, row, diff, arrows, offset, &t, &fonts, cx))
                    .collect()
            }),
        )
        .track_scroll(scroll)
        .flex_1()
        .min_h_0()
        // Sideways scrolling: trackpad swipes, or Shift + mouse wheel.
        .on_scroll_wheel(
            cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| this.on_diff_scroll(e, cx)),
        );
        // Without this, gpui turns a purely sideways swipe into vertical scrolling.
        list.style().restrict_scroll_to_axis = Some(true);
        list
    }

    /// Scrolls the code sideways. Each gesture is locked to the axis it starts
    /// in, so a sideways swipe doesn't also scroll the list up or down (and
    /// the reverse).
    fn on_diff_scroll(&mut self, e: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(px(ROW_HEIGHT));
        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
        let now = std::time::Instant::now();
        let Some(tab) = self.active.and_then(|a| self.tabs.get(a)) else {
            return;
        };
        let base = tab.scroll.0.borrow().base_handle.clone();
        // A new gesture: a trackpad touch starts, or the wheel was idle for a moment.
        let idle = self
            .scroll_axis
            .is_none_or(|(_, at)| now.duration_since(at) > SCROLL_GESTURE_GAP);
        if matches!(e.touch_phase, gpui::TouchPhase::Started) || idle {
            self.scroll_axis = None;
        }
        let horizontal = match self.scroll_axis {
            Some((axis, _)) => axis,
            None if dx == 0. && dy == 0. => return,
            None => {
                // gpui scrolls the list only along the larger axis of an event,
                // so a sideways first event has not moved it vertically.
                self.lock_y = f32::from(base.offset().y).clamp(-self.max_v_scroll(), 0.);
                dx.abs() > dy.abs()
            }
        };
        self.scroll_axis = Some((horizontal, now));
        if !horizontal {
            return;
        }
        // Hold the vertical position while scrolling sideways.
        let offset = base.offset();
        if f32::from(offset.y) != self.lock_y {
            base.set_offset(gpui::point(offset.x, px(self.lock_y)));
            cx.notify();
        }
        if dx == 0. {
            return;
        }
        // Trackpads report raw pixels; damp them so the code doesn't race away.
        let dx = match e.delta {
            gpui::ScrollDelta::Pixels(_) => dx * TRACKPAD_SPEED,
            gpui::ScrollDelta::Lines(_) => dx,
        };
        let max = self.max_h_scroll();
        if let Some(tab) = self.active.and_then(|a| self.tabs.get_mut(a)) {
            let next = (tab.h_scroll - dx).clamp(0., max);
            if next != tab.h_scroll {
                tab.h_scroll = next;
                cx.notify();
            }
        }
    }

    /// How far the list can scroll down. (`uniform_list` doesn't report a max offset.)
    fn max_v_scroll(&self) -> f32 {
        let (Some(tab), Some(diff)) = (self.active_tab(), self.diff()) else {
            return 0.;
        };
        let view_h = f32::from(tab.scroll.0.borrow().base_handle.bounds().size.height);
        (diff.rows.len() as f32 * ROW_HEIGHT - view_h).max(0.)
    }

    /// Thin scroll position bars over the right and bottom edges of the diff.
    fn render_scroll_indicators(&self, t: &Theme) -> Vec<gpui::AnyElement> {
        let Some(tab) = self.active_tab() else {
            return vec![];
        };
        let base = tab.scroll.0.borrow().base_handle.clone();
        let (bounds, offset) = (base.bounds(), base.offset());
        let (view_w, view_h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let color = t.muted.opacity(0.45);
        let mut bars = Vec::new();
        let max_y = self.max_v_scroll();
        if max_y > 0. && view_h > 0. {
            let thumb = (view_h * view_h / (view_h + max_y)).max(MIN_THUMB);
            let top = (view_h - thumb) * (-f32::from(offset.y) / max_y).clamp(0., 1.);
            bars.push(
                div()
                    .absolute()
                    .right(px(2.))
                    .top(px(top))
                    .w(px(SCROLLBAR))
                    .h(px(thumb))
                    .rounded(px(SCROLLBAR / 2.))
                    .bg(color)
                    .into_any_element(),
            );
        }
        let max_x = self.max_h_scroll();
        if max_x > 0. && view_w > 0. {
            let code = self.code_width().max(1.);
            let thumb = (view_w * code / (code + max_x)).max(MIN_THUMB);
            let left = (view_w - thumb) * (tab.h_scroll / max_x).clamp(0., 1.);
            bars.push(
                div()
                    .absolute()
                    .bottom(px(2.))
                    .left(px(left))
                    .h(px(SCROLLBAR))
                    .w(px(thumb))
                    .rounded(px(SCROLLBAR / 2.))
                    .bg(color)
                    .into_any_element(),
            );
        }
        bars
    }

    fn render_tabs(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = self.active == Some(ix);
            let name = tab.path.rsplit('/').next().unwrap_or(&tab.path).to_string();
            let (badge, badge_color) = file_badge(&name);
            let kind = self
                .list(tab.staged)
                .iter()
                .find(|c| c.path == tab.path)
                .map(|c| c.kind);
            let group: SharedString = format!("tab-{ix}").into();
            let hover = t.hover;
            div()
                .id(SharedString::from(format!(
                    "tab-{}-{}",
                    tab.staged, tab.path
                )))
                .group(group.clone())
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .pl(px(12.))
                .pr(px(6.))
                .border_r_1()
                .border_color(t.border)
                .whitespace_nowrap()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.editor).border_t_2().border_color(t.accent)
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.bg(hover))
                })
                .child(
                    div()
                        .text_size(px(8.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(badge_color)
                        .child(badge),
                )
                .child(div().when(!tab.pinned, |d| d.italic()).child(name))
                .child(
                    div()
                        .text_color(t.muted)
                        .when(!tab.pinned, |d| d.italic())
                        .child(if tab.staged {
                            "(Index)"
                        } else {
                            "(Working Tree)"
                        }),
                )
                .children(kind.map(|k| {
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(12.))
                        .text_color(status_color(k, t))
                        .child(k.letter())
                }))
                .child({
                    let close = icon_button(
                        SharedString::from(format!("close-tab-{ix}")),
                        "close",
                        if active { t.fg } else { t.muted },
                        cx,
                        cx.listener(move |this, _, window, cx| {
                            this.request_close_tab(ix, window, cx)
                        }),
                    )
                    .w(px(18.))
                    .h(px(18.))
                    .tooltip(tooltip(format!(
                        "Close ({}+W)",
                        super::workspace::secondary_key()
                    )));
                    if tab.draft.is_some() {
                        // Unsaved: a dot that turns into the close button on hover.
                        div()
                            .relative()
                            .w(px(18.))
                            .h(px(18.))
                            .flex_none()
                            .child(
                                div()
                                    .absolute()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .group_hover(group.clone(), |s| s.invisible())
                                    .child(div().size(px(8.)).rounded_full().bg(if active {
                                        t.fg
                                    } else {
                                        t.muted
                                    })),
                            )
                            .child(
                                close
                                    .absolute()
                                    .invisible()
                                    .group_hover(group.clone(), |s| s.visible()),
                            )
                            .into_any_element()
                    } else if active {
                        close.into_any_element()
                    } else {
                        close
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .into_any_element()
                    }
                })
                .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                    this.activate_tab(ix, cx);
                    if e.click_count() >= 2 {
                        this.pin_tab(ix, cx);
                    }
                }))
                .on_mouse_up(
                    gpui::MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.request_close_tab(ix, window, cx)),
                )
        });
        div()
            .id("tabs")
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .overflow_x_scroll()
            .children(tabs)
    }

    fn render_diff_header(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(diff) = self.diff() else {
            return div();
        };
        let layout = self.layout;
        let path = diff.change.path.clone();
        let staged = diff.change.staged;
        let fg = t.fg;

        let seg = |id: &'static str,
                   icon_name: &'static str,
                   label: &'static str,
                   on: bool,
                   target: Layout,
                   cx: &mut Context<Self>| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .cursor_pointer()
                .when(on, |d| d.bg(t.sel).text_color(t.sel_fg))
                .when(!on, |d| d.text_color(t.muted))
                .child(icon(icon_name, if on { t.sel_fg } else { t.muted }))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_layout(target, cx)))
        };

        let p_open = path.clone();
        let p_stage = path.clone();
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .bg(t.panel)
            .border_b_1()
            .border_color(t.border)
            .child(self.render_tabs(t, cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .mr(px(6.))
                            .child(format!("+{} −{}", diff.file.added, diff.file.removed)),
                    )
                    .when(self.active_tab().is_some_and(|t| t.draft.is_some()), |d| {
                        let button = |id: &'static str, label: &'static str, primary: bool| {
                            div()
                                .id(id)
                                .h(px(22.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .text_size(px(12.))
                                .cursor_pointer()
                                .when(primary, |d| d.bg(t.accent).text_color(t.on_accent))
                                .when(!primary, |d| {
                                    d.border_1().border_color(t.border).text_color(t.fg)
                                })
                                .hover(|s| s.opacity(0.85))
                                .child(label)
                        };
                        d.child(
                            button("discard-draft", "Discard", false)
                                .tooltip(tooltip("Drop unsaved edits"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(a) = this.active {
                                        this.discard_draft(a, cx)
                                    }
                                })),
                        )
                        .child(
                            button("save-draft", "Save", true)
                                .mr(px(8.))
                                .tooltip(tooltip(format!(
                                    "Write these edits to disk ({}+S)",
                                    super::workspace::secondary_key()
                                )))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(a) = this.active {
                                        this.save_tab(a, window, cx)
                                    }
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .mr(px(4.))
                            .border_1()
                            .border_color(t.border)
                            .rounded(px(5.))
                            .overflow_hidden()
                            .child(seg(
                                "seg-sbs",
                                "split",
                                "Side by side",
                                layout == Layout::SideBySide,
                                Layout::SideBySide,
                                cx,
                            ))
                            .child(seg(
                                "seg-inl",
                                "inline",
                                "Inline",
                                layout == Layout::Inline,
                                Layout::Inline,
                                cx,
                            )),
                    )
                    .child(
                        icon_button(
                            "prev-change",
                            "arrow-up",
                            fg,
                            cx,
                            cx.listener(|this, _, _, cx| this.jump_change(false, cx)),
                        )
                        .tooltip(tooltip("Previous Change (Shift+F7)")),
                    )
                    .child(
                        icon_button(
                            "next-change",
                            "arrow-down",
                            fg,
                            cx,
                            cx.listener(|this, _, _, cx| this.jump_change(true, cx)),
                        )
                        .tooltip(tooltip("Next Change (F7)")),
                    )
                    .child(
                        icon_button(
                            "open-current",
                            "open",
                            fg,
                            cx,
                            cx.listener(move |this, _, _, cx| this.open_file(&p_open, cx)),
                        )
                        .tooltip(tooltip("Open File")),
                    )
                    .child(if staged {
                        icon_button(
                            "unstage-current",
                            "minus",
                            fg,
                            cx,
                            cx.listener(move |this, _, _, cx| {
                                this.unstage(vec![p_stage.clone()], cx)
                            }),
                        )
                        .tooltip(tooltip("Unstage File"))
                    } else {
                        icon_button(
                            "stage-current",
                            "plus",
                            fg,
                            cx,
                            cx.listener(move |this, _, _, cx| {
                                this.stage(vec![p_stage.clone()], cx)
                            }),
                        )
                        .tooltip(tooltip("Stage File"))
                    }),
            )
    }
}

/// Width of the column between the two sides that holds the merge arrows.
const GUTTER: f32 = 34.;
/// Font size of code lines.
const CODE_SIZE: f32 = 12.5;
/// Thickness of the scroll position bars.
const SCROLLBAR: f32 = 6.;
const MIN_THUMB: f32 = 24.;
/// Scale applied to trackpad pixel deltas when scrolling sideways.
const TRACKPAD_SPEED: f32 = 0.6;
/// Events further apart than this start a new scroll gesture.
const SCROLL_GESTURE_GAP: std::time::Duration = std::time::Duration::from_millis(180);

/// The merge arrows for the first row of a change block.
fn merge_buttons(
    block: usize,
    arrows: (bool, bool),
    staged: bool,
    t: &Theme,
    cx: &mut Context<Workspace>,
) -> gpui::Div {
    let button = |id: String,
                  name: &'static str,
                  tip: &'static str,
                  dir: MergeDirection,
                  cx: &mut Context<Workspace>| {
        let hover = t.hover;
        div()
            .id(SharedString::from(id))
            .w(px(15.))
            .h(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(super::widgets::icon_sized(name, 13., t.accent))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.merge_block(block, dir, cx)
            }))
    };
    let (to_right, to_left) = arrows;
    let right_tip = if staged {
        "Unstage this change"
    } else {
        "Revert this change in the working tree"
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .when(to_right, |d| {
            d.child(button(
                format!("merge-r-{block}"),
                "arrow-right",
                right_tip,
                MergeDirection::ToRight,
                cx,
            ))
        })
        .when(to_left, |d| {
            d.child(button(
                format!("merge-l-{block}"),
                "arrow-left",
                "Stage this change",
                MergeDirection::ToLeft,
                cx,
            ))
        })
}

fn breadcrumbs(path: &str, t: &Theme) -> Vec<gpui::AnyElement> {
    let mut out = Vec::new();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(icon("chevron-right", t.faint).into_any_element());
        }
        let last = i + 1 == parts.len();
        out.push(
            div()
                .when(last, |d| d.text_color(t.fg))
                .child(part.to_string())
                .into_any_element(),
        );
    }
    out
}

fn column_head(icon_name: &str, label: &str, t: &Theme) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(4.))
        .child(icon(icon_name, t.muted))
        .child(label.to_string())
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    ix: usize,
    row: &Row,
    diff: &DiffState,
    arrows: (bool, bool),
    offset: f32,
    t: &Theme,
    fonts: &(SharedString, SharedString),
    cx: &mut Context<Workspace>,
) -> gpui::AnyElement {
    let (mono, ui) = fonts;
    let base = div()
        .id(ix)
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_none()
        .font_family(mono.clone())
        .text_size(px(CODE_SIZE));
    match row {
        Row::Hunk { header } => base
            .items_center()
            .pl(px(14.))
            .bg(t.hunk)
            .text_color(t.hunk_fg)
            .text_size(px(12.))
            .child(header.clone())
            .into_any_element(),
        Row::Fold { id, hidden } => {
            let id = *id;
            let border = t.border;
            base.items_center()
                .font_family(ui.clone())
                .justify_center()
                .gap(px(6.))
                .text_color(t.muted)
                .text_size(px(12.))
                .border_t_1()
                .border_b_1()
                .border_color(border)
                .cursor_pointer()
                .hover(|s| s.text_color(t.fg))
                .child(icon("chevron-down", t.muted))
                .child(format!(
                    "{hidden} unchanged line{} hidden",
                    if *hidden == 1 { "" } else { "s" }
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.expand_fold(id, cx)))
                .into_any_element()
        }
        Row::Pair { left, right, block } => {
            let cursor_new = matches!(diff.cursor, Some(CursorLine::New(n)) if right.as_ref().is_some_and(|s| s.line == n));
            let cursor_old = right.is_none()
                && matches!(diff.cursor, Some(CursorLine::Old(o)) if left.as_ref().is_some_and(|s| s.line == o));
            let click_target = right
                .as_ref()
                .map(|s| CursorLine::New(s.line))
                .or(left.as_ref().map(|s| CursorLine::Old(s.line)));
            let gutter = div()
                .w(px(GUTTER))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_r_1()
                .border_color(t.border)
                .when_some(*block, |d, b| {
                    d.child(merge_buttons(b, arrows, diff.change.staged, t, cx))
                });
            base.child(half(left.as_ref(), cursor_old, None, diff.lang, offset, t))
                .child(gutter)
                .child(half(
                    right.as_ref(),
                    cursor_new,
                    blame_ghost(diff, cursor_new, right.as_ref()),
                    diff.lang,
                    offset,
                    t,
                ))
                .when_some(click_target, |d, target| {
                    d.on_click(cx.listener(move |this, _, _, cx| this.set_cursor(target, cx)))
                })
                .into_any_element()
        }
        Row::Line {
            old,
            new,
            side,
            block,
        } => {
            let is_cursor = match diff.cursor {
                Some(CursorLine::New(n)) => *new == Some(n),
                Some(CursorLine::Old(o)) => new.is_none() && *old == Some(o),
                None => false,
            };
            let target = new.map(CursorLine::New).or(old.map(CursorLine::Old));
            let ghost = if new.is_some() {
                blame_ghost(diff, is_cursor, Some(side))
            } else {
                None
            };
            let (line_bg, sign) = kind_style(side.kind, t);
            let num = |n: Option<usize>| {
                div()
                    .w(px(44.))
                    .flex_none()
                    .text_right()
                    .pr(px(8.))
                    .text_color(if is_cursor { t.fg } else { t.faint })
                    .child(n.map(|n| n.to_string()).unwrap_or_default())
            };
            base.when_some(line_bg, |d, bg| d.bg(bg))
                .when(is_cursor && side.kind == LineKind::Context, |d| {
                    d.bg(t.cursor_line)
                })
                .child(num(*old))
                .child(num(*new))
                .child(
                    div()
                        .w(px(16.))
                        .flex_none()
                        .text_center()
                        .text_color(t.faint)
                        .child(sign),
                )
                .child(code(side, diff.lang, t, ghost, t.ghost, offset))
                .when_some(*block, |d, b| {
                    d.child(
                        merge_buttons(b, arrows, diff.change.staged, t, cx)
                            .flex_none()
                            .px(px(6.)),
                    )
                })
                .when_some(target, |d, target| {
                    d.on_click(cx.listener(move |this, _, _, cx| this.set_cursor(target, cx)))
                })
                .into_any_element()
        }
    }
}

fn blame_ghost(diff: &DiffState, is_cursor: bool, side: Option<&Side>) -> Option<String> {
    if !is_cursor {
        return None;
    }
    let side = side?;
    let blame = diff.blame.as_ref()?.line(side.line)?;
    Some(format!(
        "{}, {} · {}",
        blame.author,
        blame.relative_time(),
        blame.summary
    ))
}

fn kind_style(kind: LineKind, t: &Theme) -> (Option<Hsla>, &'static str) {
    match kind {
        LineKind::Context => (None, ""),
        LineKind::Removed => (Some(t.del_line), "−"),
        LineKind::Added => (Some(t.add_line), "+"),
    }
}

/// One side of a side-by-side row: line number, sign and code.
fn half(
    side: Option<&Side>,
    is_cursor: bool,
    ghost: Option<String>,
    lang: Lang,
    offset: f32,
    t: &Theme,
) -> gpui::Div {
    let Some(side) = side else {
        return div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(pattern_slash(t.filler, 4.0, 4.0));
    };
    let (bg, sign) = kind_style(side.kind, t);
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .overflow_hidden()
        .when_some(bg, |d, bg| d.bg(bg))
        .when(is_cursor && side.kind == LineKind::Context, |d| {
            d.bg(t.cursor_line)
        })
        .child(
            div()
                .w(px(48.))
                .flex_none()
                .text_right()
                .pr(px(8.))
                .text_color(if is_cursor { t.fg } else { t.faint })
                .child(side.line.to_string()),
        )
        .child(
            div()
                .w(px(16.))
                .flex_none()
                .text_center()
                .text_color(t.faint)
                .child(sign),
        )
        .child(code(side, lang, t, ghost, t.ghost, offset))
}

fn code(
    side: &Side,
    lang: Lang,
    t: &Theme,
    ghost: Option<String>,
    ghost_color: Hsla,
    offset: f32,
) -> gpui::Div {
    let chg_bg = match side.kind {
        LineKind::Added => Some(t.add_chg),
        LineKind::Removed => Some(t.del_chg),
        LineKind::Context => None,
    };
    let highlights = merge_highlights(
        &side.text,
        &syntax::highlight(&side.text, lang),
        &side.changes,
        chg_bg,
        t,
    );
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .overflow_hidden()
        .whitespace_nowrap()
        .pr(px(12.))
        .child(
            div()
                .flex_none()
                .when(offset > 0., |d| d.ml(px(-offset)))
                .child(StyledText::new(side.text.clone()).with_highlights(highlights)),
        )
        .when_some(ghost, |d, g| {
            d.child(
                div()
                    .flex_none()
                    .pl(px(28.))
                    .italic()
                    .text_size(px(12.))
                    .text_color(ghost_color)
                    .child(g),
            )
        })
}

/// Combines syntax colors with intra-line change backgrounds into sorted,
/// non-overlapping highlight ranges.
fn merge_highlights(
    text: &str,
    tokens: &[(Range<usize>, syntax::Token)],
    changes: &[Range<usize>],
    chg_bg: Option<Hsla>,
    t: &Theme,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for (r, _) in tokens {
        cuts.extend([r.start, r.end]);
    }
    for r in changes {
        cuts.extend([r.start.min(text.len()), r.end.min(text.len())]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b || !text.is_char_boundary(a) || !text.is_char_boundary(b) {
            continue;
        }
        let token = tokens
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, k)| *k);
        let changed = changes.iter().any(|r| r.start <= a && b <= r.end);
        if token.is_none() && !changed {
            continue;
        }
        let style = HighlightStyle {
            color: token.map(|k| t.token(k)),
            font_style: (token == Some(syntax::Token::Comment)).then_some(FontStyle::Italic),
            background_color: if changed { chg_bg } else { None },
            ..Default::default()
        };
        out.push((a..b, style));
    }
    out
}

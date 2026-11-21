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
        _window: &mut Window,
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
            let note = if diff.old_text.is_empty() && diff.new_text.is_empty() {
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
        pane.child(
            uniform_list(
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
                    range
                        .filter_map(|ix| diff.rows.get(ix).map(|row| (ix, row)))
                        .map(|(ix, row)| render_row(ix, row, diff, arrows, &t, &fonts, cx))
                        .collect()
                }),
            )
            .track_scroll(scroll)
            .flex_1()
            .min_h_0(),
        )
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
                        cx.listener(move |this, _, _, cx| this.close_tab(ix, cx)),
                    )
                    .w(px(18.))
                    .h(px(18.))
                    .tooltip(tooltip(format!(
                        "Close ({}+W)",
                        super::workspace::secondary_key()
                    )));
                    if active {
                        close
                    } else {
                        close
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
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
                    cx.listener(move |this, _, _, cx| this.close_tab(ix, cx)),
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

fn render_row(
    ix: usize,
    row: &Row,
    diff: &DiffState,
    arrows: (bool, bool),
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
        .text_size(px(12.5));
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
            base.child(half(left.as_ref(), cursor_old, None, diff.lang, t))
                .child(gutter)
                .child(half(
                    right.as_ref(),
                    cursor_new,
                    blame_ghost(diff, cursor_new, right.as_ref()),
                    diff.lang,
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
                .child(code(side, diff.lang, t, ghost, t.ghost))
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
        .child(code(side, lang, t, ghost, t.ghost))
}

fn code(side: &Side, lang: Lang, t: &Theme, ghost: Option<String>, ghost_color: Hsla) -> gpui::Div {
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

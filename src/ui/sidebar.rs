//! The changes pane: commit box, Commit button with its menu, and the
//! Staged Changes / Changes groups.

use gpui::{
    anchored, deferred, div, point, prelude::*, px, Context, MouseButton, SharedString, Window,
};

use super::widgets::{icon, icon_button, tooltip};
use super::workspace::{CommitMode, Workspace};
use crate::git::{ChangeKind, FileChange};
use crate::theme::{ActiveTheme as _, Theme};

impl Workspace {
    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        div()
            .w(px(300.))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .child(self.render_sidebar_header(&t, cx))
            .child(self.render_commit_box(&t, window, cx))
            .children(self.error.clone().map(|e| {
                div()
                    .mx(px(12.))
                    .mb(px(8.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.error.opacity(0.5))
                    .text_size(px(12.))
                    .text_color(t.error)
                    .child(e)
            }))
            .child(
                div()
                    .id("groups")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(8.))
                    .when(!self.staged.is_empty(), |d| {
                        d.child(self.render_group(true, &t, cx))
                    })
                    .child(self.render_group(false, &t, cx)),
            )
    }

    fn render_sidebar_header(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(14.))
            .pr(px(8.))
            .gap(px(2.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("SOURCE CONTROL"),
            )
            .child(
                icon_button(
                    "hdr-commit",
                    "check",
                    t.fg,
                    cx,
                    cx.listener(|this, _, window, cx| this.commit(CommitMode::Staged, window, cx)),
                )
                .tooltip(tooltip("Commit")),
            )
            .child(
                icon_button(
                    "hdr-refresh",
                    "refresh",
                    t.fg,
                    cx,
                    cx.listener(|this, _, _, cx| this.refresh(cx)),
                )
                .tooltip(tooltip("Refresh")),
            )
    }

    fn render_commit_box(
        &mut self,
        t: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.busy.clone();
        let menu_open = self.commit_menu_open;
        let commit_label: SharedString = busy
            .clone()
            .map(SharedString::from)
            .unwrap_or("Commit".into());
        let accent_hover = t.accent_hover;
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(12.))
            .pt(px(2.))
            .pb(px(10.))
            .child(
                div()
                    .flex()
                    .bg(t.input)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(5.))
                    .px(px(8.))
                    .py(px(6.))
                    .line_height(px(18.))
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .relative()
                    .h(px(28.))
                    .flex()
                    .rounded(px(4.))
                    .bg(t.accent)
                    .text_color(t.on_accent)
                    .child(
                        div()
                            .id("commit")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .rounded_l(px(4.))
                            .cursor_pointer()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .hover(move |s| s.bg(accent_hover))
                            .when(busy.is_none(), |d| d.child(icon("check", t.on_accent)))
                            .child(commit_label)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.commit(CommitMode::Staged, window, cx)
                            })),
                    )
                    .child(div().w(px(1.)).my(px(4.)).bg(t.on_accent.opacity(0.35)))
                    .child(
                        div()
                            .id("commit-more")
                            .w(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_r(px(4.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(accent_hover))
                            .child(icon("chevron-down", t.on_accent))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.commit_menu_open = !this.commit_menu_open;
                                cx.notify();
                            })),
                    )
                    .when(menu_open, |d| d.child(self.render_commit_menu(t, cx))),
            )
    }

    fn render_commit_menu(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let item =
            |id: &'static str, label: &'static str, mode: CommitMode, cx: &mut Context<Self>| {
                let hover = t.sel;
                let hover_fg = t.sel_fg;
                div()
                    .id(id)
                    .px(px(10.))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover).text_color(hover_fg))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| this.commit(mode, window, cx)))
            };
        deferred(
            anchored()
                .position_mode(gpui::AnchoredPositionMode::Local)
                .offset(point(px(0.), px(32.)))
                .child(
                    div()
                        .w(px(276.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .bg(t.panel)
                        .text_color(t.fg)
                        .border_1()
                        .border_color(t.border)
                        .rounded(px(6.))
                        .shadow_lg()
                        .child(item("m-commit", "Commit Staged", CommitMode::Staged, cx))
                        .child(item("m-all", "Commit All", CommitMode::All, cx))
                        .child(item(
                            "m-amend",
                            "Commit Staged (Amend)",
                            CommitMode::Amend,
                            cx,
                        ))
                        .child(item("m-push", "Commit & Push", CommitMode::AndPush, cx)),
                ),
        )
        .with_priority(1)
    }

    fn render_group(&self, staged: bool, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let list = if staged { &self.staged } else { &self.unstaged };
        let open = if staged {
            self.staged_open
        } else {
            self.changes_open
        };
        let title = if staged { "Staged Changes" } else { "Changes" };
        let paths: Vec<String> = list.iter().map(|c| c.path.clone()).collect();
        let changes: Vec<FileChange> = list.clone();
        let fg = t.fg;
        let hover = t.hover;

        let mut actions = div()
            .flex()
            .gap(px(1.))
            .invisible()
            .group_hover("group-h", |s| s.visible());
        if staged {
            let p = paths.clone();
            actions = actions.child(
                icon_button(
                    "unstage-all",
                    "minus",
                    fg,
                    cx,
                    cx.listener(move |this, _, _, cx| this.unstage(p.clone(), cx)),
                )
                .tooltip(tooltip("Unstage All Changes")),
            );
        } else {
            let p = paths.clone();
            actions = actions
                .child(
                    icon_button(
                        "discard-all",
                        "discard",
                        fg,
                        cx,
                        cx.listener(move |this, _, window, cx| {
                            this.discard(changes.clone(), window, cx)
                        }),
                    )
                    .tooltip(tooltip("Discard All Changes")),
                )
                .child(
                    icon_button(
                        "stage-all",
                        "plus",
                        fg,
                        cx,
                        cx.listener(move |this, _, _, cx| this.stage(p.clone(), cx)),
                    )
                    .tooltip(tooltip("Stage All Changes")),
                );
        }

        let header = div()
            .id(if staged { "staged-h" } else { "changes-h" })
            .group("group-h")
            .h(px(24.))
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(6.))
            .pr(px(10.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_size(px(12.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if staged {
                    this.staged_open = !this.staged_open
                } else {
                    this.changes_open = !this.changes_open
                }
                cx.notify();
            }))
            .child(icon(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                fg,
            ))
            .child(div().flex_1().child(title))
            .when(!list.is_empty(), |d| d.child(actions))
            .child(
                div()
                    .min_w(px(18.))
                    .h(px(18.))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .bg(t.badge)
                    .text_color(t.badge_fg)
                    .text_size(px(10.))
                    .child(list.len().to_string()),
            );

        let mut group = div().flex().flex_col().child(header);
        if open {
            if list.is_empty() && !staged {
                group = group.child(
                    div()
                        .pl(px(24.))
                        .pr(px(10.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .text_color(t.muted)
                        .child(if self.staged.is_empty() {
                            "No changes in the working tree."
                        } else {
                            "No unstaged changes."
                        }),
                );
            }
            for change in list {
                group = group.child(self.render_file_row(change, t, cx));
            }
        }
        group
    }

    fn render_file_row(
        &self,
        change: &FileChange,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .selected
            .as_ref()
            .is_some_and(|(s, p)| *s == change.staged && *p == change.path);
        let (bg, fg) = if selected {
            (Some(t.sel), t.sel_fg)
        } else {
            (None, t.fg)
        };
        let hover = t.hover;
        let status_color = status_color(change.kind, t);
        let row_group: SharedString = format!("row-{}-{}", change.staged, change.path).into();
        let path = change.path.clone();
        let staged = change.staged;
        let deleted = change.kind == ChangeKind::Deleted;

        let mut actions = div().flex().gap(px(1.));
        if !selected {
            actions = actions
                .invisible()
                .group_hover(row_group.clone(), |s| s.visible());
        }
        {
            let p = path.clone();
            actions = actions.child(
                icon_button(
                    SharedString::from(format!("open-{row_group}")),
                    "open",
                    fg,
                    cx,
                    cx.listener(move |this, _, _, cx| this.open_file(&p, cx)),
                )
                .tooltip(tooltip("Open File")),
            );
        }
        if staged {
            let p = path.clone();
            actions = actions.child(
                icon_button(
                    SharedString::from(format!("unstage-{row_group}")),
                    "minus",
                    fg,
                    cx,
                    cx.listener(move |this, _, _, cx| this.unstage(vec![p.clone()], cx)),
                )
                .tooltip(tooltip("Unstage Changes")),
            );
        } else {
            let c = change.clone();
            let p = path.clone();
            actions = actions
                .child(
                    icon_button(
                        SharedString::from(format!("discard-{row_group}")),
                        "discard",
                        fg,
                        cx,
                        cx.listener(move |this, _, window, cx| {
                            this.discard(vec![c.clone()], window, cx)
                        }),
                    )
                    .tooltip(tooltip("Discard Changes")),
                )
                .child(
                    icon_button(
                        SharedString::from(format!("stage-{row_group}")),
                        "plus",
                        fg,
                        cx,
                        cx.listener(move |this, _, _, cx| this.stage(vec![p.clone()], cx)),
                    )
                    .tooltip(tooltip("Stage Changes")),
                );
        }

        let (badge, badge_color) = file_badge(change.file_name());
        div()
            .id(SharedString::from(format!("file-{row_group}")))
            .group(row_group.clone())
            .h(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .pl(px(24.))
            .pr(px(10.))
            .cursor_pointer()
            .text_color(fg)
            .when_some(bg, |d, bg| d.bg(bg))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.select(staged, path.clone(), cx)))
            .child(
                div()
                    .w(px(16.))
                    .flex_none()
                    .text_size(px(8.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(badge_color)
                    .child(badge),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .when(deleted, |d| d.line_through())
                    .child(change.file_name().to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(if selected { fg.opacity(0.7) } else { t.muted })
                    .child(change.dir().to_string()),
            )
            .child(actions)
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_center()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(status_color)
                    .child(change.kind.letter()),
            )
    }
}

pub(super) fn status_color(kind: ChangeKind, t: &Theme) -> gpui::Hsla {
    match kind {
        ChangeKind::Modified | ChangeKind::Renamed => t.st_modified,
        ChangeKind::Added => t.st_added,
        ChangeKind::Deleted | ChangeKind::Conflicted => t.st_deleted,
        ChangeKind::Untracked => t.st_untracked,
    }
}

/// A two-letter file-type badge and its color, like the mock's file icons.
pub(super) fn file_badge(name: &str) -> (String, gpui::Hsla) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let color = |hex: u32| -> gpui::Hsla { gpui::rgb(hex).into() };
    match ext.as_str() {
        "rs" => ("rs".into(), color(0xdea584)),
        "toml" => ("tm".into(), color(0x9c6b4e)),
        "svg" => ("sv".into(), color(0xe2a03f)),
        "md" => ("md".into(), color(0x519aba)),
        "json" => ("{}".into(), color(0xcbcb41)),
        "js" | "mjs" | "cjs" | "jsx" => ("js".into(), color(0xcbcb41)),
        "ts" | "tsx" => ("ts".into(), color(0x519aba)),
        "py" => ("py".into(), color(0x519aba)),
        "go" => ("go".into(), color(0x519aba)),
        "lock" => ("lk".into(), color(0x8a8a8a)),
        "yml" | "yaml" => ("ym".into(), color(0xa074c4)),
        _ => {
            let short: String = ext.chars().take(2).collect();
            (
                if short.is_empty() || name.starts_with('.') {
                    "··".into()
                } else {
                    short
                },
                color(0x8a8a8a),
            )
        }
    }
}

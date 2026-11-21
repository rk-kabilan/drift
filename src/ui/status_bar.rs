//! The status bar: repository, branch, ahead/behind, blame for the cursor line.

use std::time::Duration;

use gpui::{
    div, percentage, prelude::*, px, svg, Animation, AnimationExt as _, Context, Transformation,
    Window,
};

use super::widgets::{icon_sized, tooltip};
use super::workspace::{CursorLine, RemoteOp, Workspace};
use crate::syntax::Lang;
use crate::theme::{ActiveTheme as _, Appearance};

impl Workspace {
    pub(super) fn render_status_bar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        let fg = t.on_accent;
        let hover = gpui::white().opacity(0.12);
        let item = |id: &'static str| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .whitespace_nowrap()
                .hover(move |s| s.bg(hover))
        };

        let mut bar = div()
            .h(px(24.))
            .flex_none()
            .flex()
            .bg(t.accent)
            .text_color(fg)
            .text_size(px(12.));

        if self.repo.is_none() {
            return bar;
        }

        let head = &self.head;
        bar = bar
            .child(
                item("sb-repo")
                    .child(icon_sized("repo", 14., fg))
                    .child(head.repo_name.clone())
                    .tooltip(tooltip(self.root.display().to_string())),
            )
            .child(
                item("sb-branch")
                    .cursor_pointer()
                    .child(icon_sized("branch", 14., fg))
                    .child(format!(
                        "{}{}",
                        head.branch,
                        if head.dirty { "*" } else { "" }
                    ))
                    .tooltip(tooltip(format!(
                        "Switch branch ({}+Shift+B)",
                        super::workspace::secondary_key()
                    )))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_branch_picker(window, cx)),
                    ),
            );

        // Sync: a spinning icon and a short status while fetch/pull/push runs.
        let spinning = self.busy.is_some();
        let sync_icon = svg()
            .path("icons/sync.svg")
            .size(px(14.))
            .flex_none()
            .text_color(fg);
        // Only animate while busy, so an idle window doesn't redraw every frame.
        let sync_icon = if spinning {
            sync_icon
                .with_animation(
                    "sync-spin",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |svg, delta| {
                        // Counter-clockwise, matching the arrows in the icon.
                        svg.with_transformation(Transformation::rotate(percentage(1.0 - delta)))
                    },
                )
                .into_any_element()
        } else {
            sync_icon.into_any_element()
        };
        let (label, tip) = match (&self.busy, head.has_upstream, head.short_sha.is_some()) {
            (Some(busy), _, _) => (busy.clone(), busy.clone()),
            (None, true, _) => (
                format!("{}↓ {}↑", head.behind, head.ahead),
                format!(
                    "Sync: {} commit{} to pull, {} to push. Click to fetch, pull and push.",
                    head.behind,
                    if head.behind == 1 { "" } else { "s" },
                    head.ahead
                ),
            ),
            (None, false, true) => (
                "Publish branch".to_string(),
                "No upstream branch. Click to push it to the remote.".to_string(),
            ),
            (None, false, false) => (String::new(), String::new()),
        };
        if !label.is_empty() {
            bar = bar.child(
                item("sb-sync")
                    .when(!spinning, |d| d.cursor_pointer())
                    .child(sync_icon)
                    .child(label)
                    .tooltip(tooltip(tip))
                    .when(!spinning, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.remote(RemoteOp::Sync, cx)))
                    }),
            );
        }
        if let Some(note) = self.status_note {
            bar = bar.child(
                item("sb-note")
                    .opacity(0.9)
                    .child(icon_sized("check", 14., fg))
                    .child(note),
            );
        }
        bar = bar.child(div().flex_1());

        if let Some(diff) = self.diff() {
            if let (true, Some(CursorLine::New(line))) = (self.show_blame, diff.cursor) {
                if let Some(b) = diff.blame.as_ref().and_then(|b| b.line(line)) {
                    bar = bar.child(
                        item("sb-blame")
                            .max_w(px(460.))
                            .overflow_hidden()
                            .child(icon_sized("commit", 14., fg))
                            .child(div().overflow_hidden().text_ellipsis().child(format!(
                                "{}, {} · {} ({})",
                                b.author,
                                b.relative_time(),
                                b.summary,
                                b.short_sha
                            )))
                            .tooltip(tooltip(format!("Blame for line {line}"))),
                    );
                } else {
                    bar = bar.child(item("sb-blame").opacity(0.85).child("Not committed yet"));
                }
            }
            let pos = match diff.cursor {
                Some(CursorLine::New(n)) => format!("Ln {n}"),
                Some(CursorLine::Old(n)) => format!("Ln {n} (HEAD)"),
                None => String::new(),
            };
            if !pos.is_empty() {
                bar = bar.child(item("sb-pos").child(pos));
            }
            bar = bar.child(item("sb-lang").child(Lang::name_for_path(&diff.change.path)));
        }
        let theme_icon = if t.appearance == Appearance::Dark {
            "sun"
        } else {
            "moon"
        };
        bar.child(
            item("sb-theme")
                .cursor_pointer()
                .child(icon_sized(theme_icon, 14., fg))
                .tooltip(tooltip("Toggle light/dark theme"))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
        )
    }
}

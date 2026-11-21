//! The status bar: repository, branch, ahead/behind, blame for the cursor line.

use gpui::{div, prelude::*, px, Context, Window};

use super::widgets::{icon_sized, tooltip};
use super::workspace::{CursorLine, Workspace};
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
                    .child(icon_sized("branch", 14., fg))
                    .child(format!(
                        "{}{}",
                        head.branch,
                        if head.dirty { "*" } else { "" }
                    )),
            );
        if head.has_upstream {
            let tip = format!(
                "Synchronize: {} commit{} to pull, {} to push",
                head.behind,
                if head.behind == 1 { "" } else { "s" },
                head.ahead
            );
            bar = bar.child(
                item("sb-sync")
                    .cursor_pointer()
                    .child(icon_sized("sync", 14., fg))
                    .child(format!("{}↓ {}↑", head.behind, head.ahead))
                    .tooltip(tooltip(tip))
                    .on_click(cx.listener(|this, _, _, cx| this.sync(cx))),
            );
        } else if head.short_sha.is_some() {
            bar = bar.child(
                item("sb-publish")
                    .cursor_pointer()
                    .child(icon_sized("sync", 14., fg))
                    .child("Publish")
                    .tooltip(tooltip(
                        "No upstream branch. Push to set one up with `git push -u`.",
                    )),
            );
        }
        if let Some(busy) = &self.busy {
            bar = bar.child(item("sb-busy").child(busy.clone()));
        }
        bar = bar.child(div().flex_1());

        if let Some(diff) = &self.diff {
            if let Some(CursorLine::New(line)) = diff.cursor {
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

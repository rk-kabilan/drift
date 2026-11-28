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
    ) -> gpui::Div {
        let t = cx.theme().clone();
        let fg = t.on_accent;

        let mut bar = div()
            .h(px(24.))
            .flex_none()
            .flex()
            .bg(t.accent)
            .text_color(fg)
            .text_size(px(12.));

        if self.repo.is_none() {
            // The welcome screen keeps the app-wide items on the right.
            bar = bar.child(div().flex_1());
            return self.status_bar_app_items(bar, cx);
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
                        "Switch branch{}",
                        super::shortcuts::hint("SwitchBranch", cx)
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
        let tip = if self.busy.is_none() && !label.is_empty() {
            let settings = cx.global::<crate::settings::Settings>();
            match (&self.auto_fetch_error, settings.auto_fetch) {
                (_, false) => format!("{tip}\nAuto fetch is off."),
                (Some(e), true) => format!("{tip}\nAuto fetch failed: {e}"),
                (None, true) => format!(
                    "{tip}\nFetching automatically every {}.",
                    crate::settings::interval_label(settings.auto_fetch_interval)
                ),
            }
        } else {
            tip
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
        if let Some((level, text)) = self.panel.alert.clone() {
            let (icon, tip) = match level {
                crate::output::Level::Error => ("error", "Show the error in Output"),
                _ => ("warning", "Show in Output"),
            };
            bar = bar.child(
                item("sb-alert")
                    .cursor_pointer()
                    .max_w(px(520.))
                    .overflow_hidden()
                    .bg(gpui::black().opacity(0.18))
                    .child(icon_sized(icon, 14., fg))
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(text),
                    )
                    .tooltip(tooltip(format!(
                        "{tip}{}",
                        super::shortcuts::hint("ShowOutput", cx)
                    )))
                    .on_click(cx.listener(|this, _, window, cx| this.show_output(window, cx))),
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
            if self.show_blame {
                if let Some((author, when, tip)) = self.blame_for_status(diff) {
                    bar = bar.child(
                        item("sb-blame")
                            .max_w(px(420.))
                            .overflow_hidden()
                            .child(icon_sized("account", 14., fg))
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(author),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .opacity(0.85)
                                    .child(when),
                            )
                            .tooltip(tooltip(tip)),
                    );
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
        self.status_bar_app_items(bar, cx)
    }

    /// Who last changed the cursor line: (author, "· 3 days ago", tooltip).
    /// For a line not committed yet that is you (`user.name`); for a file from
    /// the history graph, the commit's author.
    fn blame_for_status(
        &self,
        diff: &super::workspace::DiffState,
    ) -> Option<(String, String, String)> {
        if let Some(c) = self.active_tab().and_then(|t| t.commit.as_ref()) {
            let age = crate::git::relative_time_since(c.time);
            return Some((
                c.author.clone(),
                format!("· {age}"),
                format!("{} ({}), {age}", c.summary, c.short_sha),
            ));
        }
        let Some(CursorLine::New(line)) = diff.cursor else {
            return None;
        };
        match diff.blame.as_ref().and_then(|b| b.line(line)) {
            Some(b) => Some((
                b.author.clone(),
                format!("· {}", b.relative_time()),
                format!(
                    "Line {line}: {} ({}), {}",
                    b.summary,
                    b.short_sha,
                    b.relative_time()
                ),
            )),
            None => Some((
                self.user_name.clone().unwrap_or_else(|| "You".into()),
                "· Not committed yet".into(),
                format!("Line {line} has changes that are not committed yet"),
            )),
        }
    }

    /// Update check, settings and theme: shown with or without a repository.
    fn status_bar_app_items(&mut self, mut bar: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        let t = cx.theme().clone();
        let fg = t.on_accent;
        if let Some(update) = cx.global::<crate::update::Available>().0.clone() {
            bar = bar.child(
                item("sb-update")
                    .cursor_pointer()
                    .child(icon_sized("update", 14., fg))
                    .child(format!("Drift {} available", update.version))
                    .tooltip(tooltip("Open the download page"))
                    .on_click(move |_, _, cx| cx.open_url(&update.url)),
            );
        } else {
            let checking = self.checking_updates;
            let update_icon = svg()
                .path("icons/update.svg")
                .size(px(14.))
                .flex_none()
                .text_color(fg);
            // A gentle pulse while the check runs.
            let update_icon = if checking {
                update_icon
                    .with_animation(
                        "update-pulse",
                        Animation::new(Duration::from_millis(1000)).repeat(),
                        |svg, delta| svg.opacity(0.35 + 0.65 * (1.0 - (delta * 2.0 - 1.0).abs())),
                    )
                    .into_any_element()
            } else {
                update_icon.into_any_element()
            };
            bar = bar.child(
                item("sb-check-updates")
                    .when(!checking, |d| d.cursor_pointer())
                    .child(update_icon)
                    .tooltip(tooltip(if checking {
                        "Checking for updates…".to_string()
                    } else {
                        format!(
                            "Check for Updates{}",
                            super::shortcuts::hint("CheckForUpdates", cx)
                        )
                    }))
                    .when(!checking, |d| {
                        d.on_click(
                            cx.listener(|this, _, window, cx| this.check_for_updates(window, cx)),
                        )
                    }),
            );
        }
        let theme_icon = if t.appearance == Appearance::Dark {
            "sun"
        } else {
            "moon"
        };
        bar.child(
            item("sb-settings")
                .cursor_pointer()
                .child(icon_sized("settings", 14., fg))
                .tooltip(tooltip(format!(
                    "Settings{}",
                    super::shortcuts::hint("OpenSettings", cx)
                )))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_settings(super::settings_panel::Page::General, window, cx)
                })),
        )
        .child(
            item("sb-theme")
                .cursor_pointer()
                .child(icon_sized(theme_icon, 14., fg))
                .tooltip(tooltip("Toggle light/dark theme"))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
        )
    }
}

/// A status bar item: icon and text with a hover highlight.
fn item(id: &'static str) -> gpui::Stateful<gpui::Div> {
    let hover = gpui::white().opacity(0.12);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover))
}

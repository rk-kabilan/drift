//! The changes pane: commit box, Commit button with its menu, and the
//! Staged Changes / Changes groups.

use gpui::{
    anchored, deferred, div, point, prelude::*, px, Context, MouseButton, SharedString, Window,
};

use super::widgets::{icon, icon_button, tooltip};
use super::workspace::{CommitMode, Menu, RemoteOp, Workspace};
use crate::git::{ChangeKind, FileChange};
use crate::theme::{ActiveTheme as _, Theme};

impl Workspace {
    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = cx.theme().clone();
        // Keep room for the diff pane when the window is narrower than at drag time.
        let max = (f32::from(window.viewport_size().width) - 340.).max(200.);
        div()
            .w(px(self.sidebar_width.min(max)))
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
            .child(
                div()
                    .id("groups")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(8.))
                    .when(self.has_conflicts(), |d| {
                        d.child(self.render_group(Group::Merge, &t, cx))
                    })
                    .when(!self.staged.is_empty(), |d| {
                        d.child(self.render_group(Group::Staged, &t, cx))
                    })
                    .child(self.render_group(Group::Changes, &t, cx)),
            )
            .child(self.render_history(&t, cx))
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
            .children(self.remote_buttons("hdr", cx))
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
            .child(
                div()
                    .relative()
                    .child(
                        icon_button(
                            "hdr-more",
                            "more",
                            t.fg,
                            cx,
                            cx.listener(|this, _, _, cx| {
                                this.menu = if this.menu == Some(Menu::More) {
                                    None
                                } else {
                                    Some(Menu::More)
                                };
                                cx.notify();
                            }),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .tooltip(tooltip("More Actions")),
                    )
                    .when(self.menu == Some(Menu::More), |d| {
                        let busy = self.busy.is_some();
                        let remote = move |op: RemoteOp| {
                            move |this: &mut Workspace,
                                  _: &mut Window,
                                  cx: &mut Context<Workspace>| {
                                this.remote(op, cx)
                            }
                        };
                        d.child(render_menu(
                            t,
                            point(px(-180.), px(26.)),
                            vec![
                                MenuItem::new("mi-fetch", "Fetch", !busy, remote(RemoteOp::Fetch)),
                                MenuItem::new("mi-pull", "Pull", !busy, remote(RemoteOp::Pull)),
                                MenuItem::new("mi-push", "Push", !busy, remote(RemoteOp::Push)),
                                MenuItem::new(
                                    "mi-sync",
                                    "Sync (fetch, pull, push)",
                                    !busy,
                                    remote(RemoteOp::Sync),
                                ),
                                MenuItem::separator(),
                                MenuItem::new(
                                    "mi-branch",
                                    "Switch Branch…",
                                    true,
                                    |this, window, cx| this.open_branch_picker(window, cx),
                                ),
                                MenuItem::new(
                                    "mi-open",
                                    "Go to File…",
                                    true,
                                    |this, window, cx| this.open_file_picker(window, cx),
                                ),
                                MenuItem::new(
                                    "mi-commands",
                                    "All Git Commands…",
                                    true,
                                    |this, window, cx| this.open_command_palette(None, window, cx),
                                ),
                                MenuItem::separator(),
                                MenuItem::new(
                                    "mi-open-repo",
                                    "Open Repository…",
                                    !busy,
                                    |this, window, cx| this.open_repo_dialog(window, cx),
                                ),
                                MenuItem::new(
                                    "mi-recent-repo",
                                    "Open Recent Repository…",
                                    !busy,
                                    |this, window, cx| this.open_recent_picker(window, cx),
                                ),
                                MenuItem::new(
                                    "mi-close-repo",
                                    "Close Repository",
                                    !busy,
                                    |this, window, cx| this.close_repo(window, cx),
                                ),
                            ],
                            cx,
                        ))
                    }),
            )
    }

    fn render_commit_box(
        &mut self,
        t: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.busy.clone();
        let menu_open = self.menu == Some(Menu::Commit);
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
                                this.menu = if this.menu == Some(Menu::Commit) {
                                    None
                                } else {
                                    Some(Menu::Commit)
                                };
                                cx.notify();
                            })),
                    )
                    .when(menu_open, |d| d.child(self.render_commit_menu(t, cx))),
            )
    }

    fn render_commit_menu(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let commit = move |mode: CommitMode| {
            move |this: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
                this.commit(mode, window, cx)
            }
        };
        render_menu(
            t,
            point(px(0.), px(32.)),
            vec![
                MenuItem::new(
                    "m-commit",
                    "Commit Staged",
                    true,
                    commit(CommitMode::Staged),
                ),
                MenuItem::new("m-all", "Commit All", true, commit(CommitMode::All)),
                MenuItem::new(
                    "m-amend",
                    "Commit Staged (Amend)",
                    true,
                    commit(CommitMode::Amend),
                ),
                MenuItem::new("m-push", "Commit & Push", true, commit(CommitMode::AndPush)),
            ],
            cx,
        )
    }

    fn render_group(&self, kind: Group, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let group = kind;
        let staged = group == Group::Staged;
        let merge = group == Group::Merge;
        let list: Vec<FileChange> = self.group_files(group).cloned().collect();
        let list = &list;
        let open = match group {
            Group::Merge => self.merge_open,
            Group::Staged => self.staged_open,
            Group::Changes => self.changes_open,
        };
        let title = match group {
            Group::Merge => "Merge Changes",
            Group::Staged => "Staged Changes",
            Group::Changes => "Changes",
        };
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
        } else if merge {
            let p = paths.clone();
            actions = actions.child(
                icon_button(
                    "stage-all-merge",
                    "plus",
                    fg,
                    cx,
                    cx.listener(move |this, _, _, cx| this.stage(p.clone(), cx)),
                )
                .tooltip(tooltip("Stage All Merge Changes")),
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
            .id(match group {
                Group::Merge => "merge-h",
                Group::Staged => "staged-h",
                Group::Changes => "changes-h",
            })
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
                let open = match group {
                    Group::Merge => &mut this.merge_open,
                    Group::Staged => &mut this.staged_open,
                    Group::Changes => &mut this.changes_open,
                };
                *open = !*open;
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
            if list.is_empty() && kind == Group::Changes {
                group = group.child(
                    div()
                        .pl(px(24.))
                        .pr(px(10.))
                        .py(px(4.))
                        .text_size(px(12.))
                        .text_color(t.muted)
                        .child(if self.staged.is_empty() && !self.has_conflicts() {
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
        let selected = self.row_selected(change.staged, &change.path);
        let conflicted = change.kind == ChangeKind::Conflicted;
        let (bg, fg) = if selected {
            (Some(t.sel), t.sel_fg)
        } else if conflicted {
            (Some(t.conflict_row), t.fg)
        } else {
            (None, t.fg)
        };
        let hover = if conflicted {
            t.conflict_row_hover
        } else {
            t.hover
        };
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
        } else if change.kind == ChangeKind::Conflicted {
            // Discarding can't undo a merge; staging marks the conflicts resolved.
            let p = path.clone();
            actions = actions.child(
                icon_button(
                    SharedString::from(format!("stage-{row_group}")),
                    "plus",
                    fg,
                    cx,
                    cx.listener(move |this, _, window, cx| {
                        this.mark_resolved(p.clone(), window, cx)
                    }),
                )
                .tooltip(tooltip("Stage Changes (Mark as Resolved)")),
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
            // Single click previews the diff; double click keeps it open in its own tab.
            // Cmd/Ctrl-click and Shift-click pick several files.
            .on_click({
                let path = path.clone();
                cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                    let m = e.modifiers();
                    if m.shift {
                        this.pick_range(staged, &path, cx);
                    } else if m.secondary() {
                        this.pick_toggle(staged, &path, cx);
                    } else {
                        this.picked = vec![(staged, path.clone())];
                        this.pick_anchor = Some((staged, path.clone()));
                        this.open(staged, path.clone(), e.click_count() >= 2, cx)
                    }
                })
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let key = (staged, path.clone());
                    if !this.picked.contains(&key) {
                        this.picked = vec![key.clone()];
                        this.pick_anchor = Some(key);
                    }
                    this.menu = Some(Menu::Files { at: e.position });
                    cx.notify();
                }),
            )
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

/// A group of changed files in the changes pane, in the order drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    /// Files with merge conflicts.
    Merge,
    Staged,
    Changes,
}

impl Workspace {
    fn has_conflicts(&self) -> bool {
        self.unstaged
            .iter()
            .any(|c| c.kind == ChangeKind::Conflicted)
    }

    fn group_files(&self, group: Group) -> impl Iterator<Item = &FileChange> {
        let list = if group == Group::Staged {
            &self.staged
        } else {
            &self.unstaged
        };
        list.iter().filter(move |c| match group {
            Group::Merge => c.kind == ChangeKind::Conflicted,
            Group::Changes => c.kind != ChangeKind::Conflicted,
            Group::Staged => true,
        })
    }

    /// The active tab's changed file, as (staged, path).
    fn active_change(&self) -> Option<(bool, String)> {
        self.active_tab()
            .filter(|t| t.commit.is_none())
            .map(|t| (t.staged, t.path.clone()))
    }

    /// Whether a file row is drawn selected: the picked files when several are
    /// picked (or the menu is open on them), otherwise the active tab's file.
    fn row_selected(&self, staged: bool, path: &str) -> bool {
        let picked = self.picked.iter().any(|(s, p)| *s == staged && p == path);
        if self.picked.len() > 1 || matches!(self.menu, Some(Menu::Files { .. })) {
            return picked;
        }
        self.active_change()
            .is_some_and(|(s, p)| s == staged && p == path)
    }

    /// The file rows in the order they are drawn.
    fn visible_rows(&self) -> Vec<(bool, String)> {
        let mut rows = Vec::new();
        if self.merge_open {
            rows.extend(
                self.group_files(Group::Merge)
                    .map(|c| (false, c.path.clone())),
            );
        }
        if self.staged_open {
            rows.extend(self.staged.iter().map(|c| (true, c.path.clone())));
        }
        if self.changes_open {
            rows.extend(
                self.group_files(Group::Changes)
                    .map(|c| (false, c.path.clone())),
            );
        }
        rows
    }

    /// Starts multi-select from what is shown selected (the active tab's file).
    fn seed_pick(&mut self) {
        if self.picked.len() <= 1 {
            self.picked = self.active_change().into_iter().collect();
            if self.pick_anchor.is_none() {
                self.pick_anchor = self.picked.first().cloned();
            }
        }
    }

    fn pick_toggle(&mut self, staged: bool, path: &str, cx: &mut Context<Self>) {
        self.seed_pick();
        let key = (staged, path.to_string());
        if let Some(i) = self.picked.iter().position(|k| *k == key) {
            self.picked.remove(i);
        } else {
            self.picked.push(key.clone());
        }
        self.pick_anchor = Some(key);
        cx.notify();
    }

    fn pick_range(&mut self, staged: bool, path: &str, cx: &mut Context<Self>) {
        self.seed_pick();
        let rows = self.visible_rows();
        let to = rows.iter().position(|(s, p)| *s == staged && p == path);
        let from = self
            .pick_anchor
            .as_ref()
            .and_then(|a| rows.iter().position(|r| r == a))
            .or(to);
        if let (Some(a), Some(b)) = (from, to) {
            self.picked = rows[a.min(b)..=a.max(b)].to_vec();
            if self.pick_anchor.is_none() {
                self.pick_anchor = Some((staged, path.to_string()));
            }
        }
        cx.notify();
    }

    /// The right-click menu of the picked files, while it is open.
    pub(super) fn render_files_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let Some(Menu::Files { at }) = self.menu else {
            return None;
        };
        let files: Vec<FileChange> = self
            .picked
            .iter()
            .filter_map(|(s, p)| self.list(*s).iter().find(|c| &c.path == p).cloned())
            .collect();
        if files.is_empty() {
            return None;
        }
        let t = cx.theme().clone();
        let staged: Vec<String> = files
            .iter()
            .filter(|c| c.staged)
            .map(|c| c.path.clone())
            .collect();
        let unstaged: Vec<FileChange> = files.iter().filter(|c| !c.staged).cloned().collect();
        let paths: Vec<String> = files.iter().map(|c| c.path.clone()).collect();

        let mut items = Vec::new();
        {
            let files = files.clone();
            items.push(MenuItem::new(
                "files-open",
                "Open Changes",
                true,
                move |this, _, cx| {
                    for c in &files {
                        this.open(c.staged, c.path.clone(), true, cx);
                    }
                },
            ));
        }
        items.push(MenuItem::separator());
        if !unstaged.is_empty() {
            let p: Vec<String> = unstaged.iter().map(|c| c.path.clone()).collect();
            items.push(MenuItem::new(
                "files-stage",
                "Stage Changes",
                true,
                move |this, _, cx| {
                    this.picked.clear();
                    this.stage(p.clone(), cx)
                },
            ));
        }
        if !staged.is_empty() {
            items.push(MenuItem::new(
                "files-unstage",
                "Unstage Changes",
                true,
                move |this, _, cx| {
                    this.picked.clear();
                    this.unstage(staged.clone(), cx)
                },
            ));
        }
        if !unstaged.is_empty() {
            items.push(MenuItem::new(
                "files-discard",
                "Discard Changes",
                true,
                move |this, window, cx| this.discard(unstaged.clone(), window, cx),
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new(
            "files-reveal",
            REVEAL_LABEL,
            true,
            move |this, _, cx| this.reveal(&paths, cx),
        ));
        Some(render_context_menu(&t, at, items, cx))
    }

    /// Shows files in the system file manager, once per folder.
    fn reveal(&mut self, paths: &[String], cx: &mut Context<Self>) {
        let mut seen = std::collections::HashSet::new();
        for path in paths {
            let abs = self.root.join(path);
            let dir = abs.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            if !seen.insert(dir.clone()) {
                continue;
            }
            if abs.exists() {
                cx.reveal_path(&abs);
            } else if let Some(dir) = dir.ancestors().find(|d| d.exists()) {
                // A deleted file: open the closest folder that still exists.
                cx.open_with_system(dir);
            }
        }
    }

    /// Fetch, pull and push buttons, dimmed while a git operation runs.
    pub(super) fn remote_buttons(
        &self,
        prefix: &str,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let fg = cx.theme().fg;
        let busy = self.busy.is_some();
        [
            ("fetch", "Fetch", RemoteOp::Fetch),
            ("pull", "Pull", RemoteOp::Pull),
            ("push", "Push", RemoteOp::Push),
        ]
        .into_iter()
        .map(|(name, title, op)| {
            icon_button(
                SharedString::from(format!("{prefix}-{name}")),
                name,
                fg,
                cx,
                cx.listener(move |this, _, _, cx| this.remote(op, cx)),
            )
            .when(busy, |d| d.opacity(0.4))
            .tooltip(tooltip(format!(
                "{title}{}",
                super::shortcuts::hint(title, cx)
            )))
            .into_any_element()
        })
        .collect()
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

#[cfg(target_os = "macos")]
const REVEAL_LABEL: &str = "Reveal in Finder";
#[cfg(target_os = "windows")]
const REVEAL_LABEL: &str = "Reveal in File Explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const REVEAL_LABEL: &str = "Open Containing Folder";

type MenuAction = Box<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// One row of a dropdown menu.
pub(super) struct MenuItem {
    id: &'static str,
    label: &'static str,
    enabled: bool,
    action: Option<MenuAction>,
    /// Shortcut shown on the right.
    keys: Option<String>,
}

impl MenuItem {
    pub(super) fn new(
        id: &'static str,
        label: &'static str,
        enabled: bool,
        action: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Self {
        Self {
            id,
            label,
            enabled,
            action: Some(Box::new(action)),
            keys: None,
        }
    }

    pub(super) fn keys(mut self, keys: Option<String>) -> Self {
        self.keys = keys;
        self
    }

    pub(super) fn separator() -> Self {
        Self {
            id: "",
            label: "",
            enabled: false,
            action: None,
            keys: None,
        }
    }
}

/// A dropdown anchored at `offset` from its parent, drawn above everything else.
pub(super) fn render_menu(
    t: &Theme,
    offset: gpui::Point<gpui::Pixels>,
    items: Vec<MenuItem>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    deferred(
        anchored()
            .position_mode(gpui::AnchoredPositionMode::Local)
            .offset(offset)
            .child(menu_panel(t, items, cx)),
    )
    .with_priority(1)
}

/// A context menu at a window position (where the right click happened).
pub(super) fn render_context_menu(
    t: &Theme,
    at: gpui::Point<gpui::Pixels>,
    items: Vec<MenuItem>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    deferred(
        anchored()
            .position_mode(gpui::AnchoredPositionMode::Window)
            .position(at)
            .snap_to_window()
            .child(menu_panel(t, items, cx)),
    )
    .with_priority(1)
}

fn menu_panel(t: &Theme, items: Vec<MenuItem>, cx: &mut Context<Workspace>) -> gpui::Div {
    let rows = items.into_iter().enumerate().map(|(i, item)| {
        let Some(action) = item.action else {
            return div()
                .h(px(1.))
                .my(px(4.))
                .mx(px(6.))
                .bg(t.border)
                .into_any_element();
        };
        let (hover, hover_fg) = (t.sel, t.sel_fg);
        div()
            .id(if item.id.is_empty() {
                SharedString::from(format!("menu-{i}"))
            } else {
                item.id.into()
            })
            .px(px(10.))
            .h(px(26.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .whitespace_nowrap()
            .when(item.enabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(hover).text_color(hover_fg))
            })
            .when(!item.enabled, |d| d.text_color(t.faint))
            .child(div().flex_1().child(item.label))
            .children(item.keys.map(|k| {
                div()
                    .pl(px(24.))
                    .text_size(px(12.))
                    .text_color(t.muted)
                    .child(k)
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when(item.enabled, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = None;
                    action(this, window, cx);
                    cx.notify();
                }))
            })
            .into_any_element()
    });
    div()
        .min_w(px(200.))
        .p(px(4.))
        .flex()
        .flex_col()
        .bg(t.panel)
        .text_color(t.fg)
        .border_1()
        .border_color(t.border)
        .rounded(px(6.))
        .shadow_lg()
        .children(rows)
}

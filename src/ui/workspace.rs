//! The main window: state, git operations and the top-level layout
//! (title bar, changes pane, diff pane, status bar).

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, Context, Entity, FocusHandle, Focusable, KeyBinding, PromptLevel,
    UniformListScrollHandle, Window,
};

use crate::diff::{FileDiff, Layout, Row};
use crate::git::{Blame, FileChange, HeadInfo, Repo};
use crate::syntax::Lang;
use crate::theme::{ActiveTheme as _, Appearance, Theme, ThemeState};
use crate::ui::text_input::{TextInput, TextInputEvent};

actions!(
    drift,
    [
        Commit,
        CommitAll,
        Refresh,
        NextChange,
        PreviousChange,
        ToggleLayout,
        ToggleTheme,
        Quit
    ]
);

pub fn actions_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-enter", Commit, None),
        KeyBinding::new("secondary-r", Refresh, None),
        KeyBinding::new("f5", Refresh, None),
        KeyBinding::new("alt-f5", NextChange, None),
        KeyBinding::new("shift-alt-f5", PreviousChange, None),
        KeyBinding::new("f7", NextChange, None),
        KeyBinding::new("shift-f7", PreviousChange, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitMode {
    Staged,
    All,
    Amend,
    AndPush,
}

/// Which line the diff cursor is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorLine {
    /// A line of the old (left) text.
    Old(usize),
    /// A line of the new (right) text.
    New(usize),
}

pub struct DiffState {
    pub change: FileChange,
    pub old_text: String,
    pub new_text: String,
    pub file: FileDiff,
    pub rows: Vec<Row>,
    pub expanded: HashSet<usize>,
    pub left_label: String,
    pub right_label: String,
    pub lang: Lang,
    pub blame: Option<Blame>,
    pub cursor: Option<CursorLine>,
}

pub struct Workspace {
    pub(super) repo: Option<Repo>,
    pub(super) root: PathBuf,
    pub(super) open_error: Option<String>,
    pub(super) head: HeadInfo,
    pub(super) staged: Vec<FileChange>,
    pub(super) unstaged: Vec<FileChange>,
    pub(super) staged_open: bool,
    pub(super) changes_open: bool,
    pub(super) selected: Option<(bool, String)>,
    pub(super) diff: Option<DiffState>,
    pub(super) layout: Layout,
    pub(super) message: Entity<TextInput>,
    pub(super) commit_menu_open: bool,
    pub(super) error: Option<String>,
    pub(super) busy: Option<String>,
    pub(super) diff_scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _tasks: Vec<gpui::Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl Workspace {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (repo, open_error) = match Repo::discover(&root) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let root = repo
            .as_ref()
            .map(|r| r.root().to_path_buf())
            .unwrap_or(root);
        let message = cx.new(|cx| TextInput::new("Message", cx));
        let mut subscriptions =
            vec![
                cx.subscribe(&message, |this: &mut Self, _, _: &TextInputEvent, cx| {
                    this.error = None;
                    cx.notify();
                }),
            ];
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh(cx);
            }
        }));
        subscriptions.push(cx.observe_window_appearance(window, |_, window, cx| {
            let state = cx.global::<ThemeState>();
            if state.forced.is_none() {
                let appearance = Appearance::from(window.appearance());
                cx.global_mut::<ThemeState>().theme = Theme::for_appearance(appearance);
                cx.notify();
            }
        }));

        // Poll for outside changes (editors, terminals) while the window is in front.
        let poll = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let alive = this
                .update_in(cx, |this, window, cx| {
                    if window.is_window_active() && this.busy.is_none() {
                        this.refresh(cx);
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        });

        let mut this = Self {
            repo,
            root,
            open_error,
            head: HeadInfo::default(),
            staged: vec![],
            unstaged: vec![],
            staged_open: true,
            changes_open: true,
            selected: None,
            diff: None,
            layout: Layout::SideBySide,
            message,
            commit_menu_open: false,
            error: None,
            busy: None,
            diff_scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _tasks: vec![poll],
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        // Show the first changed file straight away.
        if let Some(first) = this.unstaged.first().or(this.staged.first()).cloned() {
            this.select(first.staged, first.path, cx);
        }
        this
    }

    pub fn title(&self) -> String {
        if self.repo.is_none() {
            return "drift".into();
        }
        format!("{} · {}", self.head.repo_name, self.head.branch)
    }

    /// Re-reads status and the selected file's diff.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else { return };
        match repo.status() {
            Ok((staged, unstaged)) => {
                let dirty = !staged.is_empty()
                    || unstaged
                        .iter()
                        .any(|c| c.kind != crate::git::ChangeKind::Untracked);
                self.head = repo.head_info(dirty);
                self.staged = staged;
                self.unstaged = unstaged;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        // Keep the selection if the file is still listed; otherwise fall back to its
        // twin in the other group (e.g. a file that was just staged).
        if let Some((staged, path)) = self.selected.clone() {
            let exists = |list: &[FileChange]| list.iter().any(|c| c.path == path);
            let target = if exists(self.list(staged)) {
                Some(staged)
            } else if exists(self.list(!staged)) {
                Some(!staged)
            } else {
                None
            };
            match target {
                Some(s) => self.load_diff(s, path, false, cx),
                None => {
                    self.selected = None;
                    self.diff = None;
                }
            }
        }
        let message_placeholder = format!(
            "Message ({}+Enter to commit on '{}')",
            secondary_key(),
            self.head.branch
        );
        self.message
            .update(cx, |m, cx| m.set_placeholder(message_placeholder, cx));
        cx.notify();
    }

    fn list(&self, staged: bool) -> &[FileChange] {
        if staged {
            &self.staged
        } else {
            &self.unstaged
        }
    }

    pub fn select(&mut self, staged: bool, path: String, cx: &mut Context<Self>) {
        self.load_diff(staged, path, true, cx);
        cx.notify();
    }

    fn load_diff(&mut self, staged: bool, path: String, reset_view: bool, _cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else { return };
        let Some(change) = self.list(staged).iter().find(|c| c.path == path).cloned() else {
            return;
        };
        let (old, new, left_label, right_label) = repo.change_texts(&change);

        let same_file = self.selected.as_ref() == Some(&(staged, path.clone()));
        if !reset_view && same_file {
            if let Some(d) = &mut self.diff {
                if d.old_text == old && d.new_text == new && d.change == change {
                    d.left_label = left_label;
                    d.right_label = right_label;
                    return;
                }
            }
        }

        let file = FileDiff::new(&old, &new);
        let keep = !reset_view && same_file;
        let (expanded, cursor) = match (&self.diff, keep) {
            (Some(d), true) => (d.expanded.clone(), d.cursor),
            _ => (HashSet::new(), None),
        };
        let rows = file.rows(self.layout, &expanded);
        let blame = if change.kind == crate::git::ChangeKind::Untracked || new.is_empty() {
            None
        } else {
            repo.blame(&change.path, &new).ok()
        };
        let lang = Lang::for_path(&change.path);
        self.selected = Some((staged, path));
        self.diff = Some(DiffState {
            change,
            old_text: old,
            new_text: new,
            file,
            rows,
            expanded,
            left_label,
            right_label,
            lang,
            blame,
            cursor,
        });
        if reset_view {
            self.diff_scroll
                .scroll_to_item(0, gpui::ScrollStrategy::Top);
            // Put the cursor on the first change so the blame shows something useful.
            if let Some(d) = &mut self.diff {
                d.cursor = first_change_cursor(&d.rows);
            }
        }
    }

    pub fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.diff {
            d.rows = d.file.rows(self.layout, &d.expanded);
        }
        cx.notify();
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.rebuild_rows(cx);
    }

    pub fn expand_fold(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.diff {
            d.expanded.insert(id);
        }
        self.rebuild_rows(cx);
    }

    pub fn set_cursor(&mut self, cursor: CursorLine, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.diff {
            d.cursor = Some(cursor);
        }
        cx.notify();
    }

    /// Scrolls to the next (or previous) block of changed lines and moves the cursor there.
    pub fn jump_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(d) = &mut self.diff else { return };
        let starts: Vec<usize> = change_starts(&d.rows);
        if starts.is_empty() {
            return;
        }
        let current = d
            .cursor
            .and_then(|c| row_for_cursor(&d.rows, c))
            .unwrap_or(0);
        let target = if forward {
            starts
                .iter()
                .copied()
                .find(|&s| s > current)
                .unwrap_or(starts[0])
        } else {
            starts
                .iter()
                .rev()
                .copied()
                .find(|&s| s < current)
                .unwrap_or(*starts.last().unwrap())
        };
        d.cursor = cursor_for_row(&d.rows[target]);
        self.diff_scroll
            .scroll_to_item(target.saturating_sub(3), gpui::ScrollStrategy::Top);
        cx.notify();
    }

    // ---- git operations ----

    fn run(
        &mut self,
        cx: &mut Context<Self>,
        op: impl FnOnce(&Repo) -> anyhow::Result<()>,
    ) -> bool {
        let Some(repo) = &self.repo else { return false };
        let result = op(repo);
        let ok = result.is_ok();
        self.error = result.err().map(|e| format!("{e:#}"));
        self.refresh(cx);
        ok
    }

    pub fn stage(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.run(cx, |r| r.stage(&paths));
    }

    pub fn unstage(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.run(cx, |r| r.unstage(&paths));
    }

    pub fn open_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let abs = self.root.join(path);
        if abs.exists() {
            cx.open_with_system(&abs);
        } else {
            self.error = Some(format!("{path} no longer exists in the working tree."));
            cx.notify();
        }
    }

    /// Asks first, since discarding cannot be undone.
    pub fn discard(
        &mut self,
        changes: Vec<FileChange>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if changes.is_empty() {
            return;
        }
        let (message, detail) = if changes.len() == 1 {
            let c = &changes[0];
            if c.kind == crate::git::ChangeKind::Untracked {
                (
                    format!("Delete the untracked file {}?", c.file_name()),
                    "The file will be removed from disk.",
                )
            } else {
                (
                    format!("Discard changes in {}?", c.file_name()),
                    "This can't be undone.",
                )
            }
        } else {
            (
                format!("Discard changes in {} files?", changes.len()),
                "Untracked files will be deleted. This can't be undone.",
            )
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(detail),
            &["Discard", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| {
                    this.run(cx, |r| r.discard(&changes));
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn commit(&mut self, mode: CommitMode, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_menu_open = false;
        if self.busy.is_some() {
            return;
        }
        let message = self.message.read(cx).text().to_string();
        if mode == CommitMode::Staged && self.staged.is_empty() {
            self.error = Some(if self.unstaged.is_empty() {
                "There are no changes to commit.".into()
            } else {
                "No staged changes. Stage files with +, or choose Commit All from the menu.".into()
            });
            cx.notify();
            return;
        }
        let all_paths: Vec<String> = self.unstaged.iter().map(|c| c.path.clone()).collect();
        let ok = self.run(cx, |r| {
            if mode == CommitMode::All {
                r.stage(&all_paths)?;
            }
            r.commit(&message, mode == CommitMode::Amend)
        });
        if ok {
            self.message.update(cx, |m, cx| m.set_text("", cx));
            window.focus(&self.message.focus_handle(cx));
            if mode == CommitMode::AndPush {
                self.push(cx);
            }
        }
    }

    pub fn push(&mut self, cx: &mut Context<Self>) {
        self.background("Pushing…", cx, |root| Repo::push(&root));
    }

    /// Pulls (fast-forward only) when behind, then pushes when ahead.
    pub fn sync(&mut self, cx: &mut Context<Self>) {
        let (behind, ahead) = (self.head.behind, self.head.ahead);
        self.background("Syncing…", cx, move |root| {
            if behind > 0 {
                Repo::pull(&root)?;
            }
            if ahead > 0 || behind == 0 {
                Repo::push(&root)?;
            }
            Ok(())
        });
    }

    fn background(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
        job: impl FnOnce(PathBuf) -> anyhow::Result<()> + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label.to_string());
        let root = self.root.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { job(root) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                this.error = result.err().map(|e| format!("{e:#}"));
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let state = cx.global_mut::<ThemeState>();
        let next = match state.theme.appearance {
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::Light,
        };
        state.forced = Some(next);
        state.theme = Theme::for_appearance(next);
        cx.refresh_windows();
    }

    /// The title bar shows only the repository and branch. It is drawn by the app
    /// on macOS (transparent title bar) and on Linux desktops without server-side
    /// decorations, where it also carries the window controls.
    fn render_title_bar(&self, client_decorated: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let fg = t.fg;
        let control =
            |id: &'static str, name: &'static str, cx: &mut Context<Self>, f: fn(&mut Window)| {
                super::widgets::icon_button(id, name, fg, cx, move |_, window, _| f(window))
            };
        div()
            .id("title-bar")
            .h(px(34.))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(t.muted)
            .when(client_decorated, |d| {
                d.on_mouse_down(gpui::MouseButton::Left, |e, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                })
            })
            .child(super::widgets::icon("repo", t.muted))
            .child(self.title())
            .when(client_decorated, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(8.))
                        .top(px(6.))
                        .flex()
                        .gap(px(4.))
                        .child(control("win-min", "minus", cx, |w| w.minimize_window()))
                        .child(control("win-max", "maximize", cx, |w| w.zoom_window()))
                        .child(control("win-close", "close", cx, |w| w.remove_window())),
                )
            })
    }
}

pub fn secondary_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    }
}

fn row_cursor_candidates(row: &Row) -> (Option<usize>, Option<usize>, bool) {
    match row {
        Row::Pair { left, right } => {
            let changed = left
                .as_ref()
                .is_some_and(|s| s.kind != crate::diff::LineKind::Context)
                || right
                    .as_ref()
                    .is_some_and(|s| s.kind != crate::diff::LineKind::Context);
            (
                left.as_ref().map(|s| s.line),
                right.as_ref().map(|s| s.line),
                changed,
            )
        }
        Row::Line { old, new, side } => (*old, *new, side.kind != crate::diff::LineKind::Context),
        _ => (None, None, false),
    }
}

fn cursor_for_row(row: &Row) -> Option<CursorLine> {
    let (old, new, _) = row_cursor_candidates(row);
    new.map(CursorLine::New).or(old.map(CursorLine::Old))
}

fn row_for_cursor(rows: &[Row], cursor: CursorLine) -> Option<usize> {
    rows.iter().position(|r| {
        let (old, new, _) = row_cursor_candidates(r);
        match cursor {
            CursorLine::New(n) => new == Some(n),
            CursorLine::Old(o) => old == Some(o) && new.is_none(),
        }
    })
}

/// Indices of the first row of each run of changed rows.
fn change_starts(rows: &[Row]) -> Vec<usize> {
    let mut starts = vec![];
    let mut prev = false;
    for (i, r) in rows.iter().enumerate() {
        let (_, _, changed) = row_cursor_candidates(r);
        if changed && !prev {
            starts.push(i);
        }
        prev = changed;
    }
    starts
}

/// The first changed line of the new text, or failing that the first change.
fn first_change_cursor(rows: &[Row]) -> Option<CursorLine> {
    rows.iter()
        .find_map(|r| match row_cursor_candidates(r) {
            (_, Some(new), true) => Some(CursorLine::New(new)),
            _ => None,
        })
        .or_else(|| {
            change_starts(rows)
                .first()
                .and_then(|&i| cursor_for_row(&rows[i]))
        })
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(&self.title());
        let client_decorated = matches!(
            window.window_decorations(),
            gpui::Decorations::Client { .. }
        );
        let show_title_bar = cfg!(target_os = "macos") || client_decorated;
        let state = cx.global::<ThemeState>();
        let (ui_font, t) = (state.ui_font.clone(), state.theme.clone());

        let body: gpui::AnyElement = if let Some(err) = &self.open_error {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(t.muted)
                .child(
                    div()
                        .text_size(px(15.))
                        .text_color(t.fg)
                        .child("No git repository here"),
                )
                .child(err.clone())
                .child("Start drift from inside a repository, or pass its path: drift <path>")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .gap(px(6.))
                .px(px(6.))
                .pb(px(6.))
                .child(self.render_sidebar(window, cx))
                .child(self.render_diff_pane(window, cx))
                .into_any_element()
        };

        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Commit, window, cx| {
                this.commit(CommitMode::Staged, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CommitAll, window, cx| {
                this.commit(CommitMode::All, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &NextChange, _, cx| this.jump_change(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousChange, _, cx| this.jump_change(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleLayout, _, cx| {
                let next = if this.layout == Layout::SideBySide {
                    Layout::Inline
                } else {
                    Layout::SideBySide
                };
                this.set_layout(next, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.commit_menu_open {
                        this.commit_menu_open = false;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(t.chrome)
            .font_family(ui_font)
            .text_size(px(13.))
            .text_color(t.fg)
            .when(show_title_bar, |d| {
                d.child(self.render_title_bar(client_decorated, cx))
            })
            .when(!show_title_bar, |d| d.pt(px(6.)))
            .child(body)
            .child(self.render_status_bar(window, cx))
    }
}

impl Workspace {
    pub fn message_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.message.focus_handle(cx)
    }
}

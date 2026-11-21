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
use crate::git::{Blame, BranchInfo, ChangeKind, FileChange, HeadInfo, Repo, TagInfo};
use crate::syntax::Lang;
use crate::theme::{ActiveTheme as _, Appearance, Theme, ThemeState};
use crate::ui::picker::{Picker, PickerEvent, PickerItem};
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
        QuickOpen,
        CommandPalette,
        SwitchBranch,
        CloseTab,
        NextTab,
        PreviousTab,
        Fetch,
        Pull,
        Push,
        Sync,
        Quit
    ]
);

pub fn actions_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-enter", Commit, None),
        KeyBinding::new("secondary-r", Refresh, None),
        KeyBinding::new("f5", Refresh, None),
        KeyBinding::new("f7", NextChange, None),
        KeyBinding::new("shift-f7", PreviousChange, None),
        KeyBinding::new("secondary-p", QuickOpen, None),
        KeyBinding::new("secondary-shift-p", CommandPalette, None),
        KeyBinding::new("f1", CommandPalette, None),
        KeyBinding::new("secondary-shift-b", SwitchBranch, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
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

/// A dropdown menu that is currently open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Commit,
    More,
}

/// A remote operation running in the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteOp {
    Fetch,
    Pull,
    Push,
    /// Fetch, then pull if behind, then push if ahead.
    Sync,
}

/// Which way a merge arrow copies a change block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeDirection {
    /// Copy the left (older) side over the right.
    ToRight,
    /// Copy the right (newer) side over the left.
    ToLeft,
}

/// What the user chose in a picker.
pub enum Pick {
    Item(usize),
    Text(String),
}

/// Runs when a picker is confirmed. The picker is already closed, so the
/// callback may open the next one.
pub type OnPick = Box<dyn FnOnce(&mut Workspace, Pick, &mut Window, &mut Context<Workspace>)>;

/// A ref chosen in [`Workspace::pick_ref`].
pub enum RefChoice {
    Branch(BranchInfo),
    Tag(TagInfo),
    /// A name that matches no branch, typed into the box.
    New(String),
}

/// Which refs [`Workspace::pick_ref`] lists.
#[derive(Clone, Copy, Debug, Default)]
pub struct RefFilter {
    pub local: bool,
    pub remote: bool,
    pub tags: bool,
    /// Lists the checked-out branch too.
    pub head: bool,
    /// Offers to create a branch with the typed name.
    pub create: bool,
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

/// An open diff. A single click opens a file in the preview tab (shown in
/// italics, replaced by the next single click); a double click pins it.
pub struct Tab {
    pub staged: bool,
    pub path: String,
    pub pinned: bool,
    pub diff: Option<DiffState>,
    pub scroll: UniformListScrollHandle,
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
    pub(super) tabs: Vec<Tab>,
    pub(super) active: Option<usize>,
    pub(super) layout: Layout,
    pub(super) message: Entity<TextInput>,
    pub(super) menu: Option<Menu>,
    pub(super) error: Option<String>,
    /// Status text of the running remote operation, e.g. "Pushing…".
    pub(super) busy: Option<String>,
    /// A short note shown in the status bar after an operation finishes.
    pub(super) status_note: Option<&'static str>,
    pub(super) picker: Option<Entity<Picker>>,
    on_pick: Option<OnPick>,
    /// The open picker is Cmd+P, which switches to commands on ">".
    picker_is_files: bool,
    pub(super) recent_commands: Vec<super::commands::Cmd>,
    pub(super) show_blame: bool,
    focus_handle: FocusHandle,
    _picker_subscription: Option<gpui::Subscription>,
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
                cx.subscribe(&message, |this: &mut Self, _, e: &TextInputEvent, cx| {
                    if matches!(e, TextInputEvent::Changed) {
                        this.error = None;
                        cx.notify();
                    }
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
            tabs: vec![],
            active: None,
            layout: Layout::SideBySide,
            message,
            menu: None,
            error: None,
            busy: None,
            status_note: None,
            picker: None,
            on_pick: None,
            picker_is_files: false,
            recent_commands: Vec::new(),
            show_blame: true,
            focus_handle: cx.focus_handle(),
            _picker_subscription: None,
            _tasks: vec![poll],
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        // Show the first changed file straight away.
        if let Some(first) = this.unstaged.first().or(this.staged.first()).cloned() {
            this.open(first.staged, first.path, false, cx);
        }
        this
    }

    pub fn title(&self) -> String {
        if self.repo.is_none() {
            return "drift".into();
        }
        format!("{} · {}", self.head.repo_name, self.head.branch)
    }

    pub fn diff(&self) -> Option<&DiffState> {
        self.active
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.diff.as_ref())
    }

    fn diff_mut(&mut self) -> Option<&mut DiffState> {
        self.active
            .and_then(|i| self.tabs.get_mut(i))
            .and_then(|t| t.diff.as_mut())
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    /// Re-reads status and every open tab's diff.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else { return };
        match repo.status() {
            Ok((staged, unstaged)) => {
                let dirty =
                    !staged.is_empty() || unstaged.iter().any(|c| c.kind != ChangeKind::Untracked);
                self.head = repo.head_info(dirty);
                self.staged = staged;
                self.unstaged = unstaged;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }

        // A tab follows its file between the two groups (e.g. after staging) and
        // closes when the file has no changes left.
        let active_key = self.active_tab().map(|t| (t.staged, t.path.clone()));
        let mut kept: Vec<Tab> = Vec::with_capacity(self.tabs.len());
        for mut tab in std::mem::take(&mut self.tabs) {
            let in_list = |staged: bool| self.list(staged).iter().any(|c| c.path == tab.path);
            if !in_list(tab.staged) {
                if in_list(!tab.staged) {
                    tab.staged = !tab.staged;
                    tab.diff = None;
                } else {
                    continue;
                }
            }
            if kept
                .iter()
                .any(|k| k.staged == tab.staged && k.path == tab.path)
            {
                continue;
            }
            kept.push(tab);
        }
        self.tabs = kept;
        self.active = if self.tabs.is_empty() {
            None
        } else {
            active_key
                .and_then(|(s, p)| {
                    self.tabs
                        .iter()
                        .position(|t| t.path == p && t.staged == s)
                        .or_else(|| self.tabs.iter().position(|t| t.path == p))
                })
                .or(Some(self.active.unwrap_or(0).min(self.tabs.len() - 1)))
        };
        for i in 0..self.tabs.len() {
            self.load_tab(i, false);
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

    pub(super) fn list(&self, staged: bool) -> &[FileChange] {
        if staged {
            &self.staged
        } else {
            &self.unstaged
        }
    }

    /// Shows a file's diff. Without `pin` it goes into the preview tab.
    pub fn open(&mut self, staged: bool, path: String, pin: bool, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.staged == staged && t.path == path)
        {
            if pin {
                self.tabs[ix].pinned = true;
            }
            self.active = Some(ix);
        } else {
            let tab = Tab {
                staged,
                path,
                pinned: pin,
                diff: None,
                scroll: UniformListScrollHandle::new(),
            };
            match self.tabs.iter().position(|t| !t.pinned) {
                Some(preview) if !pin => {
                    self.tabs[preview] = tab;
                    self.active = Some(preview);
                }
                _ => {
                    let at = self.active.map_or(self.tabs.len(), |a| a + 1);
                    self.tabs.insert(at, tab);
                    self.active = Some(at);
                }
            }
            self.load_tab(self.active.unwrap(), true);
        }
        cx.notify();
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.active = Some(ix);
            cx.notify();
        }
    }

    pub fn pin_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get_mut(ix) {
            t.pinned = true;
            cx.notify();
        }
    }

    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > ix => Some(a - 1),
            Some(a) => Some(a.min(self.tabs.len() - 1)),
            None => None,
        };
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(a) = self.active {
            let n = self.tabs.len() as isize;
            self.active = Some(((a as isize + delta).rem_euclid(n)) as usize);
            cx.notify();
        }
    }

    /// (Re)computes a tab's diff. Keeps fold and cursor state unless `reset_view`.
    fn load_tab(&mut self, ix: usize, reset_view: bool) {
        let Some(repo) = &self.repo else { return };
        let (staged, path) = (self.tabs[ix].staged, self.tabs[ix].path.clone());
        let Some(change) = self.list(staged).iter().find(|c| c.path == path).cloned() else {
            return;
        };
        let (old, new, left_label, right_label) = repo.change_texts(&change);
        let tab = &mut self.tabs[ix];

        if !reset_view {
            if let Some(d) = &mut tab.diff {
                if d.old_text == old && d.new_text == new && d.change == change {
                    d.left_label = left_label;
                    d.right_label = right_label;
                    return;
                }
            }
        }
        let file = FileDiff::new(&old, &new);
        let (expanded, cursor) = match (&tab.diff, reset_view) {
            (Some(d), false) => (d.expanded.clone(), d.cursor),
            _ => (HashSet::new(), None),
        };
        let rows = file.rows(self.layout, &expanded);
        let blame = if change.kind == ChangeKind::Untracked || new.is_empty() {
            None
        } else {
            repo.blame(&change.path, &new).ok()
        };
        let lang = Lang::for_path(&change.path);
        let cursor = if reset_view {
            first_change_cursor(&rows)
        } else {
            cursor
        };
        if reset_view {
            tab.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        }
        tab.diff = Some(DiffState {
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
    }

    pub fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let layout = self.layout;
        for tab in &mut self.tabs {
            if let Some(d) = &mut tab.diff {
                d.rows = d.file.rows(layout, &d.expanded);
            }
        }
        cx.notify();
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.rebuild_rows(cx);
    }

    pub fn expand_fold(&mut self, id: usize, cx: &mut Context<Self>) {
        let layout = self.layout;
        if let Some(d) = self.diff_mut() {
            d.expanded.insert(id);
            d.rows = d.file.rows(layout, &d.expanded);
        }
        cx.notify();
    }

    pub fn set_cursor(&mut self, cursor: CursorLine, cx: &mut Context<Self>) {
        if let Some(d) = self.diff_mut() {
            d.cursor = Some(cursor);
        }
        cx.notify();
    }

    /// Scrolls to the next (or previous) block of changed lines and moves the cursor there.
    pub fn jump_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return;
        };
        let Some(d) = &mut tab.diff else { return };
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
        tab.scroll
            .scroll_to_item(target.saturating_sub(3), gpui::ScrollStrategy::Top);
        cx.notify();
    }

    /// Which merge arrows make sense for the active diff. Left is the index (or
    /// HEAD) and right the working tree for unstaged files; for staged files left
    /// is HEAD and right the index, and HEAD cannot be written.
    pub fn merge_directions(&self) -> (bool, bool) {
        match self.diff() {
            Some(d) if d.old_text != "(binary file)" && d.new_text != "(binary file)" => {
                let to_right = true;
                let to_left = !d.change.staged;
                (to_right, to_left)
            }
            _ => (false, false),
        }
    }

    /// Copies one change block across the diff: towards the working tree
    /// (revert), into the index (stage just this block), or from HEAD into the
    /// index (unstage just this block).
    pub fn merge_block(&mut self, block: usize, direction: MergeDirection, cx: &mut Context<Self>) {
        let Some(d) = self.diff() else { return };
        let Some(range) = d.file.blocks().get(block).cloned() else {
            return;
        };
        let (staged, path) = (d.change.staged, d.change.path.clone());
        let (old, new) = (d.old_text.clone(), d.new_text.clone());
        let Some(repo) = &self.repo else { return };

        // Refuse if the file changed since the diff was drawn.
        let (cur_old, cur_new, _, _) = repo.change_texts(&d.change);
        if cur_old != old || cur_new != new {
            self.error = Some(format!(
                "{} changed on disk. The diff was refreshed; try again.",
                d.change.file_name()
            ));
            self.refresh(cx);
            return;
        }
        let result = match (direction, staged) {
            (MergeDirection::ToRight, false) => {
                let text = crate::merge::splice_lines(&new, range.new, &old, range.old);
                repo.write_worktree(&path, &text)
            }
            (MergeDirection::ToLeft, false) => {
                let text = crate::merge::splice_lines(&old, range.old, &new, range.new);
                repo.write_index(&path, &text)
            }
            (MergeDirection::ToRight, true) => {
                let text = crate::merge::splice_lines(&new, range.new, &old, range.old);
                repo.write_index(&path, &text)
            }
            (MergeDirection::ToLeft, true) => {
                Err(anyhow::anyhow!("HEAD can't be changed from the diff."))
            }
        };
        self.error = result.err().map(|e| format!("{e:#}"));
        self.refresh(cx);
    }

    // ---- git operations ----

    pub(super) fn run(
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
            if c.kind == ChangeKind::Untracked {
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
        self.menu = None;
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
                self.remote(RemoteOp::Push, cx);
            }
        }
    }

    /// Runs a fetch, pull, push or sync in the background. The status bar shows
    /// a spinning sync icon and what is happening at each step.
    pub fn remote(&mut self, op: RemoteOp, cx: &mut Context<Self>) {
        self.menu = None;
        if self.busy.is_some() || self.repo.is_none() {
            return;
        }
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let step = async |label: &'static str,
                              job: fn(&std::path::Path) -> anyhow::Result<()>,
                              cx: &mut gpui::AsyncApp|
                   -> anyhow::Result<()> {
                this.update(cx, |this, cx| {
                    this.busy = Some(label.to_string());
                    cx.notify();
                })?;
                let root = root.clone();
                let result = cx.background_spawn(async move { job(&root) }).await;
                this.update(cx, |this, cx| this.refresh(cx))?;
                result
            };
            let result: anyhow::Result<&'static str> = async {
                match op {
                    RemoteOp::Fetch => step("Fetching…", Repo::fetch, cx).await.map(|_| "Fetched"),
                    RemoteOp::Pull => step("Pulling…", Repo::pull, cx).await.map(|_| "Pulled"),
                    RemoteOp::Push => step("Pushing…", Repo::push, cx).await.map(|_| "Pushed"),
                    RemoteOp::Sync => {
                        step("Fetching…", Repo::fetch, cx).await?;
                        let (behind, upstream) =
                            this.read_with(cx, |t, _| (t.head.behind, t.head.has_upstream))?;
                        if behind > 0 {
                            step("Pulling…", Repo::pull, cx).await?;
                        }
                        let ahead = this.read_with(cx, |t, _| t.head.ahead)?;
                        if ahead > 0 || !upstream {
                            step("Pushing…", Repo::push, cx).await?;
                        }
                        Ok("Up to date")
                    }
                }
            }
            .await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(done) => this.flash(done, cx),
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Shows a short status message ("Pushed") for a couple of seconds.
    pub(super) fn flash(&mut self, text: &'static str, cx: &mut Context<Self>) {
        self.status_note = Some(text);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                if this.status_note == Some(text) {
                    this.status_note = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- pickers ----

    pub fn open_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        let keys: Vec<(bool, String)> = self
            .unstaged
            .iter()
            .chain(&self.staged)
            .map(|c| (c.staged, c.path.clone()))
            .collect();
        let items = self.change_items(true, true, cx);
        let picker = cx.new(|cx| {
            Picker::new(
                "Search changed files by name (type > for commands)",
                "No changed files match.",
                items,
                cx,
            )
        });
        self.show_picker(
            picker,
            Box::new(move |this, pick, _, cx| {
                if let Pick::Item(ix) = pick {
                    if let Some((staged, path)) = keys.get(ix).cloned() {
                        this.open(staged, path, true, cx);
                    }
                }
            }),
            window,
            cx,
        );
        self.picker_is_files = true;
    }

    /// Picker rows for changed files, under "Changes" and "Staged Changes".
    pub(super) fn change_items(
        &self,
        unstaged: bool,
        staged: bool,
        cx: &mut Context<Self>,
    ) -> Vec<PickerItem> {
        let t = cx.theme().clone();
        let mut items = Vec::new();
        for (section, list, on) in [
            ("Changes", &self.unstaged, unstaged),
            ("Staged Changes", &self.staged, staged),
        ] {
            if !on {
                continue;
            }
            for c in list.iter() {
                items.push(PickerItem {
                    haystack: c.path.clone(),
                    label_start: c.path.len() - c.file_name().len(),
                    note: String::new(),
                    icon: "file",
                    icon_color: Some(super::sidebar::file_badge(c.file_name()).1),
                    badge: Some((
                        c.kind.letter().to_string(),
                        super::sidebar::status_color(c.kind, &t),
                    )),
                    section: Some(section),
                });
            }
        }
        items
    }

    /// The branch list from the status bar and Cmd+Shift+B: check out a branch
    /// or tag, or type a new name to create a branch from HEAD.
    pub fn open_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.checkout_picker(window, cx);
    }

    pub(super) fn checkout_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_ref(
            "Select a branch or tag to check out, or type a new branch name",
            RefFilter {
                local: true,
                remote: true,
                tags: true,
                head: true,
                create: true,
            },
            |this, choice, _, cx| match choice {
                RefChoice::Branch(b) if b.is_head => {}
                RefChoice::Branch(b) => {
                    this.run(cx, |r| r.switch_branch(&b));
                }
                RefChoice::Tag(t) => {
                    this.git(&["switch", "--detach", &t.name], "Checked out tag", cx);
                }
                RefChoice::New(name) => {
                    if this.run(cx, |r| r.checkout_or_create(&name, None)) {
                        this.flash("Switched branch", cx);
                    }
                }
            },
            window,
            cx,
        );
    }

    /// Lists branches and tags, and calls `then` with the one chosen.
    pub(super) fn pick_ref(
        &mut self,
        placeholder: &'static str,
        filter: RefFilter,
        then: impl FnOnce(&mut Self, RefChoice, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = &self.repo else { return };
        let loaded = repo.branches().and_then(|b| {
            let tags = if filter.tags { repo.tags()? } else { vec![] };
            Ok((b, tags))
        });
        let (branches, tags) = match loaded {
            Ok(v) => v,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
                return;
            }
        };
        let branches: Vec<BranchInfo> = branches
            .into_iter()
            .filter(|b| {
                if b.remote {
                    filter.remote
                } else {
                    filter.local
                }
            })
            .filter(|b| filter.head || !b.is_head)
            .collect();
        // Names that already exist, so typing one doesn't offer to create it.
        let mut exact: Vec<String> = Vec::new();
        for b in &branches {
            exact.push(b.name.clone());
            if b.remote {
                if let Some((_, rest)) = b.name.split_once('/') {
                    exact.push(rest.to_string());
                }
            }
        }
        exact.extend(tags.iter().map(|t| t.name.clone()));

        let t = cx.theme().clone();
        let mut items: Vec<PickerItem> = branches
            .iter()
            .map(|b| {
                let age = crate::git::relative_time_since(b.time);
                let mut note = format!("{} · {}", b.short_sha, age);
                if !b.summary.is_empty() {
                    note = format!("{note} · {}", b.summary);
                }
                if let Some(up) = &b.upstream {
                    note = format!("tracks {up} · {note}");
                }
                PickerItem {
                    haystack: b.name.clone(),
                    label_start: if b.remote {
                        b.name.find('/').map_or(0, |i| i + 1)
                    } else {
                        0
                    },
                    note,
                    icon: if b.remote { "cloud" } else { "branch" },
                    icon_color: Some(if b.remote { t.syn_type } else { t.accent }),
                    badge: b.is_head.then(|| ("current".to_string(), t.st_added)),
                    section: Some(if b.remote {
                        "Remote branches"
                    } else {
                        "Local branches"
                    }),
                }
            })
            .collect();
        items.extend(tags.iter().map(|tag| {
            let mut note = format!(
                "{} · {}",
                tag.short_sha,
                crate::git::relative_time_since(tag.time)
            );
            if !tag.summary.is_empty() {
                note = format!("{note} · {}", tag.summary);
            }
            PickerItem {
                haystack: tag.name.clone(),
                label_start: 0,
                note,
                icon: "tag",
                icon_color: Some(t.st_modified),
                badge: None,
                section: Some("Tags"),
            }
        }));
        let create = filter.create;
        let picker = cx.new(|cx| {
            let p = Picker::new(placeholder, "No branches match.", items, cx);
            if create {
                p.with_create(|q| format!("Create new branch '{q}'"), exact, cx)
            } else {
                p
            }
        });
        let count = branches.len();
        self.show_picker(
            picker,
            Box::new(move |this, pick, window, cx| {
                let choice = match pick {
                    Pick::Item(ix) if ix < count => RefChoice::Branch(branches[ix].clone()),
                    Pick::Item(ix) => match tags.get(ix - count) {
                        Some(tag) => RefChoice::Tag(tag.clone()),
                        None => return,
                    },
                    Pick::Text(name) => RefChoice::New(name),
                };
                then(this, choice, window, cx)
            }),
            window,
            cx,
        );
    }

    /// Shows `items` and calls `then` with the index chosen.
    pub(super) fn pick_list(
        &mut self,
        placeholder: impl Into<gpui::SharedString>,
        empty_text: impl Into<gpui::SharedString>,
        items: Vec<PickerItem>,
        then: impl FnOnce(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = cx.new(|cx| Picker::new(placeholder, empty_text, items, cx));
        self.show_picker(
            picker,
            Box::new(move |this, pick, window, cx| {
                if let Pick::Item(ix) = pick {
                    then(this, ix, window, cx)
                }
            }),
            window,
            cx,
        );
    }

    /// Asks for a line of text, like VS Code's input box.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ask_text(
        &mut self,
        placeholder: impl Into<gpui::SharedString>,
        hint: impl Into<String>,
        allow_empty: bool,
        initial: Option<String>,
        then: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker = cx.new(|cx| {
            let mut p = Picker::prompt(placeholder, hint, allow_empty, cx);
            if let Some(text) = initial {
                p.set_query(text, cx);
            }
            p
        });
        self.show_picker(
            picker,
            Box::new(move |this, pick, window, cx| {
                if let Pick::Text(text) = pick {
                    then(this, text, window, cx)
                }
            }),
            window,
            cx,
        );
    }

    pub(super) fn show_picker(
        &mut self,
        picker: Entity<Picker>,
        on_pick: OnPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self._picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, event: &PickerEvent, window, cx| {
                let pick = match event {
                    PickerEvent::QueryChanged(query) => {
                        if this.picker_is_files && query.starts_with('>') {
                            let query = query.clone();
                            this.open_command_palette(Some(query), window, cx);
                        }
                        return;
                    }
                    PickerEvent::Dismiss => None,
                    PickerEvent::Confirm(ix) => Some(Pick::Item(*ix)),
                    PickerEvent::Text(text) => Some(Pick::Text(text.clone())),
                };
                let on_pick = this.on_pick.take();
                this.close_picker(window, cx);
                if let (Some(pick), Some(on_pick)) = (pick, on_pick) {
                    on_pick(this, pick, window, cx);
                }
            },
        ));
        window.focus(&picker.focus_handle(cx));
        self.picker = Some(picker);
        self.on_pick = Some(on_pick);
        self.picker_is_files = false;
        cx.notify();
    }

    pub fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_pick = None;
        if self.picker.take().is_some() {
            self._picker_subscription = None;
            window.focus(&self.message.focus_handle(cx));
            cx.notify();
        }
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
        Row::Pair { left, right, .. } => {
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
        Row::Line { old, new, side, .. } => {
            (*old, *new, side.kind != crate::diff::LineKind::Context)
        }
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
            .on_action(
                cx.listener(|this, _: &QuickOpen, window, cx| this.open_file_picker(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_command_palette(None, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SwitchBranch, window, cx| {
                    this.open_branch_picker(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                if let Some(a) = this.active {
                    this.close_tab(a, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.remote(RemoteOp::Fetch, cx)))
            .on_action(cx.listener(|this, _: &Pull, _, cx| this.remote(RemoteOp::Pull, cx)))
            .on_action(cx.listener(|this, _: &Push, _, cx| this.remote(RemoteOp::Push, cx)))
            .on_action(cx.listener(|this, _: &Sync, _, cx| this.remote(RemoteOp::Sync, cx)))
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.menu.is_some() {
                        this.menu = None;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .relative()
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
            .children(self.picker.as_ref().map(|picker| {
                // Click outside the picker to close it.
                div()
                    .id("picker-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .justify_center()
                    .pt(px(48.))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_picker(window, cx)),
                    )
                    .child(
                        div()
                            .h_full()
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(picker.clone()),
                    )
            }))
    }
}

impl Workspace {
    pub fn message_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.message.focus_handle(cx)
    }
}

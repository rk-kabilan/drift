//! The main window: state, git operations and the top-level layout
//! (title bar, changes pane, diff pane, status bar).

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, Context, Entity, FocusHandle, Focusable, PromptLevel,
    UniformListScrollHandle, Window,
};

use crate::diff::{FileDiff, Layout, Row};
use crate::git::{Blame, BranchInfo, ChangeKind, FileChange, HeadInfo, Repo, TagInfo};
use crate::syntax::Lang;
use crate::theme::{ActiveTheme as _, Appearance, Theme, ThemeState};
use crate::ui::picker::{Picker, PickerEvent, PickerItem};
use crate::ui::text_input::TextInput;

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
        CloseWindow,
        NextTab,
        PreviousTab,
        Fetch,
        Pull,
        Push,
        Sync,
        Save,
        OpenRepository,
        NewWindow,
        OpenSettings,
        CheckForUpdates,
        ToggleWordWrap,
        ToggleGraph,
        TogglePanel,
        ToggleTerminal,
        NewTerminal,
        ShowOutput,
        ClearOutput,
        ToggleMaximizedPanel,
        KillTerminal,
        FocusNextTerminal,
        FocusPreviousTerminal,
        Quit
    ]
);

/// Opens a Drift window. `start` is as for [`Workspace::new`]; `cascade` is the
/// window to offset the new one from, so the two don't sit exactly on top of
/// each other.
pub fn open_window(
    start: Option<(PathBuf, bool)>,
    cascade: Option<gpui::Bounds<gpui::Pixels>>,
    cx: &mut gpui::App,
) {
    let bounds = match cascade {
        Some(b) => gpui::Bounds::new(b.origin + gpui::point(px(28.), px(28.)), b.size),
        None => gpui::Bounds::centered(None, gpui::size(px(1280.), px(800.)), cx),
    };
    let result = cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Drift".into()),
                appears_transparent: cfg!(target_os = "macos"),
                traffic_light_position: Some(gpui::point(px(12.), px(11.))),
            }),
            window_min_size: Some(gpui::size(px(760.), px(420.))),
            app_id: Some("drift".into()),
            ..Default::default()
        },
        move |window, cx| {
            // DRIFT_THEME=light|dark pins the theme; otherwise the Settings
            // choice applies, which by default follows the OS.
            let forced = match std::env::var("DRIFT_THEME").as_deref() {
                Ok("dark") => Some(Appearance::Dark),
                Ok("light") => Some(Appearance::Light),
                _ => cx.global::<crate::settings::Settings>().theme.forced(),
            };
            let appearance = forced.unwrap_or_else(|| Appearance::from(window.appearance()));
            let state = cx.global_mut::<ThemeState>();
            state.theme = Theme::for_appearance(appearance);
            state.forced = forced;
            cx.new(|cx| Workspace::new(start, window, cx))
        },
    );
    if let Err(e) = result {
        log::error!("could not open a window: {e:#}");
    }
}

/// Brings forward the window that has the repository at `root` open, if any.
pub(super) fn activate_repo_window(root: &std::path::Path, cx: &mut gpui::App) -> bool {
    let found = cx.windows().into_iter().find_map(|handle| {
        let ws = handle.downcast::<Workspace>()?;
        ws.read(cx)
            .is_ok_and(|w| w.repo.is_some() && w.root == root)
            .then_some(ws)
    });
    match found {
        Some(ws) => ws
            .update(cx, |_, window, _| window.activate_window())
            .is_ok(),
        None => false,
    }
}

/// Quits after every window with unsaved edits has been asked about, one at a time.
pub fn quit(cx: &mut gpui::App) {
    let unsaved = cx.windows().into_iter().find_map(|handle| {
        let ws = handle.downcast::<Workspace>()?;
        ws.read(cx).is_ok_and(|w| w.has_unsaved()).then_some(ws)
    });
    match unsaved {
        Some(ws) => {
            ws.update(cx, |this, window, cx| {
                window.activate_window();
                // Saved or discarded: look at the next window. Cancel stops here.
                this.confirm_unsaved(window, cx, |_, _, cx| cx.defer(quit));
            })
            .ok();
        }
        None => cx.quit(),
    }
}

/// Default width of the changes pane, and the limits the divider can drag it to.
const SIDEBAR_WIDTH: f32 = 300.;
const SIDEBAR_MIN: f32 = 200.;
/// Width the diff pane always keeps.
const DIFF_MIN: f32 = 320.;
/// Hit area of the divider between the panes (also the gap between them).
const DIVIDER: f32 = 6.;

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
    /// The right-click menu of tab `ix`, at window position `at`.
    Tab {
        ix: usize,
        at: gpui::Point<gpui::Pixels>,
    },
    /// The right-click menu of the selected changed files, at window position `at`.
    Files {
        at: gpui::Point<gpui::Pixels>,
    },
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
    /// Hash of both texts, to tell whether the file changed since the diff was
    /// drawn without keeping two full copies of it around.
    fingerprint: u64,
    /// Either side is a binary file (shown as a placeholder, not mergeable).
    pub binary: bool,
    /// Both sides are empty.
    pub empty: bool,
    pub file: FileDiff,
    pub rows: Vec<Row>,
    pub expanded: HashSet<usize>,
    pub left_label: String,
    pub right_label: String,
    pub lang: Lang,
    pub blame: Option<Blame>,
    pub cursor: Option<CursorLine>,
    /// Where each row starts once long lines are wrapped (only while word wrap is on).
    pub wrap: Option<super::diff_pane::WrapMap>,
}

/// A file as changed by a commit in the history graph, shown read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitTab {
    pub oid: git2::Oid,
    pub short_sha: String,
    pub author: String,
    pub summary: String,
    pub time: i64,
    /// The path before the commit, when it renamed the file.
    pub old_path: Option<String>,
    pub kind: ChangeKind,
}

/// An open diff. A single click opens a file in the preview tab (shown in
/// italics, replaced by the next single click); a double click pins it.
pub struct Tab {
    pub staged: bool,
    pub path: String,
    /// Set for a file from a commit in the history graph.
    pub commit: Option<CommitTab>,
    pub pinned: bool,
    /// Only the active tab keeps its diff in memory; others are rebuilt when shown.
    pub diff: Option<DiffState>,
    /// Folds and cursor of an unloaded tab, restored when it is shown again.
    saved_view: Option<(HashSet<usize>, Option<CursorLine>)>,
    pub scroll: UniformListScrollHandle,
    /// Horizontal scroll of the code, in pixels (shared by both sides).
    pub h_scroll: f32,
    /// Unsaved edits made with the merge arrows. Nothing is written to the
    /// working tree or the index until the tab is saved.
    pub draft: Option<Draft>,
}

/// Edited text for either side of a diff, waiting to be saved.
#[derive(Clone, Debug)]
pub struct Draft {
    /// New text for the left side (the index, for an unstaged file).
    pub left: Option<String>,
    /// New text for the right side (the working tree, or the index for a staged file).
    pub right: Option<String>,
    /// Fingerprint of the files on disk when editing began, to catch outside changes.
    base: u64,
}

const BINARY: &str = "(binary file)";

pub(super) const UNSAVED_BEFORE_COMMIT: &str =
    "Some diffs have unsaved edits. Save or discard them before committing.";

fn fingerprint(old: &str, new: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    old.hash(&mut h);
    new.hash(&mut h);
    h.finish()
}

pub struct Workspace {
    pub(super) repo: Option<Repo>,
    pub(super) root: PathBuf,
    /// Why the last attempt to open a repository failed, shown on the welcome screen.
    pub(super) welcome_error: Option<String>,
    /// Recently opened repositories, newest first.
    pub(super) recent: Vec<PathBuf>,
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
    /// The bottom panel: Output and Terminal tabs.
    pub(super) panel: super::panel::Panel,
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
    /// The tab strip's scroll position, and the tab last scrolled into view.
    pub(super) tab_scroll: gpui::ScrollHandle,
    pub(super) tab_scrolled: std::cell::Cell<Option<usize>>,
    /// Changed files picked with Cmd/Ctrl- or Shift-click, as (staged, path).
    pub(super) picked: Vec<(bool, String)>,
    /// Where a Shift-click range starts.
    pub(super) pick_anchor: Option<(bool, String)>,
    /// `user.name` from the git config: the author of lines not committed yet.
    pub(super) user_name: Option<String>,
    /// The history graph at the bottom of the changes pane.
    pub(super) history: super::history::History,
    /// A background fetch is running.
    pub(super) auto_fetching: bool,
    /// Background fetches that failed in a row; each one doubles the wait.
    auto_fetch_failures: u32,
    /// Why the last background fetch failed, shown in the sync tooltip.
    pub(super) auto_fetch_error: Option<String>,
    /// Width of the changes pane, set by dragging the divider next to it.
    pub(super) sidebar_width: f32,
    /// While the divider is dragged: the pointer x and the pane width at the start.
    sidebar_drag: Option<(f32, f32)>,
    /// The diff scrollbar thumb being dragged, if any.
    pub(super) scrollbar_drag: Option<super::diff_pane::ScrollbarDrag>,
    /// Width of the diff list, measured at paint, for clamping horizontal scroll.
    pub(super) diff_width: std::rc::Rc<std::cell::Cell<f32>>,
    /// Advance of one character of the code font.
    pub(super) char_width: Option<f32>,
    /// The axis the current scroll gesture is locked to (true = sideways), and
    /// when its last event arrived.
    pub(super) scroll_axis: Option<(bool, std::time::Instant)>,
    /// Vertical offset held while a sideways gesture runs.
    pub(super) lock_y: f32,
    /// The settings panel, while it is open.
    pub(super) settings: Option<super::settings_panel::SettingsView>,
    /// An update check started from the status bar or Settings is running.
    pub(super) checking_updates: bool,
    focus_handle: FocusHandle,
    _picker_subscription: Option<gpui::Subscription>,
    _tasks: Vec<gpui::Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl Workspace {
    /// `start` is the folder to open, and whether the user named it (a failure is
    /// then reported). Without a repository the welcome screen shows.
    pub fn new(
        start: Option<(PathBuf, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let message = cx.new(|cx| TextInput::new("Message", cx));
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                if this.repo.is_none() {
                    // Another window may have opened a repository meanwhile.
                    this.recent = crate::recent::load();
                    cx.notify();
                }
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

        // Closing the window with unsaved edits asks first.
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| {
                if !this.has_unsaved() {
                    return true;
                }
                this.confirm_unsaved(window, cx, |_, window, _| window.remove_window());
                false
            })
            .unwrap_or(true)
        });

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
            repo: None,
            root: PathBuf::new(),
            welcome_error: None,
            recent: Vec::new(),
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
            panel: super::panel::Panel::new(cx),
            busy: None,
            status_note: None,
            picker: None,
            on_pick: None,
            picker_is_files: false,
            recent_commands: Vec::new(),
            show_blame: true,
            tab_scroll: Default::default(),
            tab_scrolled: Default::default(),
            picked: Vec::new(),
            pick_anchor: None,
            user_name: None,
            history: Default::default(),
            auto_fetching: false,
            auto_fetch_failures: 0,
            auto_fetch_error: None,
            sidebar_width: SIDEBAR_WIDTH,
            sidebar_drag: None,
            scrollbar_drag: None,
            diff_width: Default::default(),
            char_width: None,
            scroll_axis: None,
            lock_y: 0.,
            settings: None,
            checking_updates: false,
            focus_handle: cx.focus_handle(),
            _picker_subscription: None,
            _tasks: vec![poll, Self::auto_fetch_loop(window, cx)],
            _subscriptions: subscriptions,
        };
        let repo = start.and_then(|(path, named)| match Repo::discover(&path) {
            Ok(repo) => Some(repo),
            Err(_) => {
                if named {
                    this.welcome_error =
                        Some(format!("No git repository found at {}.", path.display()));
                }
                None
            }
        });
        this.switch_repo(repo, window, cx);
        this
    }

    /// Replaces the open repository (or closes it with `None`, which shows the
    /// welcome screen), resetting everything that belonged to the old one.
    pub(super) fn switch_repo(
        &mut self,
        repo: Option<Repo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.clear();
        self.active = None;
        self.staged.clear();
        self.unstaged.clear();
        self.picked.clear();
        self.pick_anchor = None;
        self.head = HeadInfo::default();
        self.menu = None;
        self.status_note = None;
        self.panel.alert = None;
        self.picker = None;
        self.on_pick = None;
        self._picker_subscription = None;
        self.picker_is_files = false;
        self.scroll_axis = None;
        self.scrollbar_drag = None;
        self.lock_y = 0.;
        self.history = Default::default();
        self.auto_fetch_failures = 0;
        self.auto_fetch_error = None;
        self.message.update(cx, |m, cx| m.set_text("", cx));
        match repo {
            Some(repo) => {
                self.welcome_error = None;
                self.root = repo.root().to_path_buf();
                self.user_name = repo.user_name();
                self.repo = Some(repo);
                self.recent = crate::recent::add(&self.root);
                self.refresh(cx);
                // Show the first changed file straight away.
                if let Some(first) = self.unstaged.first().or(self.staged.first()).cloned() {
                    self.open(first.staged, first.path, false, cx);
                }
                window.focus(&self.message.focus_handle(cx));
            }
            None => {
                self.repo = None;
                self.recent = crate::recent::load();
                window.focus(&self.focus_handle);
            }
        }
        cx.notify();
    }

    pub fn title(&self) -> String {
        if self.repo.is_none() {
            return "Drift".into();
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
                self.panel.refresh_error = None;
                let dirty =
                    !staged.is_empty() || unstaged.iter().any(|c| c.kind != ChangeKind::Untracked);
                self.head = repo.head_info(dirty);
                self.staged = staged;
                self.unstaged = unstaged;
                let (s, u) = (&self.staged, &self.unstaged);
                self.picked
                    .retain(|(st, p)| (if *st { s } else { u }).iter().any(|c| &c.path == p));
            }
            Err(e) => {
                // Refresh runs every few seconds: report a failure once, not each time.
                let message = format!("{e:#}");
                if self.panel.refresh_error.as_ref() != Some(&message) {
                    self.report_error(message.clone(), cx);
                }
                self.panel.refresh_error = Some(message);
            }
        }

        // A tab follows its file between the two groups (e.g. after staging) and
        // closes when the file has no changes left.
        let active_key = self
            .active_tab()
            .map(|t| (t.staged, t.path.clone(), t.commit.as_ref().map(|c| c.oid)));
        let mut kept: Vec<Tab> = Vec::with_capacity(self.tabs.len());
        for mut tab in std::mem::take(&mut self.tabs) {
            // A file from the history doesn't depend on the working tree.
            if tab.commit.is_some() {
                kept.push(tab);
                continue;
            }
            let in_list = |staged: bool| self.list(staged).iter().any(|c| c.path == tab.path);
            // A tab with unsaved edits stays put, so they are never lost silently.
            if !in_list(tab.staged) && tab.draft.is_none() {
                if in_list(!tab.staged) {
                    tab.staged = !tab.staged;
                    tab.diff = None;
                } else {
                    continue;
                }
            }
            if kept
                .iter()
                .any(|k| k.commit.is_none() && k.staged == tab.staged && k.path == tab.path)
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
                .and_then(|(s, p, c)| {
                    let oid = |t: &Tab| t.commit.as_ref().map(|c| c.oid);
                    self.tabs
                        .iter()
                        .position(|t| t.path == p && t.staged == s && oid(t) == c)
                        .or_else(|| self.tabs.iter().position(|t| t.path == p && oid(t) == c))
                })
                .or(Some(self.active.unwrap_or(0).min(self.tabs.len() - 1)))
        };
        self.load_active_tab();
        self.refresh_history(cx);

        let message_placeholder = match super::shortcuts::label("Commit", cx) {
            Some(keys) => format!("Message ({keys} to commit on '{}')", self.head.branch),
            None => format!("Message (commit on '{}')", self.head.branch),
        };
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
        self.open_tab(staged, path, None, pin, cx)
    }

    /// Shows a file as a commit in the history changed it.
    pub fn open_commit_file(
        &mut self,
        commit: CommitTab,
        path: String,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        self.open_tab(true, path, Some(commit), pin, cx)
    }

    fn open_tab(
        &mut self,
        staged: bool,
        path: String,
        commit: Option<CommitTab>,
        pin: bool,
        cx: &mut Context<Self>,
    ) {
        let oid = commit.as_ref().map(|c| c.oid);
        if let Some(ix) = self.tabs.iter().position(|t| {
            t.path == path
                && t.commit.as_ref().map(|c| c.oid) == oid
                && (oid.is_some() || t.staged == staged)
        }) {
            if pin {
                self.tabs[ix].pinned = true;
            }
            self.active = Some(ix);
            self.load_active_tab();
        } else {
            let tab = Tab {
                staged,
                path,
                commit,
                pinned: pin,
                diff: None,
                saved_view: None,
                scroll: UniformListScrollHandle::new(),
                h_scroll: 0.,
                draft: None,
            };
            match self
                .tabs
                .iter()
                .position(|t| !t.pinned && t.draft.is_none())
            {
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
            self.unload_inactive_tabs();
        }
        cx.notify();
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.active = Some(ix);
            self.load_active_tab();
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
        self.load_active_tab();
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(a) = self.active {
            let n = self.tabs.len() as isize;
            self.active = Some(((a as isize + delta).rem_euclid(n)) as usize);
            self.load_active_tab();
            cx.notify();
        }
    }

    /// Loads (or refreshes) the active tab's diff and frees every other tab's,
    /// so memory stays at one diff however many tabs are open.
    fn load_active_tab(&mut self) {
        self.unload_inactive_tabs();
        if let Some(a) = self.active.filter(|&a| a < self.tabs.len()) {
            self.load_tab(a, false);
        }
    }

    fn unload_inactive_tabs(&mut self) {
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if Some(i) != self.active {
                if let Some(d) = tab.diff.take() {
                    tab.saved_view = Some((d.expanded, d.cursor));
                }
            }
        }
    }

    /// (Re)computes a tab's diff. Keeps fold and cursor state unless `reset_view`.
    fn load_tab(&mut self, ix: usize, reset_view: bool) {
        let Some(repo) = &self.repo else { return };
        let (staged, path) = (self.tabs[ix].staged, self.tabs[ix].path.clone());
        let (change, (old, new, mut left_label, mut right_label)) = if let Some(commit) =
            &self.tabs[ix].commit
        {
            let change = FileChange {
                path: path.clone(),
                kind: commit.kind,
                staged: true,
            };
            let (old, new) = repo
                .commit_texts(commit.oid, &path, commit.old_path.as_deref())
                .unwrap_or_default();
            let left = format!("{}^ (parent)", commit.short_sha);
            let right = format!("{} · {}", commit.short_sha, commit.summary);
            (change, (old, new, left, right))
        } else {
            let Some(change) = self.list(staged).iter().find(|c| c.path == path).cloned() else {
                return;
            };
            let texts = repo.change_texts(&change);
            (change, texts)
        };
        let from_history = self.tabs[ix].commit.is_some();
        let tab = &mut self.tabs[ix];
        // Unsaved edits replace what is on disk in the view.
        let (old, new) = match &tab.draft {
            Some(draft) => {
                if draft.left.is_some() {
                    left_label.push_str(" · unsaved");
                }
                if draft.right.is_some() {
                    right_label.push_str(" · unsaved");
                }
                (
                    draft.left.clone().unwrap_or(old),
                    draft.right.clone().unwrap_or(new),
                )
            }
            None => (old, new),
        };

        let print = fingerprint(&old, &new);
        if !reset_view {
            if let Some(d) = &mut tab.diff {
                if d.fingerprint == print && d.change == change {
                    d.left_label = left_label;
                    d.right_label = right_label;
                    return;
                }
            }
        }
        let file = FileDiff::new(&old, &new);
        let saved = tab.saved_view.take();
        let (expanded, cursor) = match (tab.diff.take(), saved, reset_view) {
            (_, _, true) => (HashSet::new(), None),
            (Some(d), _, false) => (d.expanded, d.cursor),
            (None, Some(view), false) => view,
            (None, None, false) => (HashSet::new(), None),
        };
        let rows = file.rows(self.layout, &expanded);
        let blame = if from_history || change.kind == ChangeKind::Untracked || new.is_empty() {
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
            tab.h_scroll = 0.;
        }
        tab.diff = Some(DiffState {
            change,
            fingerprint: print,
            binary: old == BINARY || new == BINARY,
            empty: old.is_empty() && new.is_empty(),
            file,
            rows,
            expanded,
            left_label,
            right_label,
            lang,
            blame,
            cursor,
            wrap: None,
        });
    }

    pub fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let layout = self.layout;
        for tab in &mut self.tabs {
            if let Some(d) = &mut tab.diff {
                d.rows = d.file.rows(layout, &d.expanded);
                d.wrap = None;
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
            d.wrap = None;
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
        let visual = d.wrap.as_ref().map_or(target, |w| w.visual_of(target));
        tab.scroll
            .scroll_to_item(visual.saturating_sub(3), gpui::ScrollStrategy::Top);
        cx.notify();
    }

    /// Which merge arrows make sense for the active diff. Left is the index (or
    /// HEAD) and right the working tree for unstaged files; for staged files left
    /// is HEAD and right the index, and HEAD cannot be written.
    pub fn merge_directions(&self) -> (bool, bool) {
        if self.active_tab().is_some_and(|t| t.commit.is_some()) {
            // History is read-only.
            return (false, false);
        }
        match self.diff() {
            Some(d) if !d.binary => {
                let to_right = true;
                let to_left = !d.change.staged;
                (to_right, to_left)
            }
            _ => (false, false),
        }
    }

    /// Copies one change block across the diff into the tab's unsaved draft:
    /// towards the working tree (revert), into the index (stage just this
    /// block), or from HEAD into the index (unstage just this block). Nothing
    /// is written until the tab is saved.
    pub fn merge_block(&mut self, block: usize, direction: MergeDirection, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let Some(d) = self.diff() else { return };
        let Some(range) = d.file.blocks().get(block).cloned() else {
            return;
        };
        let (change, shown) = (d.change.clone(), d.fingerprint);
        if direction == MergeDirection::ToLeft && change.staged {
            self.report_warning("HEAD can't be changed from the diff.", cx);
            cx.notify();
            return;
        }
        let Some(repo) = &self.repo else { return };
        let (disk_old, disk_new, _, _) = repo.change_texts(&change);
        let tab = &mut self.tabs[ix];
        let draft = tab.draft.get_or_insert_with(|| Draft {
            left: None,
            right: None,
            base: fingerprint(&disk_old, &disk_new),
        });
        let old = draft.left.clone().unwrap_or_else(|| disk_old.clone());
        let new = draft.right.clone().unwrap_or_else(|| disk_new.clone());
        if fingerprint(&old, &new) != shown {
            // The view is stale (the file changed outside); redraw before editing.
            if draft.left.is_none() && draft.right.is_none() {
                tab.draft = None;
            }
            self.report_warning(
                format!(
                    "{} changed on disk. The diff was refreshed; try again.",
                    change.file_name()
                ),
                cx,
            );
            self.load_tab(ix, false);
            cx.notify();
            return;
        }
        match direction {
            MergeDirection::ToRight => {
                draft.right = Some(crate::merge::splice_lines(&new, range.new, &old, range.old))
            }
            MergeDirection::ToLeft => {
                draft.left = Some(crate::merge::splice_lines(&old, range.old, &new, range.new))
            }
        }
        // A side edited back to what is on disk is no longer unsaved.
        if draft.left.as_deref() == Some(disk_old.as_str()) {
            draft.left = None;
        }
        if draft.right.as_deref() == Some(disk_new.as_str()) {
            draft.right = None;
        }
        if draft.left.is_none() && draft.right.is_none() {
            tab.draft = None;
        }
        // Editing a preview tab keeps it open, as in an editor.
        tab.pinned = true;
        self.load_tab(ix, false);
        cx.notify();
    }

    pub fn has_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.draft.is_some())
    }

    /// Writes a tab's draft to the index and/or working tree. Asks first if the
    /// file changed on disk since editing began.
    pub fn save_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let Some(draft) = tab.draft.clone() else {
            return;
        };
        let (staged, path) = (tab.staged, tab.path.clone());
        let Some(repo) = &self.repo else { return };
        let on_disk = self.list(staged).iter().find(|c| c.path == path).map(|c| {
            let (old, new, _, _) = repo.change_texts(c);
            fingerprint(&old, &new)
        });
        if on_disk != Some(draft.base) {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            let answer = window.prompt(
                PromptLevel::Warning,
                &format!("{name} changed on disk since you started editing."),
                Some("Saving replaces those outside changes with your edits."),
                &["Overwrite", "Cancel"],
                cx,
            );
            cx.spawn(async move |this, cx| {
                if answer.await == Ok(0) {
                    this.update(cx, |this, cx| this.write_draft(ix, cx)).ok();
                }
            })
            .detach();
            return;
        }
        self.write_draft(ix, cx);
    }

    pub(super) fn write_draft(&mut self, ix: usize, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self.tabs.get(ix) else {
            return false;
        };
        let Some(draft) = tab.draft.clone() else {
            return true;
        };
        let (staged, path) = (tab.staged, tab.path.clone());
        let ok = self.run(cx, |r| {
            if staged {
                // Left is HEAD (read-only); right is the index.
                if let Some(text) = &draft.right {
                    r.write_index(&path, text)?;
                }
            } else {
                if let Some(text) = &draft.left {
                    r.write_index(&path, text)?;
                }
                if let Some(text) = &draft.right {
                    r.write_worktree(&path, text)?;
                }
            }
            Ok(())
        });
        if ok {
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|t| t.staged == staged && t.path == path)
            {
                tab.draft = None;
            }
            self.refresh(cx);
            self.flash("Saved", cx);
        }
        ok
    }

    /// Drops a tab's unsaved edits and shows the files on disk again.
    pub fn discard_draft(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(ix) {
            tab.draft = None;
            self.load_tab(ix, false);
            cx.notify();
        }
    }

    /// Closes every tab `close` picks. Tabs with unsaved edits stay open, so
    /// edits are never thrown away by a bulk close.
    pub fn close_tabs(&mut self, close: impl Fn(usize) -> bool, cx: &mut Context<Self>) {
        let active = self
            .active
            .and_then(|a| self.tabs.get(a))
            .map(|t| (t.staged, t.path.clone(), t.commit.as_ref().map(|c| c.oid)));
        let mut kept_unsaved = 0;
        let mut ix = 0;
        self.tabs.retain(|t| {
            let drop = close(ix);
            ix += 1;
            if drop && t.draft.is_some() {
                kept_unsaved += 1;
                return true;
            }
            !drop
        });
        self.active = active
            .and_then(|(s, p, c)| {
                self.tabs.iter().position(|t| {
                    t.staged == s && t.path == p && t.commit.as_ref().map(|c| c.oid) == c
                })
            })
            .or(if self.tabs.is_empty() { None } else { Some(0) });
        if kept_unsaved > 0 {
            self.report_warning(
                format!(
                "{kept_unsaved} tab{} with unsaved edits stayed open. Save or discard them first.",
                if kept_unsaved == 1 { "" } else { "s" }
            ),
                cx,
            );
        }
        self.load_active_tab();
        cx.notify();
    }

    /// Closes a tab, asking to save its unsaved edits first.
    pub fn request_close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if tab.draft.is_none() {
            self.close_tab(ix, cx);
            return;
        }
        let name = tab.path.rsplit('/').next().unwrap_or(&tab.path).to_string();
        let key = (tab.staged, tab.path.clone());
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save your changes to {name}?"),
            Some("Your changes will be lost if you don't save them."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let choice = answer.await;
            this.update(cx, |this, cx| {
                // The tab may have moved while the prompt was open.
                let Some(ix) = this
                    .tabs
                    .iter()
                    .position(|t| (t.staged, t.path.clone()) == key)
                else {
                    return;
                };
                match choice {
                    Ok(0) => {
                        if this.write_draft(ix, cx) {
                            if let Some(ix) = this
                                .tabs
                                .iter()
                                .position(|t| (t.staged, t.path.clone()) == key)
                            {
                                this.close_tab(ix, cx);
                            }
                        }
                    }
                    Ok(1) => {
                        this.tabs[ix].draft = None;
                        this.close_tab(ix, cx);
                    }
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }

    /// Runs `then` once every unsaved draft is saved or discarded, asking first.
    pub fn confirm_unsaved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let count = self.tabs.iter().filter(|t| t.draft.is_some()).count();
        if count == 0 {
            then(self, window, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Save changes to {count} file{}?",
                if count == 1 { "" } else { "s" }
            ),
            Some("Your changes will be lost if you don't save them."),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await;
            this.update_in(cx, |this, window, cx| {
                match choice {
                    Ok(0) => {
                        let mut ok = true;
                        while let Some(ix) = this.tabs.iter().position(|t| t.draft.is_some()) {
                            if !this.write_draft(ix, cx) {
                                ok = false;
                                break;
                            }
                        }
                        if !ok {
                            return;
                        }
                    }
                    Ok(1) => {
                        for tab in &mut this.tabs {
                            tab.draft = None;
                        }
                    }
                    _ => return,
                }
                then(this, window, cx)
            })
            .ok();
        })
        .detach();
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
        if let Err(e) = result {
            self.report_error(format!("{e:#}"), cx);
        }
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
            self.report_warning(format!("{path} no longer exists in the working tree."), cx);
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
        if self.has_unsaved() {
            self.report_warning(UNSAVED_BEFORE_COMMIT, cx);
            cx.notify();
            return;
        }
        let message = self.message.read(cx).text().to_string();
        if mode == CommitMode::Staged && self.staged.is_empty() {
            self.report_warning(
                if self.unstaged.is_empty() {
                    "There are no changes to commit.".to_string()
                } else {
                    "No staged changes. Stage files with +, or choose Commit All from the menu."
                        .to_string()
                },
                cx,
            );
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
            // Let a background fetch finish first, so the two don't fight over refs.
            while this.read_with(cx, |t, _| t.auto_fetching).unwrap_or(false) {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
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
                    Err(e) => this.report_error(format!("{e:#}"), cx),
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

    /// Fetches in the background every few seconds (see Settings). Runs one
    /// fetch at a time, skips while another git operation runs, and waits
    /// longer after each failure so an offline or locked remote isn't hammered.
    fn auto_fetch_loop(window: &mut Window, cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn_in(window, async move |this, cx| loop {
            let wait = this
                .update(cx, |this, cx| {
                    let s = cx.global::<crate::settings::Settings>();
                    let base = s
                        .auto_fetch_interval
                        .max(crate::settings::MIN_FETCH_INTERVAL);
                    let backoff = 1u64 << this.auto_fetch_failures.min(7);
                    // Without auto fetch, look again shortly in case it is turned on.
                    if s.auto_fetch {
                        (base * backoff).min(30 * 60)
                    } else {
                        2
                    }
                })
                .unwrap_or(5);
            cx.background_executor()
                .timer(Duration::from_secs(wait))
                .await;
            let Ok(Some(root)) = this.update(cx, |this, cx| this.auto_fetch_due(cx)) else {
                if this.update(cx, |_, _| ()).is_err() {
                    break;
                }
                continue;
            };
            let result = cx
                .background_spawn(async move { Repo::auto_fetch(&root) })
                .await;
            let alive = this
                .update(cx, |this, cx| {
                    this.auto_fetching = false;
                    match result {
                        Ok(()) => {
                            this.auto_fetch_failures = 0;
                            this.auto_fetch_error = None;
                        }
                        Err(e) => {
                            let msg = format!("{e:#}");
                            if this.auto_fetch_error.as_deref() != Some(msg.as_str()) {
                                log::warn!("background fetch failed: {msg}");
                            }
                            this.auto_fetch_failures += 1;
                            this.auto_fetch_error = Some(msg);
                        }
                    }
                    if this.busy.is_none() {
                        this.refresh(cx);
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
    }

    /// The repository root, when a background fetch should start now (and
    /// marks it as running).
    fn auto_fetch_due(&mut self, cx: &mut Context<Self>) -> Option<PathBuf> {
        if !cx.global::<crate::settings::Settings>().auto_fetch
            || self.auto_fetching
            || self.busy.is_some()
        {
            return None;
        }
        let repo = self.repo.as_ref()?;
        if !repo.has_remote() {
            return None;
        }
        self.auto_fetching = true;
        Some(self.root.clone())
    }

    /// Turns word wrap in the diff on or off, and remembers it.
    pub fn toggle_word_wrap(&mut self, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.word_wrap = !s.word_wrap);
        if let Some(tab) = self.active.and_then(|a| self.tabs.get_mut(a)) {
            tab.h_scroll = 0.;
            if let Some(d) = &mut tab.diff {
                d.wrap = None;
            }
        }
        cx.notify();
    }

    pub(super) fn word_wrap(&self, cx: &gpui::App) -> bool {
        cx.global::<crate::settings::Settings>().word_wrap
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
                self.report_error(format!("{e:#}"), cx);
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
        // A picker replaces the Settings panel (e.g. the command palette opened from it).
        self.settings = None;
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

    /// Switches between light and dark, and remembers the choice in Settings.
    pub fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = match cx.theme().appearance {
            Appearance::Light => crate::settings::ThemeChoice::Dark,
            Appearance::Dark => crate::settings::ThemeChoice::Light,
        };
        self.set_theme(next, window, cx);
    }

    /// Applies a theme choice to every window and saves it in Settings.
    pub fn set_theme(
        &mut self,
        choice: crate::settings::ThemeChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let appearance = choice
            .forced()
            .unwrap_or_else(|| Appearance::from(window.appearance()));
        let state = cx.global_mut::<ThemeState>();
        state.forced = choice.forced();
        state.theme = Theme::for_appearance(appearance);
        crate::settings::update(cx, |s| s.theme = choice);
        cx.refresh_windows();
    }

    /// The title bar shows only the repository and branch. It is drawn by the app
    /// on macOS (transparent title bar) and on Linux desktops without server-side
    /// decorations, where it also carries the window controls.
    /// The draggable divider between the changes pane and the diff pane.
    /// Double-click puts it back to the default width.
    /// Closes this window, asking about unsaved edits first.
    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_unsaved() {
            self.confirm_unsaved(window, cx, |_, window, _| window.remove_window());
        } else {
            window.remove_window();
        }
    }

    /// The diff, and the bottom panel under it.
    fn render_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let maximized = self.panel.open && self.panel.maximized;
        let mut main = div().flex_1().min_w_0().h_full().flex().flex_col();
        if !maximized {
            main = main.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_diff_pane(window, cx)),
            );
        }
        if self.panel.open {
            if !maximized {
                main = main.child(self.render_panel_grip(cx));
            }
            main = main.child(self.render_panel(window, cx));
        }
        main
    }

    fn render_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let line = t.accent;
        let active = self.sidebar_drag.is_some();
        div()
            .id("sidebar-divider")
            .group("sidebar-divider")
            .w(px(DIVIDER))
            .h_full()
            .flex_none()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .child(
                div()
                    .w(px(2.))
                    .h_full()
                    .rounded(px(1.))
                    .when(active, |d| d.bg(line))
                    .group_hover("sidebar-divider", |s| s.bg(line.opacity(0.6))),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if e.click_count >= 2 {
                        this.sidebar_width = SIDEBAR_WIDTH;
                        this.sidebar_drag = None;
                    } else {
                        this.sidebar_drag = Some((f32::from(e.position.x), this.sidebar_width));
                    }
                    cx.notify();
                }),
            )
    }

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
            .on_mouse_down(gpui::MouseButton::Left, move |e, window, _| {
                if e.click_count >= 2 {
                    // macOS does what System Settings asks for (zoom by default);
                    // elsewhere double-click maximizes or restores.
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else {
                        window.zoom_window();
                    }
                } else if client_decorated {
                    window.start_window_move();
                }
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

        let body: gpui::AnyElement = if self.repo.is_none() {
            self.render_welcome(cx)
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .px(px(6.))
                .pb(px(6.))
                .child(self.render_sidebar(window, cx))
                .child(self.render_divider(cx))
                .child(self.render_main(window, cx))
                .into_any_element()
        };

        let dragging = self.sidebar_drag.is_some();
        let panel_dragging = self.panel.drag.is_some();
        let scrollbar_dragging = self.scrollbar_drag.is_some();
        div()
            .id("workspace")
            .key_context("Workspace")
            .when(scrollbar_dragging, |d| {
                d.on_mouse_move(
                    cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| this.drag_scrollbar(e, cx)),
                )
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.scrollbar_drag = None;
                        cx.notify();
                    }),
                )
            })
            .when(panel_dragging, |d| {
                d.cursor_row_resize()
                    .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, window, cx| {
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            // The button came up outside the window.
                            this.end_panel_drag(cx);
                        } else {
                            this.drag_panel(f32::from(e.position.y), window, cx);
                        }
                    }))
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.end_panel_drag(cx)),
                    )
            })
            .when(dragging, |d| {
                d.cursor_col_resize()
                    .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, window, cx| {
                        let Some((start_x, start_w)) = this.sidebar_drag else {
                            return;
                        };
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            // The button came up outside the window.
                            this.sidebar_drag = None;
                        } else {
                            let max =
                                (f32::from(window.viewport_size().width) - DIFF_MIN - 3. * DIVIDER)
                                    .max(SIDEBAR_MIN);
                            this.sidebar_width = (start_w + f32::from(e.position.x) - start_x)
                                .clamp(SIDEBAR_MIN, max);
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.sidebar_drag = None;
                            cx.notify();
                        }),
                    )
            })
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
            .on_action(
                cx.listener(|this, _: &ToggleTheme, window, cx| this.toggle_theme(window, cx)),
            )
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
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                // As in VS Code: with no diff open, Cmd/Ctrl+W closes the window.
                match this.active {
                    Some(a) => this.request_close_tab(a, window, cx),
                    None => this.close_window(window, cx),
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseWindow, window, cx| this.close_window(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Save, window, cx| {
                if let Some(a) = this.active {
                    this.save_tab(a, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.remote(RemoteOp::Fetch, cx)))
            .on_action(cx.listener(|this, _: &Pull, _, cx| this.remote(RemoteOp::Pull, cx)))
            .on_action(cx.listener(|this, _: &Push, _, cx| this.remote(RemoteOp::Push, cx)))
            .on_action(cx.listener(|this, _: &Sync, _, cx| this.remote(RemoteOp::Sync, cx)))
            .on_action(
                cx.listener(|this, _: &OpenRepository, window, cx| {
                    this.open_repo_dialog(window, cx)
                }),
            )
            .on_action(cx.listener(|_, _: &NewWindow, window, cx| {
                let cascade = window.window_bounds().get_bounds();
                cx.defer(move |cx| open_window(None, Some(cascade), cx));
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings(super::settings_panel::Page::General, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| {
                this.check_for_updates(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleWordWrap, _, cx| this.toggle_word_wrap(cx)))
            .on_action(cx.listener(|this, _: &ToggleGraph, _, cx| this.toggle_graph(cx)))
            .on_action(
                cx.listener(|this, _: &TogglePanel, window, cx| this.toggle_panel(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleTerminal, window, cx| {
                    this.toggle_terminal(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &NewTerminal, window, cx| this.new_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowOutput, window, cx| this.show_output(window, cx)))
            .on_action(cx.listener(|this, _: &ClearOutput, _, cx| this.clear_output(cx)))
            .on_action(cx.listener(|this, _: &ToggleMaximizedPanel, window, cx| {
                this.toggle_maximized_panel(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &KillTerminal, window, cx| this.kill_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusNextTerminal, window, cx| {
                this.cycle_terminal(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPreviousTerminal, window, cx| {
                this.cycle_terminal(-1, window, cx)
            }))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.defer(quit)))
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
            .children(self.render_tab_menu(cx))
            .children(self.render_files_menu(cx))
            .children(self.render_settings(window, cx))
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

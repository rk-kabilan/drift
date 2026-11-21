//! The command palette (Cmd+Shift+P): every git command by name, like VS Code's
//! "Git: …" commands. Commands that need more input chain pickers and prompts.

use gpui::{prelude::*, Context, PromptLevel, Window};

use super::picker::{Picker, PickerItem};
use super::workspace::{Pick, RefChoice, RefFilter, RemoteOp, Workspace};
use crate::git::{ChangeKind, CommitOptions, FileChange, Repo};
use crate::theme::ActiveTheme as _;

macro_rules! commands {
    ($($variant:ident => $title:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Cmd { $($variant),* }

        impl Cmd {
            pub const ALL: &[Cmd] = &[$(Cmd::$variant),*];

            pub fn title(self) -> &'static str {
                match self { $(Cmd::$variant => $title),* }
            }
        }
    };
}

commands! {
    AddRemote => "Add Remote...",
    AddToGitignore => "Add to .gitignore",
    ApplyLatestStash => "Apply Latest Stash",
    ApplyStash => "Apply Stash...",
    Checkout => "Checkout to...",
    CheckoutDetached => "Checkout to (Detached)...",
    CherryPick => "Cherry Pick...",
    CloseAllDiffs => "Close All Diff Editors",
    Commit => "Commit",
    CommitAmend => "Commit (Amend)",
    CommitSignedOff => "Commit (Signed Off)",
    CommitAll => "Commit All",
    CommitAllAmend => "Commit All (Amend)",
    CommitAllSignedOff => "Commit All (Signed Off)",
    CommitEmpty => "Commit Empty",
    CommitStaged => "Commit Staged",
    CommitStagedAmend => "Commit Staged (Amend)",
    CommitStagedSignedOff => "Commit Staged (Signed Off)",
    CreateBranch => "Create Branch...",
    CreateBranchFrom => "Create Branch From...",
    CreateTag => "Create Tag...",
    DeleteBranch => "Delete Branch...",
    DeleteRemoteBranch => "Delete Remote Branch...",
    DeleteRemoteTag => "Delete Remote Tag...",
    DeleteTag => "Delete Tag...",
    DiscardAll => "Discard All Changes",
    DiscardAllTracked => "Discard All Tracked Changes",
    DiscardAllUntracked => "Discard All Untracked Changes",
    DiscardChanges => "Discard Changes",
    DropAllStashes => "Drop All Stashes...",
    DropStash => "Drop Stash...",
    Fetch => "Fetch",
    FetchPrune => "Fetch (Prune)",
    FetchAll => "Fetch From All Remotes",
    MergeBranch => "Merge Branch...",
    OpenAllChanges => "Open All Changes",
    OpenStagedChanges => "Open Staged Changes",
    PopLatestStash => "Pop Latest Stash",
    PopStash => "Pop Stash...",
    PublishBranch => "Publish Branch...",
    Pull => "Pull",
    PullRebase => "Pull (Rebase)",
    PullFrom => "Pull from...",
    Push => "Push",
    PushFollowTags => "Push (Follow Tags)",
    PushTags => "Push Tags",
    PushTo => "Push to...",
    RebaseBranch => "Rebase Branch...",
    Refresh => "Refresh",
    RemoveRemote => "Remove Remote",
    RenameBranch => "Rename Branch...",
    RevertUnsaved => "Revert Unsaved Edits",
    SaveChanges => "Save",
    SaveAll => "Save All",
    StageAll => "Stage All Changes",
    StageAllMerge => "Stage All Merge Changes",
    StageAllTracked => "Stage All Tracked Changes",
    StageAllUntracked => "Stage All Untracked Changes",
    StageChanges => "Stage Changes",
    Stash => "Stash",
    StashIncludeUntracked => "Stash (Include Untracked)",
    StashStaged => "Stash Staged",
    Sync => "Sync",
    SyncRebase => "Sync (Rebase)",
    ToggleBlame => "Toggle Git Blame Status Bar Item",
    UndoLastCommit => "Undo Last Commit",
    UnstageAll => "Unstage All Changes",
    UnstageChanges => "Unstage Changes",
}

/// Which files a commit takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Staged files, or every change when nothing is staged.
    Smart,
    Staged,
    All,
}

impl Workspace {
    pub fn open_command_palette(
        &mut self,
        query: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.repo.is_none() {
            return;
        }
        let t = cx.theme().clone();
        let mut order: Vec<Cmd> = self.recent_commands.clone();
        order.extend(
            Cmd::ALL
                .iter()
                .filter(|c| !self.recent_commands.contains(c)),
        );
        let has_recent = !self.recent_commands.is_empty();
        let items = order
            .iter()
            .enumerate()
            .map(|(i, cmd)| PickerItem {
                haystack: format!("Git: {}", cmd.title()),
                label_start: 0,
                note: String::new(),
                icon: "",
                icon_color: None,
                badge: (i == 0 && has_recent).then(|| ("recently used".to_string(), t.muted)),
                section: has_recent.then_some(if i < self.recent_commands.len() {
                    "Recently used"
                } else {
                    "Other commands"
                }),
            })
            .collect();
        let picker = cx.new(|cx| {
            let mut p = Picker::new("Type a git command", "No matching commands.", items, cx)
                .with_query_prefix(">", cx);
            p.set_query(query.unwrap_or_else(|| ">".into()), cx);
            p
        });
        self.show_picker(
            picker,
            Box::new(move |this, pick, window, cx| {
                if let Pick::Item(ix) = pick {
                    if let Some(&cmd) = order.get(ix) {
                        this.run_command(cmd, window, cx);
                    }
                }
            }),
            window,
            cx,
        );
    }

    pub fn run_command(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        self.recent_commands.retain(|c| *c != cmd);
        self.recent_commands.insert(0, cmd);
        self.recent_commands.truncate(5);
        self.error = None;
        let no_opts = CommitOptions::default();
        let amend = CommitOptions {
            amend: true,
            ..no_opts
        };
        let signoff = CommitOptions {
            signoff: true,
            ..no_opts
        };
        match cmd {
            // ---- commit ----
            Cmd::Commit => self.commit_cmd(Scope::Smart, no_opts, window, cx),
            Cmd::CommitAmend => self.commit_cmd(Scope::Smart, amend, window, cx),
            Cmd::CommitSignedOff => self.commit_cmd(Scope::Smart, signoff, window, cx),
            Cmd::CommitAll => self.commit_cmd(Scope::All, no_opts, window, cx),
            Cmd::CommitAllAmend => self.commit_cmd(Scope::All, amend, window, cx),
            Cmd::CommitAllSignedOff => self.commit_cmd(Scope::All, signoff, window, cx),
            Cmd::CommitStaged => self.commit_cmd(Scope::Staged, no_opts, window, cx),
            Cmd::CommitStagedAmend => self.commit_cmd(Scope::Staged, amend, window, cx),
            Cmd::CommitStagedSignedOff => self.commit_cmd(Scope::Staged, signoff, window, cx),
            Cmd::CommitEmpty => self.commit_cmd(
                Scope::Staged,
                CommitOptions {
                    allow_empty: true,
                    ..no_opts
                },
                window,
                cx,
            ),
            Cmd::UndoLastCommit => {
                let Some(repo) = &self.repo else { return };
                match repo.undo_last_commit() {
                    Ok(message) => {
                        if self.message.read(cx).text().trim().is_empty() {
                            self.message.update(cx, |m, cx| m.set_text(message, cx));
                        }
                        self.flash("Undid last commit", cx);
                    }
                    Err(e) => self.error = Some(format!("{e:#}")),
                }
                self.refresh(cx);
            }

            // ---- stage, unstage, discard ----
            Cmd::StageAll => {
                self.git(&["add", "-A"], "Staged all changes", cx);
            }
            Cmd::StageAllTracked => {
                self.git(&["add", "-u"], "Staged tracked changes", cx);
            }
            Cmd::StageAllUntracked => {
                let paths = self.unstaged_paths(|c| c.kind == ChangeKind::Untracked);
                self.stage_paths(paths, cx);
            }
            Cmd::StageAllMerge => {
                let paths = self.unstaged_paths(|c| c.kind == ChangeKind::Conflicted);
                self.stage_paths(paths, cx);
            }
            Cmd::StageChanges => {
                if let Some((false, path)) = self.active_file() {
                    self.stage(vec![path], cx);
                } else {
                    self.need_file("stage", cx);
                }
            }
            Cmd::UnstageAll => {
                let paths: Vec<String> = self.staged.iter().map(|c| c.path.clone()).collect();
                if paths.is_empty() {
                    self.flash("Nothing is staged", cx);
                } else {
                    self.unstage(paths, cx);
                }
            }
            Cmd::UnstageChanges => {
                if let Some((true, path)) = self.active_file() {
                    self.unstage(vec![path], cx);
                } else {
                    self.need_file("unstage", cx);
                }
            }
            Cmd::DiscardAll => {
                let changes = self.unstaged.clone();
                self.discard_or_note(changes, window, cx);
            }
            Cmd::DiscardAllTracked => {
                let changes = self.unstaged_where(|c| c.kind != ChangeKind::Untracked);
                self.discard_or_note(changes, window, cx);
            }
            Cmd::DiscardAllUntracked => {
                let changes = self.unstaged_where(|c| c.kind == ChangeKind::Untracked);
                self.discard_or_note(changes, window, cx);
            }
            Cmd::DiscardChanges => {
                let change = self
                    .active_file()
                    .filter(|(staged, _)| !staged)
                    .and_then(|(_, path)| self.unstaged.iter().find(|c| c.path == path).cloned());
                match change {
                    Some(c) => self.discard(vec![c], window, cx),
                    None => self.need_file("discard", cx),
                }
            }
            Cmd::AddToGitignore => {
                let items = self.change_items(true, false, cx);
                let paths: Vec<String> = self.unstaged.iter().map(|c| c.path.clone()).collect();
                let picker = cx.new(|cx| {
                    Picker::new("Select a file to ignore, or type a pattern", "", items, cx)
                        .with_create(|q| format!("Add '{q}' to .gitignore"), paths.clone(), cx)
                });
                self.show_picker(
                    picker,
                    Box::new(move |this, pick, _, cx| {
                        let pattern = match pick {
                            Pick::Item(ix) => match paths.get(ix) {
                                Some(p) => format!("/{p}"),
                                None => return,
                            },
                            Pick::Text(text) => text,
                        };
                        if this.run(cx, |r| r.add_to_gitignore(&pattern)) {
                            this.flash("Added to .gitignore", cx);
                        }
                    }),
                    window,
                    cx,
                );
            }

            // ---- tabs ----
            Cmd::OpenAllChanges => self.open_all(false, cx),
            Cmd::OpenStagedChanges => self.open_all(true, cx),
            Cmd::CloseAllDiffs => self.confirm_unsaved(window, cx, |this, _, cx| {
                this.tabs.clear();
                this.active = None;
                cx.notify();
            }),
            Cmd::SaveChanges => {
                if let Some(a) = self.active {
                    self.save_tab(a, window, cx)
                }
            }
            Cmd::SaveAll => {
                while let Some(ix) = self.tabs.iter().position(|t| t.draft.is_some()) {
                    if !self.write_draft(ix, cx) {
                        break;
                    }
                }
            }
            Cmd::RevertUnsaved => {
                if let Some(a) = self.active {
                    self.discard_draft(a, cx)
                }
            }
            Cmd::Refresh => self.refresh(cx),
            Cmd::ToggleBlame => {
                self.show_blame = !self.show_blame;
                cx.notify();
            }

            // ---- branches ----
            Cmd::Checkout => self.checkout_picker(window, cx),
            Cmd::CheckoutDetached => self.pick_ref(
                "Select a branch or tag to check out in detached mode",
                RefFilter {
                    local: true,
                    remote: true,
                    tags: true,
                    head: true,
                    create: false,
                },
                |this, choice, _, cx| {
                    if let Some(name) = ref_name(&choice) {
                        this.git(&["switch", "--detach", &name], "Checked out (detached)", cx);
                    }
                },
                window,
                cx,
            ),
            Cmd::CreateBranch => self.ask_text(
                "Branch name",
                "Enter a name for the new branch and press Enter",
                false,
                None,
                |this, name, _, cx| {
                    this.git(&["switch", "-c", &name], "Created branch", cx);
                },
                window,
                cx,
            ),
            Cmd::CreateBranchFrom => self.pick_ref(
                "Select a ref to create the branch from",
                RefFilter {
                    local: true,
                    remote: true,
                    tags: true,
                    head: true,
                    create: false,
                },
                |this, choice, window, cx| {
                    let Some(from) = ref_name(&choice) else {
                        return;
                    };
                    this.ask_text(
                        "Branch name",
                        format!("Enter a name for the new branch from {from} and press Enter"),
                        false,
                        None,
                        move |this, name, _, cx| {
                            this.git(&["switch", "-c", &name, &from], "Created branch", cx);
                        },
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            Cmd::RenameBranch => {
                let current = self.head.branch.clone();
                self.ask_text(
                    "New branch name",
                    format!("Enter a new name for {current} and press Enter"),
                    false,
                    Some(current),
                    |this, name, _, cx| {
                        this.git(&["branch", "-m", &name], "Renamed branch", cx);
                    },
                    window,
                    cx,
                );
            }
            Cmd::DeleteBranch => self.pick_ref(
                "Select a branch to delete",
                RefFilter {
                    local: true,
                    ..Default::default()
                },
                |this, choice, window, cx| {
                    let Some(name) = ref_name(&choice) else {
                        return;
                    };
                    this.delete_branch(name, window, cx);
                },
                window,
                cx,
            ),
            Cmd::DeleteRemoteBranch => self.pick_ref(
                "Select a remote branch to delete",
                RefFilter {
                    remote: true,
                    ..Default::default()
                },
                |this, choice, window, cx| {
                    let Some(full) = ref_name(&choice) else {
                        return;
                    };
                    let Some((remote, branch)) = full.split_once('/') else {
                        return;
                    };
                    let args = vec![
                        "push".to_string(),
                        remote.to_string(),
                        "--delete".to_string(),
                        branch.to_string(),
                    ];
                    this.confirm(
                        format!("Delete the remote branch {full}?"),
                        "It will be removed from the remote for everyone.",
                        "Delete",
                        move |this, _, cx| {
                            this.run_background(vec![("Deleting…", args)], "Deleted", cx)
                        },
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            Cmd::MergeBranch => self.pick_ref(
                "Select a branch to merge into the current branch",
                RefFilter {
                    local: true,
                    remote: true,
                    tags: true,
                    ..Default::default()
                },
                |this, choice, _, cx| {
                    if let Some(name) = ref_name(&choice) {
                        this.git(&["merge", "--no-edit", &name], "Merged", cx);
                    }
                },
                window,
                cx,
            ),
            Cmd::RebaseBranch => self.pick_ref(
                "Select a branch to rebase the current branch onto",
                RefFilter {
                    local: true,
                    remote: true,
                    ..Default::default()
                },
                |this, choice, _, cx| {
                    if let Some(name) = ref_name(&choice) {
                        this.git(&["rebase", &name], "Rebased", cx);
                    }
                },
                window,
                cx,
            ),
            Cmd::CherryPick => self.ask_text(
                "Commit hash",
                "Enter the commit to cherry-pick and press Enter",
                false,
                None,
                |this, sha, _, cx| {
                    this.git(&["cherry-pick", &sha], "Cherry-picked", cx);
                },
                window,
                cx,
            ),

            // ---- tags ----
            Cmd::CreateTag => self.ask_text(
                "Tag name",
                "Enter a name for the tag and press Enter",
                false,
                None,
                |this, name, window, cx| {
                    this.ask_text(
                        "Message (optional)",
                        "Enter a message to create an annotated tag, or leave it empty",
                        true,
                        None,
                        move |this, message, _, cx| {
                            if message.is_empty() {
                                this.git(&["tag", &name], "Created tag", cx);
                            } else {
                                this.git(&["tag", "-a", &name, "-m", &message], "Created tag", cx);
                            }
                        },
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            Cmd::DeleteTag => {
                self.pick_tag("Select a tag to delete", window, cx, |this, tag, _, cx| {
                    this.git(&["tag", "-d", &tag], "Deleted tag", cx);
                })
            }
            Cmd::DeleteRemoteTag => self.pick_tag(
                "Select a tag to delete from the remote",
                window,
                cx,
                |this, tag, window, cx| {
                    this.pick_remote(window, cx, move |this, remote, _, cx| {
                        let args = vec![
                            "push".to_string(),
                            remote,
                            "--delete".to_string(),
                            format!("refs/tags/{tag}"),
                        ];
                        this.run_background(vec![("Deleting…", args)], "Deleted remote tag", cx);
                    });
                },
            ),

            // ---- remotes ----
            Cmd::AddRemote => self.ask_text(
                "Remote name",
                "Enter a name for the remote, e.g. origin, and press Enter",
                false,
                None,
                |this, name, window, cx| {
                    this.ask_text(
                        "Remote URL",
                        format!("Enter the URL for {name} and press Enter"),
                        false,
                        None,
                        move |this, url, _, cx| {
                            this.git(&["remote", "add", &name, &url], "Added remote", cx);
                        },
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            Cmd::RemoveRemote => self.pick_remote(window, cx, |this, remote, window, cx| {
                let name = remote.clone();
                this.confirm(
                    format!("Remove the remote {remote}?"),
                    "Its remote-tracking branches are removed too.",
                    "Remove",
                    move |this, _, cx| {
                        this.git(&["remote", "remove", &name], "Removed remote", cx);
                    },
                    window,
                    cx,
                );
            }),
            Cmd::Fetch => self.background(&[&["fetch"]], "Fetching…", "Fetched", cx),
            Cmd::FetchPrune => {
                self.background(&[&["fetch", "--prune"]], "Fetching…", "Fetched", cx)
            }
            Cmd::FetchAll => self.background(&[&["fetch", "--all"]], "Fetching…", "Fetched", cx),
            Cmd::Pull => self.remote(RemoteOp::Pull, cx),
            Cmd::PullRebase => self.background(&[&["pull", "--rebase"]], "Pulling…", "Pulled", cx),
            Cmd::PullFrom => self.pick_ref(
                "Select a remote branch to pull from",
                RefFilter {
                    remote: true,
                    ..Default::default()
                },
                |this, choice, _, cx| {
                    let Some(full) = ref_name(&choice) else {
                        return;
                    };
                    if let Some((remote, branch)) = full.split_once('/') {
                        this.background(&[&["pull", remote, branch]], "Pulling…", "Pulled", cx);
                    }
                },
                window,
                cx,
            ),
            Cmd::Push => self.remote(RemoteOp::Push, cx),
            Cmd::PushFollowTags => {
                self.background(&[&["push", "--follow-tags"]], "Pushing…", "Pushed", cx)
            }
            Cmd::PushTags => self.background(&[&["push", "--tags"]], "Pushing…", "Pushed tags", cx),
            Cmd::PushTo => self.pick_remote(window, cx, |this, remote, _, cx| {
                let set_upstream = !this.head.has_upstream;
                let mut args = vec!["push"];
                if set_upstream {
                    args.push("-u");
                }
                args.extend([remote.as_str(), "HEAD"]);
                this.background(&[&args], "Pushing…", "Pushed", cx);
            }),
            Cmd::PublishBranch => {
                if self.head.has_upstream {
                    self.flash("Branch is already published", cx);
                } else {
                    self.pick_remote(window, cx, |this, remote, _, cx| {
                        this.background(
                            &[&["push", "-u", &remote, "HEAD"]],
                            "Publishing…",
                            "Published",
                            cx,
                        );
                    });
                }
            }
            Cmd::Sync => self.remote(RemoteOp::Sync, cx),
            Cmd::SyncRebase => {
                if !self.head.has_upstream {
                    self.error =
                        Some("This branch has no upstream. Use Publish Branch first.".to_string());
                    cx.notify();
                } else {
                    self.run_background(
                        vec![
                            ("Fetching…", strings(&["fetch"])),
                            ("Pulling…", strings(&["pull", "--rebase"])),
                            ("Pushing…", strings(&["push"])),
                        ],
                        "Up to date",
                        cx,
                    );
                }
            }

            // ---- stash ----
            Cmd::Stash => self.stash_cmd(&[], window, cx),
            Cmd::StashIncludeUntracked => self.stash_cmd(&["--include-untracked"], window, cx),
            Cmd::StashStaged => self.stash_cmd(&["--staged"], window, cx),
            Cmd::ApplyLatestStash => {
                self.git(&["stash", "apply"], "Applied stash", cx);
            }
            Cmd::PopLatestStash => {
                self.git(&["stash", "pop"], "Popped stash", cx);
            }
            Cmd::ApplyStash => {
                self.pick_stash("Select a stash to apply", window, cx, |this, s, _, cx| {
                    this.git(&["stash", "apply", &s], "Applied stash", cx);
                })
            }
            Cmd::PopStash => {
                self.pick_stash("Select a stash to pop", window, cx, |this, s, _, cx| {
                    this.git(&["stash", "pop", &s], "Popped stash", cx);
                })
            }
            Cmd::DropStash => self.pick_stash(
                "Select a stash to drop",
                window,
                cx,
                |this, s, window, cx| {
                    let reference = s.clone();
                    this.confirm(
                        format!("Drop {s}?"),
                        "The stashed changes will be lost.",
                        "Drop",
                        move |this, _, cx| {
                            this.git(&["stash", "drop", &reference], "Dropped stash", cx);
                        },
                        window,
                        cx,
                    );
                },
            ),
            Cmd::DropAllStashes => {
                let count = self
                    .repo
                    .as_ref()
                    .and_then(|r| r.stashes().ok())
                    .map_or(0, |s| s.len());
                if count == 0 {
                    self.flash("There are no stashes", cx);
                } else {
                    self.confirm(
                        format!(
                            "Drop all {count} stash{}?",
                            if count == 1 { "" } else { "es" }
                        ),
                        "The stashed changes will be lost.",
                        "Drop All",
                        |this, _, cx| {
                            this.git(&["stash", "clear"], "Dropped all stashes", cx);
                        },
                        window,
                        cx,
                    );
                }
            }
        }
    }

    // ---- helpers ----

    /// Runs a quick git command and refreshes. Shows `done` in the status bar.
    pub(super) fn git(
        &mut self,
        args: &[&str],
        done: &'static str,
        cx: &mut Context<Self>,
    ) -> bool {
        let args = strings(args);
        let ok = self.run(cx, |r| Repo::run(r.root(), &args).map(drop));
        if ok {
            self.flash(done, cx);
        }
        ok
    }

    /// Runs one network command in the background with a spinner.
    fn background(
        &mut self,
        args: &[&[&str]],
        label: &'static str,
        done: &'static str,
        cx: &mut Context<Self>,
    ) {
        let steps = args.iter().map(|a| (label, strings(a))).collect();
        self.run_background(steps, done, cx);
    }

    /// Runs git commands one after another in the background. The status bar
    /// spins and shows each step's label.
    pub(super) fn run_background(
        &mut self,
        steps: Vec<(&'static str, Vec<String>)>,
        done: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() || self.repo.is_none() {
            return;
        }
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let mut result = Ok(());
            for (label, args) in steps {
                if this
                    .update(cx, |this, cx| {
                        this.busy = Some(label.to_string());
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                let root = root.clone();
                result = cx
                    .background_spawn(async move { Repo::run(&root, &args).map(drop) })
                    .await;
                if result.is_err() {
                    break;
                }
            }
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => this.flash(done, cx),
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Asks before a step that can't be undone.
    fn confirm(
        &mut self,
        message: String,
        detail: &'static str,
        button: &'static str,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(detail),
            &[button, "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| then(this, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn commit_cmd(
        &mut self,
        scope: Scope,
        options: CommitOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        if self.has_unsaved() {
            self.error = Some(super::workspace::UNSAVED_BEFORE_COMMIT.into());
            cx.notify();
            return;
        }
        let scope = match scope {
            Scope::Smart if self.staged.is_empty() => Scope::All,
            Scope::Smart => Scope::Staged,
            other => other,
        };
        let nothing = match scope {
            Scope::Staged => self.staged.is_empty(),
            _ => self.staged.is_empty() && self.unstaged.is_empty(),
        };
        if nothing && !options.amend && !options.allow_empty {
            self.error = Some(if self.unstaged.is_empty() {
                "There are no changes to commit.".into()
            } else {
                "No staged changes. Stage files with +, or use Git: Commit All.".into()
            });
            cx.notify();
            return;
        }
        let message = self.message.read(cx).text().to_string();
        if message.trim().is_empty() && !options.amend {
            self.ask_text(
                "Commit message",
                "Enter a commit message and press Enter",
                false,
                None,
                move |this, message, window, cx| {
                    this.message.update(cx, |m, cx| m.set_text(message, cx));
                    this.commit_cmd(scope, options, window, cx);
                },
                window,
                cx,
            );
            return;
        }
        let all_paths: Vec<String> = self.unstaged.iter().map(|c| c.path.clone()).collect();
        let ok = self.run(cx, |r| {
            if scope == Scope::All && !all_paths.is_empty() {
                r.stage(&all_paths)?;
            }
            r.commit_with(&message, options)
        });
        if ok {
            self.message.update(cx, |m, cx| m.set_text("", cx));
            self.flash("Committed", cx);
        }
    }

    fn stash_cmd(
        &mut self,
        flags: &'static [&'static str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_text(
            "Stash message (optional)",
            "Enter a message for the stash, or leave it empty, and press Enter",
            true,
            None,
            move |this, message, _, cx| {
                let mut args = vec!["stash", "push"];
                args.extend_from_slice(flags);
                if !message.is_empty() {
                    args.extend(["-m", message.as_str()]);
                }
                this.git(&args, "Stashed", cx);
            },
            window,
            cx,
        );
    }

    fn delete_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else { return };
        match Repo::run(repo.root(), &strings(&["branch", "-d", &name])) {
            Ok(_) => {
                self.flash("Deleted branch", cx);
                self.refresh(cx);
            }
            Err(e) if format!("{e:#}").contains("not fully merged") => {
                let branch = name.clone();
                self.confirm(
                    format!("The branch {name} is not fully merged. Delete it anyway?"),
                    "Its commits that aren't on another branch will be lost.",
                    "Delete",
                    move |this, _, cx| {
                        this.git(&["branch", "-D", &branch], "Deleted branch", cx);
                    },
                    window,
                    cx,
                );
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
            }
        }
    }

    fn pick_remote(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(repo) = &self.repo else { return };
        let remotes = repo.remotes().unwrap_or_default();
        match remotes.as_slice() {
            [] => {
                self.error =
                    Some("This repository has no remotes. Use Git: Add Remote… first.".into());
                cx.notify();
            }
            [only] => then(self, only.clone(), window, cx),
            _ => {
                let t = cx.theme().clone();
                let items = remotes
                    .iter()
                    .map(|r| PickerItem {
                        haystack: r.clone(),
                        label_start: 0,
                        note: String::new(),
                        icon: "cloud",
                        icon_color: Some(t.syn_type),
                        badge: None,
                        section: None,
                    })
                    .collect();
                self.pick_list(
                    "Select a remote",
                    "No remotes match.",
                    items,
                    move |this, ix, window, cx| {
                        if let Some(r) = remotes.get(ix) {
                            then(this, r.clone(), window, cx)
                        }
                    },
                    window,
                    cx,
                );
            }
        }
    }

    fn pick_tag(
        &mut self,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.pick_ref(
            placeholder,
            RefFilter {
                tags: true,
                ..Default::default()
            },
            |this, choice, window, cx| {
                if let RefChoice::Tag(t) = choice {
                    then(this, t.name, window, cx)
                }
            },
            window,
            cx,
        );
    }

    fn pick_stash(
        &mut self,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(repo) = &self.repo else { return };
        let stashes = match repo.stashes() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
                return;
            }
        };
        if stashes.is_empty() {
            self.flash("There are no stashes", cx);
            return;
        }
        let t = cx.theme().clone();
        let items = stashes
            .iter()
            .map(|s| PickerItem {
                haystack: s.message.clone(),
                label_start: 0,
                note: format!(
                    "{} · {}",
                    s.reference,
                    crate::git::relative_time_since(s.time)
                ),
                icon: "stash",
                icon_color: Some(t.accent),
                badge: None,
                section: None,
            })
            .collect();
        self.pick_list(
            placeholder,
            "No stashes match.",
            items,
            move |this, ix, window, cx| {
                if let Some(s) = stashes.get(ix) {
                    then(this, s.reference.clone(), window, cx)
                }
            },
            window,
            cx,
        );
    }

    /// The (staged, path) of the file in the active tab.
    fn active_file(&self) -> Option<(bool, String)> {
        self.active_tab().map(|t| (t.staged, t.path.clone()))
    }

    fn need_file(&mut self, verb: &str, cx: &mut Context<Self>) {
        self.error = Some(format!("Open a file's changes first to {verb} it."));
        cx.notify();
    }

    fn unstaged_where(&self, f: impl Fn(&FileChange) -> bool) -> Vec<FileChange> {
        self.unstaged.iter().filter(|c| f(c)).cloned().collect()
    }

    fn unstaged_paths(&self, f: impl Fn(&FileChange) -> bool) -> Vec<String> {
        self.unstaged_where(f).into_iter().map(|c| c.path).collect()
    }

    fn stage_paths(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            self.flash("Nothing to stage", cx);
        } else {
            self.stage(paths, cx);
        }
    }

    fn discard_or_note(
        &mut self,
        changes: Vec<FileChange>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if changes.is_empty() {
            self.flash("Nothing to discard", cx);
        } else {
            self.discard(changes, window, cx);
        }
    }

    fn open_all(&mut self, staged: bool, cx: &mut Context<Self>) {
        let paths: Vec<String> = self.list(staged).iter().map(|c| c.path.clone()).collect();
        for path in paths {
            self.open(staged, path, true, cx);
        }
    }
}

fn ref_name(choice: &RefChoice) -> Option<String> {
    match choice {
        RefChoice::Branch(b) => Some(b.name.clone()),
        RefChoice::Tag(t) => Some(t.name.clone()),
        RefChoice::New(_) => None,
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

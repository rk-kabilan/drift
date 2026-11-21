use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context as _, Result};
use git2::{Repository, Status, StatusOptions};

/// The single-letter state shown next to each file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl ChangeKind {
    pub fn letter(self) -> &'static str {
        match self {
            ChangeKind::Modified => "M",
            ChangeKind::Added => "A",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Untracked => "U",
            ChangeKind::Conflicted => "C",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    pub kind: ChangeKind,
    /// True for an entry in "Staged Changes", false for "Changes".
    pub staged: bool,
}

impl FileChange {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn dir(&self) -> &str {
        match self.path.rfind('/') {
            Some(ix) => &self.path[..ix],
            None => "",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HeadInfo {
    pub repo_name: String,
    /// Branch name, or a short hash when HEAD is detached.
    pub branch: String,
    pub short_sha: Option<String>,
    pub dirty: bool,
    pub ahead: usize,
    pub behind: usize,
    pub has_upstream: bool,
}

/// A local or remote-tracking branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    /// `main` for a local branch, `origin/main` for a remote one.
    pub name: String,
    pub remote: bool,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub short_sha: String,
    pub summary: String,
    pub time: i64,
}

/// Extra flags for a commit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitOptions {
    pub amend: bool,
    /// Adds a `Signed-off-by` trailer.
    pub signoff: bool,
    /// Allows a commit with no changes.
    pub allow_empty: bool,
}

/// A tag, newest first in [`Repo::tags`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagInfo {
    pub name: String,
    pub short_sha: String,
    pub summary: String,
    pub time: i64,
}

/// An entry of `git stash list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashInfo {
    /// `stash@{0}`.
    pub reference: String,
    pub message: String,
    pub time: i64,
}

pub struct Repo {
    repo: Repository,
    root: PathBuf,
}

impl Repo {
    /// Opens the repository that contains `path` (searching parent folders).
    pub fn discover(path: &Path) -> Result<Self> {
        let repo = Repository::discover(path)
            .with_context(|| format!("no git repository found at {}", path.display()))?;
        let root = repo
            .workdir()
            .ok_or_else(|| anyhow!("bare repositories are not supported"))?
            .to_path_buf();
        Ok(Self { repo, root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn abs_path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Staged and unstaged changes, each list sorted by path.
    pub fn status(&self) -> Result<(Vec<FileChange>, Vec<FileChange>)> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .renames_head_to_index(true);
        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        for entry in statuses.iter() {
            let Some(path) = entry.path() else { continue };
            let path = path.to_string();
            let s = entry.status();
            if s.contains(Status::CONFLICTED) {
                unstaged.push(FileChange {
                    path,
                    kind: ChangeKind::Conflicted,
                    staged: false,
                });
                continue;
            }
            let index_kind = if s.contains(Status::INDEX_NEW) {
                Some(ChangeKind::Added)
            } else if s.contains(Status::INDEX_DELETED) {
                Some(ChangeKind::Deleted)
            } else if s.contains(Status::INDEX_RENAMED) {
                Some(ChangeKind::Renamed)
            } else if s.intersects(Status::INDEX_MODIFIED | Status::INDEX_TYPECHANGE) {
                Some(ChangeKind::Modified)
            } else {
                None
            };
            let wt_kind = if s.contains(Status::WT_NEW) {
                Some(ChangeKind::Untracked)
            } else if s.contains(Status::WT_DELETED) {
                Some(ChangeKind::Deleted)
            } else if s.contains(Status::WT_RENAMED) {
                Some(ChangeKind::Renamed)
            } else if s.intersects(Status::WT_MODIFIED | Status::WT_TYPECHANGE) {
                Some(ChangeKind::Modified)
            } else {
                None
            };
            if let Some(kind) = index_kind {
                staged.push(FileChange {
                    path: path.clone(),
                    kind,
                    staged: true,
                });
            }
            if let Some(kind) = wt_kind {
                unstaged.push(FileChange {
                    path,
                    kind,
                    staged: false,
                });
            }
        }
        staged.sort_by(|a, b| a.path.cmp(&b.path));
        unstaged.sort_by(|a, b| a.path.cmp(&b.path));
        Ok((staged, unstaged))
    }

    pub fn head_info(&self, dirty: bool) -> HeadInfo {
        let repo_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".into());
        let mut info = HeadInfo {
            repo_name,
            dirty,
            ..Default::default()
        };
        match self.repo.head() {
            Ok(head) => {
                let oid = head.target();
                info.short_sha = oid.map(|o| short(&o));
                if head.is_branch() {
                    info.branch = head.shorthand().unwrap_or("HEAD").to_string();
                    if let Ok(branch) = self.repo.find_branch(&info.branch, git2::BranchType::Local)
                    {
                        if let Ok(upstream) = branch.upstream() {
                            info.has_upstream = true;
                            if let (Some(local), Some(remote)) = (oid, upstream.get().target()) {
                                if let Ok((a, b)) = self.repo.graph_ahead_behind(local, remote) {
                                    info.ahead = a;
                                    info.behind = b;
                                }
                            }
                        }
                    }
                } else {
                    info.branch = info.short_sha.clone().unwrap_or_else(|| "HEAD".into());
                }
            }
            Err(_) => {
                // Unborn branch: a fresh repository with no commits yet.
                info.branch = self
                    .repo
                    .find_reference("HEAD")
                    .ok()
                    .and_then(|r| {
                        r.symbolic_target()
                            .map(|t| t.trim_start_matches("refs/heads/").to_string())
                    })
                    .unwrap_or_else(|| "HEAD".into());
            }
        }
        info
    }

    /// Text of `path` at HEAD, or `None` when it does not exist there.
    pub fn head_text(&self, path: &str) -> Option<String> {
        let tree = self.repo.head().ok()?.peel_to_tree().ok()?;
        let entry = tree.get_path(Path::new(path)).ok()?;
        let blob = self.repo.find_blob(entry.id()).ok()?;
        Some(decode(blob.content()))
    }

    /// Text of `path` in the index, or `None` when it is not staged.
    pub fn index_text(&self, path: &str) -> Option<String> {
        let index = self.repo.index().ok()?;
        let entry = index.get_path(Path::new(path), 0)?;
        let blob = self.repo.find_blob(entry.id).ok()?;
        Some(decode(blob.content()))
    }

    /// Text of `path` in the working tree, or `None` when it is missing.
    pub fn worktree_text(&self, path: &str) -> Option<String> {
        std::fs::read(self.abs_path(path)).ok().map(|b| decode(&b))
    }

    /// Before/after texts for a change, plus labels for the two diff columns.
    pub fn change_texts(&self, change: &FileChange) -> (String, String, String, String) {
        let head_label = match self.repo.head().ok().and_then(|h| h.target()) {
            Some(oid) => format!("HEAD {}", short(&oid)),
            None => "HEAD".to_string(),
        };
        if change.staged {
            let old = self.head_text(&change.path).unwrap_or_default();
            let new = self.index_text(&change.path).unwrap_or_default();
            (old, new, head_label, "Index".into())
        } else {
            let index = self.index_text(&change.path);
            let head = self.head_text(&change.path);
            let left_label = if index.is_some() && index != head {
                "Index".to_string()
            } else {
                head_label
            };
            let old = index.unwrap_or_default();
            let new = self.worktree_text(&change.path).unwrap_or_default();
            (old, new, left_label, "Working Tree".into())
        }
    }

    pub fn blame(&self, path: &str, contents: &str) -> Result<super::Blame> {
        super::blame::blame(&self.repo, path, contents)
    }

    // ---- writes, through the git command ----

    pub fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["add", "-A", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git(&args, None).map(drop)
    }

    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.repo.head().is_ok() {
            let mut args = vec!["reset", "-q", "HEAD", "--"];
            args.extend(paths.iter().map(String::as_str));
            self.git(&args, None).map(drop)
        } else {
            let mut args = vec!["rm", "-q", "--cached", "-r", "--"];
            args.extend(paths.iter().map(String::as_str));
            self.git(&args, None).map(drop)
        }
    }

    /// Throws away working-tree changes. Untracked files are deleted.
    pub fn discard(&self, changes: &[FileChange]) -> Result<()> {
        let mut tracked = Vec::new();
        for change in changes.iter().filter(|c| !c.staged) {
            if change.kind == ChangeKind::Untracked {
                let path = self.abs_path(&change.path);
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)?;
                } else {
                    std::fs::remove_file(&path)?;
                }
            } else {
                tracked.push(change.path.as_str());
            }
        }
        if !tracked.is_empty() {
            let mut args = vec!["checkout", "--"];
            args.extend(tracked);
            self.git(&args, None)?;
        }
        Ok(())
    }

    /// Commits the index. When amending with an empty message, the previous
    /// commit's message is kept.
    pub fn commit(&self, message: &str, amend: bool) -> Result<()> {
        self.commit_with(
            message,
            CommitOptions {
                amend,
                ..Default::default()
            },
        )
    }

    pub fn commit_with(&self, message: &str, options: CommitOptions) -> Result<()> {
        let amend = options.amend;
        let kept;
        let message = if amend && message.trim().is_empty() {
            kept = self.last_commit_message().unwrap_or_default();
            kept.as_str()
        } else {
            message
        };
        if message.trim().is_empty() {
            bail!("Enter a commit message first.");
        }
        let mut args = vec!["commit", "-q", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        if options.signoff {
            args.push("--signoff");
        }
        if options.allow_empty {
            args.push("--allow-empty");
        }
        self.git(&args, Some(message)).map(drop)
    }

    /// Runs `git` with `args` in `root` and returns its stdout. For commands
    /// with no dedicated method, and for running in the background.
    pub fn run(root: &Path, args: &[String]) -> Result<String> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run_git(root, &args, None)
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self
            .git(&["remote"], None)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        let out = self.git(
            &[
                "for-each-ref",
                "refs/tags",
                "--sort=-creatordate",
                "--format=%(refname:short)%00%(objectname:short)%00%(subject)%00%(creatordate:unix)",
            ],
            None,
        )?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\0');
                Some(TagInfo {
                    name: f.next()?.to_string(),
                    short_sha: f.next().unwrap_or_default().to_string(),
                    summary: f.next().unwrap_or_default().to_string(),
                    time: f.next().and_then(|t| t.parse().ok()).unwrap_or(0),
                })
            })
            .filter(|t| !t.name.is_empty())
            .collect())
    }

    pub fn stashes(&self) -> Result<Vec<StashInfo>> {
        let out = self.git(&["stash", "list", "--format=%gd%x00%gs%x00%ct"], None)?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\0');
                Some(StashInfo {
                    reference: f.next()?.to_string(),
                    message: f.next().unwrap_or_default().to_string(),
                    time: f.next().and_then(|t| t.parse().ok()).unwrap_or(0),
                })
            })
            .collect())
    }

    /// Checks out `name`, creating it from `start` (or HEAD) when no local
    /// branch has that name. A remote branch of the same name is tracked.
    pub fn checkout_or_create(&self, name: &str, start: Option<&str>) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Enter a branch name.");
        }
        if self.repo.find_branch(name, git2::BranchType::Local).is_ok() {
            return self.git(&["switch", name], None).map(drop);
        }
        self.git(&["check-ref-format", "--branch", name], None)
            .map_err(|_| anyhow!("'{name}' is not a valid branch name."))?;
        match start {
            Some(start) => self.git(&["switch", "-c", name, start], None).map(drop),
            // `git switch <name>` tracks a remote branch of that name when there is one.
            None if self.remote_branch_named(name) => self.git(&["switch", name], None).map(drop),
            None => self.git(&["switch", "-c", name], None).map(drop),
        }
    }

    fn remote_branch_named(&self, name: &str) -> bool {
        self.remotes().unwrap_or_default().iter().any(|r| {
            self.repo
                .find_branch(&format!("{r}/{name}"), git2::BranchType::Remote)
                .is_ok()
        })
    }

    /// Undoes the last commit, keeping its changes staged. Returns its message.
    pub fn undo_last_commit(&self) -> Result<String> {
        let head = self
            .repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .ok_or_else(|| anyhow!("There is no commit to undo."))?;
        let message = head.message().unwrap_or_default().trim_end().to_string();
        if head.parent_count() == 0 {
            self.git(&["update-ref", "-d", "HEAD"], None)?;
        } else {
            self.git(&["reset", "--soft", "HEAD~1"], None)?;
        }
        Ok(message)
    }

    /// Appends `pattern` to the root .gitignore, on its own line.
    pub fn add_to_gitignore(&self, pattern: &str) -> Result<()> {
        let path = self.root.join(".gitignore");
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.lines().any(|l| l.trim() == pattern) {
            return Ok(());
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(pattern);
        text.push('\n');
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Pushes the current branch. A branch without an upstream is published to
    /// `origin` (or the only remote) and set to track it.
    pub fn push(root: &Path) -> Result<()> {
        let has_upstream = run_git(
            root,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            None,
        )
        .is_ok();
        if has_upstream {
            return run_git(root, &["push"], None).map(drop);
        }
        let remotes = run_git(root, &["remote"], None)?;
        let remotes: Vec<&str> = remotes
            .lines()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .collect();
        let remote = if remotes.contains(&"origin") {
            "origin"
        } else {
            match remotes.as_slice() {
                [only] => only,
                [] => bail!("This repository has no remote to push to."),
                _ => bail!("No upstream branch, and more than one remote. Set one with `git push -u <remote> <branch>`."),
            }
        };
        run_git(root, &["push", "-u", remote, "HEAD"], None).map(drop)
    }

    pub fn pull(root: &Path) -> Result<()> {
        run_git(root, &["pull", "--ff-only"], None).map(drop)
    }

    pub fn fetch(root: &Path) -> Result<()> {
        run_git(root, &["fetch", "--all", "--prune"], None).map(drop)
    }

    /// Local and remote-tracking branches, local first, each sorted by name.
    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let mut out = Vec::new();
        for item in self.repo.branches(None)? {
            let (branch, kind) = item?;
            let Some(name) = branch.name()?.map(str::to_string) else {
                continue;
            };
            let remote = kind == git2::BranchType::Remote;
            if remote && name.ends_with("/HEAD") {
                continue;
            }
            let commit = branch.get().peel_to_commit().ok();
            out.push(BranchInfo {
                is_head: !remote && branch.is_head(),
                upstream: if remote {
                    None
                } else {
                    branch
                        .upstream()
                        .ok()
                        .and_then(|u| u.name().ok().flatten().map(str::to_string))
                },
                short_sha: commit.as_ref().map(|c| short(&c.id())).unwrap_or_default(),
                summary: commit
                    .as_ref()
                    .and_then(|c| c.summary().map(str::to_string))
                    .unwrap_or_default(),
                time: commit.as_ref().map(|c| c.time().seconds()).unwrap_or(0),
                name,
                remote,
            });
        }
        out.sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// Checks out a local branch, or creates a tracking branch for a remote one
    /// (reusing a local branch of the same name if it exists).
    pub fn switch_branch(&self, branch: &BranchInfo) -> Result<()> {
        if !branch.remote {
            return self.git(&["switch", &branch.name], None).map(drop);
        }
        let local = branch
            .name
            .split_once('/')
            .map_or(branch.name.as_str(), |(_, rest)| rest);
        if self
            .repo
            .find_branch(local, git2::BranchType::Local)
            .is_ok()
        {
            self.git(&["switch", local], None).map(drop)
        } else {
            self.git(&["switch", "--track", &branch.name], None)
                .map(drop)
        }
    }

    /// Raw bytes of `path` in the working tree, for line-ending detection.
    fn worktree_raw(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.abs_path(path)).ok()
    }

    fn index_raw(&self, path: &str) -> Option<Vec<u8>> {
        let index = self.repo.index().ok()?;
        let entry = index.get_path(Path::new(path), 0)?;
        Some(self.repo.find_blob(entry.id).ok()?.content().to_vec())
    }

    /// Replaces the working-tree file with `text`, keeping its line endings.
    pub fn write_worktree(&self, path: &str, text: &str) -> Result<()> {
        let crlf = self.worktree_raw(path).is_some_and(|b| has_crlf(&b));
        let abs = self.abs_path(path);
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(abs, with_eol(text, crlf))?;
        Ok(())
    }

    /// Replaces the staged version of `path` with `text`, keeping its line endings
    /// and file mode. Adds the path to the index when it is not there yet.
    pub fn write_index(&self, path: &str, text: &str) -> Result<()> {
        let crlf = self
            .index_raw(path)
            .or_else(|| self.worktree_raw(path))
            .is_some_and(|b| has_crlf(&b));
        let data = with_eol(text, crlf);
        let mut index = self.repo.index()?;
        let entry = match index.get_path(Path::new(path), 0) {
            Some(e) => e,
            None => git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: 0,
                id: git2::Oid::zero(),
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            },
        };
        index.add_frombuffer(&entry, data.as_bytes())?;
        index.write()?;
        Ok(())
    }

    pub fn last_commit_message(&self) -> Option<String> {
        let commit = self.repo.head().ok()?.peel_to_commit().ok()?;
        commit.message().map(|m| m.trim_end().to_string())
    }

    fn git(&self, args: &[&str], stdin: Option<&str>) -> Result<String> {
        run_git(&self.root, args, stdin)
    }
}

fn run_git(root: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .context("could not run `git`; is it installed and on PATH?")?;
    if let Some(input) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(input.as_bytes())?;
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        bail!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            msg
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn has_crlf(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == b"\r\n")
}

fn with_eol(text: &str, crlf: bool) -> String {
    if crlf {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

pub(crate) fn short(oid: &git2::Oid) -> String {
    oid.to_string()[..7].to_string()
}

fn decode(bytes: &[u8]) -> String {
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return "(binary file)".to_string();
    }
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway repository with one commit containing `a.txt` and `b.txt`.
    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("drift-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let f = Self { dir };
            f.git(&["init", "-q", "-b", "main"]);
            f.git(&["config", "user.name", "Test Author"]);
            f.git(&["config", "user.email", "test@example.com"]);
            f.git(&["config", "commit.gpgsign", "false"]);
            f.write("a.txt", "one\ntwo\nthree\n");
            f.write("b.txt", "bee\n");
            f.git(&["add", "-A"]);
            f.git(&["commit", "-q", "-m", "Initial commit"]);
            f
        }

        fn git(&self, args: &[&str]) -> String {
            run_git(&self.dir, args, None).unwrap()
        }

        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.dir.join(path), text).unwrap();
        }

        fn repo(&self) -> Repo {
            Repo::discover(&self.dir).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn summary(list: &[FileChange]) -> Vec<(String, &'static str)> {
        list.iter()
            .map(|c| (c.path.clone(), c.kind.letter()))
            .collect()
    }

    #[test]
    fn status_groups_staged_and_unstaged() {
        let f = Fixture::new("status");
        f.write("a.txt", "one\n2\nthree\n");
        std::fs::remove_file(f.dir.join("b.txt")).unwrap();
        f.write("new.txt", "fresh\n");
        f.write("staged.txt", "s\n");
        f.git(&["add", "staged.txt"]);
        let (staged, unstaged) = f.repo().status().unwrap();
        assert_eq!(summary(&staged), [("staged.txt".to_string(), "A")]);
        assert_eq!(
            summary(&unstaged),
            [
                ("a.txt".to_string(), "M"),
                ("b.txt".to_string(), "D"),
                ("new.txt".to_string(), "U")
            ]
        );
    }

    #[test]
    fn stage_unstage_and_commit() {
        let f = Fixture::new("commit");
        f.write("a.txt", "one\n2\nthree\n");
        f.write("new.txt", "fresh\n");
        let repo = f.repo();
        repo.stage(&["a.txt".into(), "new.txt".into()]).unwrap();
        let (staged, unstaged) = repo.status().unwrap();
        assert_eq!(staged.len(), 2);
        assert!(unstaged.is_empty());

        repo.unstage(&["new.txt".into()]).unwrap();
        let (staged, unstaged) = repo.status().unwrap();
        assert_eq!(summary(&staged), [("a.txt".to_string(), "M")]);
        assert_eq!(summary(&unstaged), [("new.txt".to_string(), "U")]);

        assert!(repo.commit("   ", false).is_err());
        repo.commit("Change a\n\nWith a body.", false).unwrap();
        assert_eq!(
            repo.last_commit_message().as_deref(),
            Some("Change a\n\nWith a body.")
        );
        let (staged, _) = repo.status().unwrap();
        assert!(staged.is_empty());

        // Amending with an empty message keeps the previous one.
        repo.stage(&["new.txt".into()]).unwrap();
        repo.commit("", true).unwrap();
        assert_eq!(
            repo.last_commit_message().as_deref(),
            Some("Change a\n\nWith a body.")
        );
        assert_eq!(f.git(&["rev-list", "--count", "HEAD"]).trim(), "2");
    }

    #[test]
    fn discard_restores_tracked_and_deletes_untracked() {
        let f = Fixture::new("discard");
        f.write("a.txt", "changed\n");
        f.write("new.txt", "fresh\n");
        let repo = f.repo();
        let (_, unstaged) = repo.status().unwrap();
        repo.discard(&unstaged).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.dir.join("a.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );
        assert!(!f.dir.join("new.txt").exists());
        let (staged, unstaged) = repo.status().unwrap();
        assert!(staged.is_empty() && unstaged.is_empty());
    }

    #[test]
    fn change_texts_compare_the_right_versions() {
        let f = Fixture::new("texts");
        f.write("a.txt", "one\nSTAGED\nthree\n");
        f.git(&["add", "a.txt"]);
        f.write("a.txt", "one\nSTAGED\nthree\nworktree\n");
        let repo = f.repo();
        let (staged, unstaged) = repo.status().unwrap();
        let (old, new, left, right) = repo.change_texts(&staged[0]);
        assert_eq!(
            (old.as_str(), new.as_str()),
            ("one\ntwo\nthree\n", "one\nSTAGED\nthree\n")
        );
        assert!(left.starts_with("HEAD "));
        assert_eq!(right, "Index");
        let (old, new, left, right) = repo.change_texts(&unstaged[0]);
        assert_eq!(
            (old.as_str(), new.as_str()),
            ("one\nSTAGED\nthree\n", "one\nSTAGED\nthree\nworktree\n")
        );
        assert_eq!((left.as_str(), right.as_str()), ("Index", "Working Tree"));
    }

    #[test]
    fn blame_marks_uncommitted_lines() {
        let f = Fixture::new("blame");
        let repo = f.repo();
        let text = "one\nNEW\nthree\n";
        f.write("a.txt", text);
        let blame = repo.blame("a.txt", text).unwrap();
        let first = blame.line(1).expect("line 1 is committed");
        assert_eq!(first.author, "Test Author");
        assert_eq!(first.summary, "Initial commit");
        assert!(blame.line(2).is_none());
        assert!(blame.line(3).is_some());
    }

    #[test]
    fn writes_blocks_to_index_and_worktree() {
        let f = Fixture::new("write");
        f.write("a.txt", "one\nTWO\nthree\nfour\n");
        let repo = f.repo();
        // Stage only the first edit: index gets TWO, working tree keeps both edits.
        repo.write_index("a.txt", "one\nTWO\nthree\n").unwrap();
        assert_eq!(repo.index_text("a.txt").unwrap(), "one\nTWO\nthree\n");
        let (staged, unstaged) = repo.status().unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(unstaged.len(), 1);
        repo.write_worktree("a.txt", "one\nTWO\nthree\n").unwrap();
        let (_, unstaged) = repo.status().unwrap();
        assert!(unstaged.is_empty());
        // A brand-new path can be staged from a buffer.
        repo.write_index("new.txt", "hello\n").unwrap();
        assert_eq!(repo.index_text("new.txt").unwrap(), "hello\n");
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let f = Fixture::new("crlf");
        f.write("w.txt", "a\r\nb\r\n");
        let repo = f.repo();
        repo.write_worktree("w.txt", "a\nB\n").unwrap();
        assert_eq!(std::fs::read(f.dir.join("w.txt")).unwrap(), b"a\r\nB\r\n");
    }

    #[test]
    fn lists_and_switches_branches() {
        let f = Fixture::new("branches");
        let remote = f.dir.join("remote.git");
        f.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
        f.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.git(&["push", "-q", "-u", "origin", "main"]);
        f.git(&["branch", "feature"]);
        f.git(&["push", "-q", "origin", "feature:topic"]);
        f.git(&["fetch", "-q"]);
        let repo = f.repo();
        let branches = repo.branches().unwrap();
        let names: Vec<(&str, bool, bool)> = branches
            .iter()
            .map(|b| (b.name.as_str(), b.remote, b.is_head))
            .collect();
        assert_eq!(
            names,
            [
                ("feature", false, false),
                ("main", false, true),
                ("origin/main", true, false),
                ("origin/topic", true, false)
            ]
        );
        assert_eq!(branches[1].upstream.as_deref(), Some("origin/main"));
        let topic = branches.iter().find(|b| b.name == "origin/topic").unwrap();
        repo.switch_branch(topic).unwrap();
        assert_eq!(repo.head_info(false).branch, "topic");
    }

    #[test]
    fn checkout_creates_a_missing_branch() {
        let f = Fixture::new("checkout-create");
        let remote = f.dir.join("remote.git");
        f.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
        f.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.git(&["push", "-q", "origin", "main:topic"]);
        f.git(&["fetch", "-q"]);
        let repo = f.repo();
        // A new name creates a local branch from HEAD.
        repo.checkout_or_create("feature/new", None).unwrap();
        assert_eq!(repo.head_info(false).branch, "feature/new");
        // An existing local branch is checked out, not recreated.
        repo.checkout_or_create("main", None).unwrap();
        assert_eq!(repo.head_info(false).branch, "main");
        // A name that exists only on the remote tracks it.
        repo.checkout_or_create("topic", None).unwrap();
        let info = repo.head_info(false);
        assert_eq!(info.branch, "topic");
        assert!(info.has_upstream);
        assert!(repo.checkout_or_create("bad name..", None).is_err());
    }

    #[test]
    fn lists_tags_and_stashes_and_undoes_commits() {
        let f = Fixture::new("tags-stashes");
        let repo = f.repo();
        f.git(&["tag", "-a", "v1.0", "-m", "First release"]);
        let tags = repo.tags().unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "v1.0");
        assert_eq!(tags[0].summary, "First release");

        f.write("a.txt", "stashed\n");
        f.git(&["stash", "push", "-q", "-m", "wip on a"]);
        let stashes = repo.stashes().unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].reference, "stash@{0}");
        assert!(stashes[0].message.contains("wip on a"));

        f.write("a.txt", "second\n");
        repo.stage(&["a.txt".into()]).unwrap();
        repo.commit_with(
            "Second",
            CommitOptions {
                signoff: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(repo
            .last_commit_message()
            .unwrap()
            .contains("Signed-off-by:"));
        let message = repo.undo_last_commit().unwrap();
        assert!(message.starts_with("Second"));
        let (staged, _) = repo.status().unwrap();
        assert_eq!(staged.len(), 1);

        repo.add_to_gitignore("/build").unwrap();
        repo.add_to_gitignore("/build").unwrap();
        let ignore = std::fs::read_to_string(f.dir.join(".gitignore")).unwrap();
        assert_eq!(ignore.matches("/build").count(), 1);
    }

    #[test]
    fn push_publishes_a_branch_without_upstream() {
        let f = Fixture::new("publish");
        let remote = f.dir.join("remote.git");
        f.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
        f.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        Repo::push(&f.dir).unwrap();
        let info = f.repo().head_info(false);
        assert!(info.has_upstream);
        Repo::fetch(&f.dir).unwrap();
    }

    #[test]
    fn head_info_reports_branch_and_ahead_behind() {
        let f = Fixture::new("head");
        let remote = f.dir.join("remote.git");
        f.git(&["init", "-q", "--bare", remote.to_str().unwrap()]);
        f.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
        f.git(&["push", "-q", "-u", "origin", "main"]);
        f.write("b.txt", "bee 2\n");
        f.git(&["commit", "-qam", "Local change"]);
        let info = f.repo().head_info(false);
        assert_eq!(info.branch, "main");
        assert!(info.has_upstream);
        assert_eq!((info.ahead, info.behind), (1, 0));
    }
}

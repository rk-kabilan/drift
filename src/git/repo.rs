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
        self.git(&args, Some(message)).map(drop)
    }

    pub fn push(root: &Path) -> Result<()> {
        run_git(root, &["push"], None).map(drop)
    }

    pub fn pull(root: &Path) -> Result<()> {
        run_git(root, &["pull", "--ff-only"], None).map(drop)
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

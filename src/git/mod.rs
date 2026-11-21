//! Git access. Reads (status, contents, blame, ahead/behind) go through libgit2;
//! writes (stage, unstage, discard, commit, push) go through the `git` command so
//! hooks, signing and credential helpers behave exactly as on the command line.

mod blame;
mod repo;

pub use blame::{relative_time_since, Blame};
pub use repo::{BranchInfo, ChangeKind, CommitOptions, FileChange, HeadInfo, Repo, TagInfo};

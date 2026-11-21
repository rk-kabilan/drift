//! Git access. Reads (status, contents, blame, ahead/behind) go through libgit2;
//! writes (stage, unstage, discard, commit, push) go through the `git` command so
//! hooks, signing and credential helpers behave exactly as on the command line.

mod blame;
mod repo;

pub use blame::{relative_time_since, Blame};
pub use repo::{BranchInfo, ChangeKind, CommitOptions, FileChange, HeadInfo, Repo, TagInfo};

/// Caps libgit2's memory. By default it may map up to 8 GB of pack files on
/// 64-bit systems; a viewer only touches a few objects at a time, so small
/// windows keep resident memory low even in very large repositories.
pub fn limit_memory() {
    // SAFETY: called once at startup, before any other libgit2 use.
    unsafe {
        let _ = git2::opts::set_mwindow_size(8 * 1024 * 1024);
        let _ = git2::opts::set_mwindow_mapped_limit(64 * 1024 * 1024);
        let _ = git2::opts::set_mwindow_file_limit(64);
    }
}

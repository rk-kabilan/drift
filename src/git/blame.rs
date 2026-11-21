use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use git2::{BlameOptions, Repository};

/// Who last changed a line, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameInfo {
    pub author: String,
    /// Commit time as seconds since the Unix epoch.
    pub time: i64,
    pub summary: String,
    pub short_sha: String,
}

impl BlameInfo {
    pub fn relative_time(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        relative_time(now - self.time)
    }
}

/// Blame for every line of a file's working-tree contents. Lines that are not
/// committed yet have no entry.
#[derive(Clone, Debug, Default)]
pub struct Blame {
    lines: Vec<Option<BlameInfo>>,
}

impl Blame {
    /// Blame for a 1-based line number.
    pub fn line(&self, line: usize) -> Option<&BlameInfo> {
        self.lines.get(line.checked_sub(1)?)?.as_ref()
    }
}

pub(super) fn blame(repo: &Repository, path: &str, contents: &str) -> Result<Blame> {
    let mut opts = BlameOptions::new();
    let committed = repo.blame_file(Path::new(path), Some(&mut opts))?;
    let blame = committed.blame_buffer(contents.as_bytes())?;
    let line_count = contents.lines().count();
    let mut lines = vec![None; line_count];
    for hunk in blame.iter() {
        let oid = hunk.final_commit_id();
        if oid.is_zero() {
            continue;
        }
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        // Read the author from the commit: hunks from `blame_buffer` can carry a
        // null signature, which git2 would dereference.
        let author = commit.author();
        let info = BlameInfo {
            author: author.name().unwrap_or("unknown").to_string(),
            time: commit.time().seconds(),
            summary: commit.summary().unwrap_or("").to_string(),
            short_sha: super::repo::short(&oid),
        };
        let start = hunk.final_start_line().saturating_sub(1);
        for slot in lines.iter_mut().skip(start).take(hunk.lines_in_hunk()) {
            *slot = Some(info.clone());
        }
    }
    Ok(Blame { lines })
}

pub fn relative_time(secs: i64) -> String {
    let secs = secs.max(0);
    let (n, unit) = match secs {
        s if s < 60 => return "just now".into(),
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 86_400 * 7 => (s / 86_400, "day"),
        s if s < 86_400 * 30 => (s / (86_400 * 7), "week"),
        s if s < 86_400 * 365 => (s / (86_400 * 30), "month"),
        s => (s / (86_400 * 365), "year"),
    };
    if n == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

#[cfg(test)]
mod tests {
    use super::relative_time;

    #[test]
    fn formats_relative_times() {
        assert_eq!(relative_time(5), "just now");
        assert_eq!(relative_time(120), "2 minutes ago");
        assert_eq!(relative_time(3 * 86_400), "3 days ago");
        assert_eq!(relative_time(14 * 86_400), "2 weeks ago");
        assert_eq!(relative_time(400 * 86_400), "1 year ago");
    }
}

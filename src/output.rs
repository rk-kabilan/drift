//! The Output log shown in the bottom panel: git commands Drift ran, and the
//! errors and warnings of the operations they belong to.
//!
//! One bounded log for the whole app, filled from any thread (git commands run
//! in the background). Each entry remembers the repository it belongs to, so a
//! window shows only its own. The oldest entries drop off past
//! [`MAX_ENTRIES`] or [`MAX_BYTES`] of text.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 2_000;
pub const MAX_BYTES: usize = 512 * 1024;
/// Longer messages (a command with hundreds of paths) are cut to this.
const MAX_LINE: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// A git command and how long it took.
    Info,
    /// Something the user asked for can't go ahead ("Nothing to commit").
    Warning,
    /// A git command or operation failed.
    Error,
}

pub struct Entry {
    /// Increases by one for every entry ever added, so a window can tell which
    /// errors are new since it last showed the log.
    pub seq: u64,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub level: Level,
    /// The repository the entry belongs to; `None` shows in every window.
    pub root: Option<Arc<Path>>,
    pub text: Box<str>,
}

#[derive(Default)]
pub struct Log {
    entries: VecDeque<Entry>,
    bytes: usize,
    next_seq: u64,
    /// Repository paths in use, shared by their entries instead of copied.
    roots: Vec<Arc<Path>>,
}

impl Log {
    pub fn entries(&self) -> &VecDeque<Entry> {
        &self.entries
    }

    /// The `seq` the next entry will get.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    fn intern(&mut self, root: &Path) -> Arc<Path> {
        if let Some(r) = self.roots.iter().find(|r| ***r == *root) {
            return r.clone();
        }
        // Forget paths no entry uses any more.
        self.roots.retain(|r| Arc::strong_count(r) > 1);
        let r: Arc<Path> = Arc::from(root);
        self.roots.push(r.clone());
        r
    }

    fn push(&mut self, level: Level, root: Option<&Path>, text: &str) {
        let mut text = mask_credentials(text.trim_end());
        if text.len() > MAX_LINE {
            let mut cut = MAX_LINE;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        let root = root.map(|r| self.intern(r));
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        self.bytes += text.len();
        self.entries.push_back(Entry {
            seq: self.next_seq,
            time,
            level,
            root,
            text: text.into_boxed_str(),
        });
        self.next_seq += 1;
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            match self.entries.pop_front() {
                Some(e) => self.bytes -= e.text.len(),
                None => break,
            }
        }
    }

    /// Removes the entries of `root` (and those of no repository).
    fn clear(&mut self, root: &Path) {
        self.entries
            .retain(|e| e.root.as_deref().is_some_and(|r| r != root));
        self.bytes = self.entries.iter().map(|e| e.text.len()).sum();
        self.roots.retain(|r| Arc::strong_count(r) > 1);
    }
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// The log, locked. Keep the lock short: git commands log from other threads.
pub fn lock() -> LogGuard {
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(Log::default);
    LogGuard(guard)
}

pub struct LogGuard(MutexGuard<'static, Option<Log>>);

impl std::ops::Deref for LogGuard {
    type Target = Log;
    fn deref(&self) -> &Log {
        self.0.as_ref().expect("initialized in lock()")
    }
}

impl std::ops::DerefMut for LogGuard {
    fn deref_mut(&mut self) -> &mut Log {
        self.0.as_mut().expect("initialized in lock()")
    }
}

/// Adds a message to the log.
pub fn push(level: Level, root: Option<&Path>, text: &str) {
    lock().push(level, root, text);
}

pub fn clear(root: &Path) {
    lock().clear(root);
}

/// Whether an entry shows in the window of `root`.
pub fn belongs(e: &Entry, root: &Path) -> bool {
    e.root.as_deref().is_none_or(|r| r == root)
}

/// "14:03:27" in the local time zone.
pub fn clock(time: i64) -> String {
    let local = jiff::Timestamp::from_second(time)
        .ok()
        .map(|ts| ts.to_zoned(jiff::tz::TimeZone::system()));
    match local {
        Some(z) => format!("{:02}:{:02}:{:02}", z.hour(), z.minute(), z.second()),
        None => String::from("--:--:--"),
    }
}

/// Hides the password or token in URLs such as `https://user:token@host/repo`.
pub fn mask_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        // The authority ends at the first '/', whitespace or quote.
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                let user = &authority[..at];
                match user.split_once(':') {
                    Some((name, _)) => {
                        out.push_str(name);
                        out.push_str(":***");
                    }
                    None => out.push_str("***"),
                }
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Sends `log` crate warnings and errors to the Output log as well as to `inner`
/// (env_logger, which writes to stderr).
pub struct TeeLogger<L> {
    pub inner: L,
}

impl<L: log::Log> log::Log for TeeLogger<L> {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata) || metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        // Only Drift's own messages: gpui logs renderer details nobody acts on.
        if record.level() <= log::Level::Warn && record.target().starts_with("drift") {
            let level = if record.level() == log::Level::Error {
                Level::Error
            } else {
                Level::Warning
            };
            push(level, None, &record.args().to_string());
        }
        if self.inner.enabled(record.metadata()) {
            self.inner.log(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_tokens_in_urls() {
        assert_eq!(
            mask_credentials("fatal: https://me:ghp_secret@github.com/a/b.git not found"),
            "fatal: https://me:***@github.com/a/b.git not found"
        );
        assert_eq!(
            mask_credentials("https://token@host/x"),
            "https://***@host/x"
        );
        assert_eq!(
            mask_credentials("ssh://git@host:22/x and https://host/y"),
            "ssh://***@host:22/x and https://host/y"
        );
        assert_eq!(mask_credentials("no urls here"), "no urls here");
    }

    #[test]
    fn stays_within_its_caps() {
        let mut log = Log::default();
        let root = Path::new("/tmp/repo");
        for i in 0..(MAX_ENTRIES + 50) {
            log.push(Level::Info, Some(root), &format!("line {i}"));
        }
        assert_eq!(log.entries().len(), MAX_ENTRIES);
        assert_eq!(log.entries().front().unwrap().seq, 50);
        let big = "x".repeat(MAX_LINE);
        for _ in 0..(MAX_BYTES / MAX_LINE + 10) {
            log.push(Level::Error, Some(root), &big);
        }
        assert!(log.bytes <= MAX_BYTES);
        assert_eq!(log.roots.len(), 1);
        log.clear(root);
        assert!(log.entries().is_empty());
        assert_eq!(log.bytes, 0);
    }

    #[test]
    fn entries_belong_to_their_repository() {
        let mut log = Log::default();
        log.push(Level::Info, Some(Path::new("/a")), "a");
        log.push(Level::Info, Some(Path::new("/b")), "b");
        log.push(Level::Warning, None, "everyone");
        let shown: Vec<_> = log
            .entries()
            .iter()
            .filter(|e| belongs(e, Path::new("/a")))
            .map(|e| &*e.text)
            .collect();
        assert_eq!(shown, ["a", "everyone"]);
    }
}

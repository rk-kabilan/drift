//! Recently opened repositories, kept as a small text file with one path per line.
//!
//! The file lives in the per-user state folder:
//! macOS `~/Library/Application Support/drift/`, Windows `%APPDATA%\drift\`,
//! Linux `$XDG_STATE_HOME/drift/` (default `~/.local/state/drift/`).

use std::path::{Path, PathBuf};

const MAX: usize = 10;

/// Drift's per-user state folder (see the module docs).
pub fn state_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home().map(|h| h.join(".local/state")))
    }?;
    Some(dir.join("drift"))
}

fn file() -> Option<PathBuf> {
    state_dir().map(|d| d.join("recent-repositories"))
}

/// Recent repositories, newest first. Folders that no longer exist are left out.
pub fn load() -> Vec<PathBuf> {
    file().map(|f| load_from(&f)).unwrap_or_default()
}

/// Moves `path` to the top of the list and returns the new list.
pub fn add(path: &Path) -> Vec<PathBuf> {
    file().map(|f| add_in(&f, path)).unwrap_or_default()
}

/// Drops `path` from the list and returns the new list.
pub fn remove(path: &Path) -> Vec<PathBuf> {
    file().map(|f| remove_in(&f, path)).unwrap_or_default()
}

fn load_from(file: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut list: Vec<PathBuf> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let path = PathBuf::from(line);
        if path.is_dir() && !list.contains(&path) {
            list.push(path);
        }
        if list.len() == MAX {
            break;
        }
    }
    list
}

fn add_in(file: &Path, path: &Path) -> Vec<PathBuf> {
    let mut list = load_from(file);
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
    save(file, &list);
    list
}

fn remove_in(file: &Path, path: &Path) -> Vec<PathBuf> {
    let mut list = load_from(file);
    list.retain(|p| p != path);
    save(file, &list);
    list
}

fn save(file: &Path, list: &[PathBuf]) {
    let mut text = String::new();
    for p in list {
        // A path with a newline in it can't be stored one per line; skip it.
        if let Some(s) = p.to_str().filter(|s| !s.contains('\n')) {
            text.push_str(s);
            text.push('\n');
        }
    }
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(file, text));
    if let Err(e) = result {
        log::warn!(
            "could not save recent repositories to {}: {e}",
            file.display()
        );
    }
}

/// `path` with the home folder shown as `~`.
pub fn display(path: &Path) -> String {
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        if let Ok(rest) = path.strip_prefix(&home) {
            let sep = std::path::MAIN_SEPARATOR;
            return if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~{sep}{}", rest.display())
            };
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_and_limit() {
        let dir = std::env::temp_dir().join(format!("drift-recent-{}", std::process::id()));
        let file = dir.join("state/recent");
        let repos: Vec<PathBuf> = (0..12).map(|i| dir.join(format!("r{i}"))).collect();
        for r in &repos {
            std::fs::create_dir_all(r).unwrap();
        }
        for r in &repos {
            add_in(&file, r);
        }
        let list = load_from(&file);
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0], repos[11]);

        add_in(&file, &repos[5]);
        assert_eq!(load_from(&file)[0], repos[5]);
        assert_eq!(load_from(&file).len(), MAX);

        remove_in(&file, &repos[5]);
        assert!(!load_from(&file).contains(&repos[5]));

        std::fs::remove_dir_all(&repos[11]).unwrap();
        assert!(!load_from(&file).contains(&repos[11]));
        std::fs::remove_dir_all(&dir).ok();
    }
}

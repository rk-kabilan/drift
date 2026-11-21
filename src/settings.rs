//! User settings, kept as a small `key=value` text file in the state folder
//! (see [`crate::recent::state_dir`]):
//!
//! ```text
//! theme=dark
//! check_updates=false
//! key.CommandPalette=cmd-shift-p
//! ```
//!
//! `theme` is `system`, `light` or `dark`. Each `key.<Action>` line replaces the
//! default shortcut of that action (see [`crate::ui::shortcuts`]); an empty
//! value removes the shortcut. Unknown lines are kept as they are.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::theme::Appearance;

const FILE: &str = "settings";
const KEY_PREFIX: &str = "key.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow the system's light or dark appearance.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn forced(self) -> Option<Appearance> {
        match self {
            ThemeChoice::System => None,
            ThemeChoice::Light => Some(Appearance::Light),
            ThemeChoice::Dark => Some(Appearance::Dark),
        }
    }

    fn name(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "system" => Some(ThemeChoice::System),
            "light" => Some(ThemeChoice::Light),
            "dark" => Some(ThemeChoice::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Check for a new release once a day.
    pub check_updates: bool,
    /// Shortcut overrides: action id → keystroke (`""` means no shortcut).
    pub keys: BTreeMap<String, String>,
    /// Lines this version doesn't understand, written back unchanged.
    other: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            check_updates: true,
            keys: BTreeMap::new(),
            other: Vec::new(),
        }
    }
}

impl gpui::Global for Settings {}

impl Settings {
    pub fn load() -> Self {
        file().map(|f| load_from(&f)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(f) = file() {
            save_to(&f, self);
        }
    }
}

/// Changes the settings, saves them, and redraws every window.
pub fn update(cx: &mut gpui::App, f: impl FnOnce(&mut Settings)) {
    let mut s = cx.global::<Settings>().clone();
    f(&mut s);
    if &s != cx.global::<Settings>() {
        s.save();
        cx.set_global(s);
        cx.refresh_windows();
    }
}

fn file() -> Option<PathBuf> {
    crate::recent::state_dir().map(|d| d.join(FILE))
}

fn parse(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            s.other.push(line.to_string());
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            // A bad value for a known key is dropped, and the default used.
            "theme" => s.theme = ThemeChoice::from_name(value).unwrap_or_default(),
            "check_updates" => s.check_updates = value != "false",
            _ => match key.strip_prefix(KEY_PREFIX) {
                Some(action) if !action.is_empty() => {
                    s.keys.insert(action.to_string(), value.to_string());
                }
                _ => s.other.push(line.to_string()),
            },
        }
    }
    s
}

fn format(s: &Settings) -> String {
    let mut text = format!(
        "theme={}\ncheck_updates={}\n",
        s.theme.name(),
        s.check_updates
    );
    for (action, keys) in &s.keys {
        text.push_str(&format!("{KEY_PREFIX}{action}={keys}\n"));
    }
    for line in &s.other {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn load_from(file: &Path) -> Settings {
    std::fs::read_to_string(file)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

fn save_to(file: &Path, s: &Settings) {
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(file, format(s)));
    if let Err(e) = result {
        log::warn!("could not save settings to {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "theme=dark\ncheck_updates=false\nkey.Refresh=f5\nkey.Quit=\nfuture=1\n";
        let s = parse(text);
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(!s.check_updates);
        assert_eq!(s.keys.get("Refresh").map(String::as_str), Some("f5"));
        assert_eq!(s.keys.get("Quit").map(String::as_str), Some(""));
        assert_eq!(parse(&format(&s)), s);
        assert!(format(&s).contains("future=1"));
    }

    #[test]
    fn bad_values_fall_back() {
        let s = parse("theme=purple\ncheck_updates=maybe\n");
        assert_eq!(s.theme, ThemeChoice::System);
        assert!(s.check_updates);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("drift-settings-{}", std::process::id()));
        let file = dir.join("state/settings");
        let mut s = Settings {
            theme: ThemeChoice::Light,
            ..Default::default()
        };
        s.keys.insert("CommandPalette".into(), "ctrl-k".into());
        save_to(&file, &s);
        assert_eq!(load_from(&file), s);
        std::fs::remove_dir_all(&dir).ok();
    }
}

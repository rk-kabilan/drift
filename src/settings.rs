//! User settings, kept as a small `key=value` text file in the state folder
//! (see [`crate::recent::state_dir`]):
//!
//! ```text
//! theme=dark
//! check_updates=false
//! word_wrap=false
//! show_graph=true
//! auto_fetch=true
//! auto_fetch_interval=5
//! key.CommandPalette=cmd-shift-p
//! ```
//!
//! `theme` is `system`, `light` or `dark`. `auto_fetch_interval` is in seconds. Each `key.<Action>` line replaces the
//! default shortcut of that action (see [`crate::ui::shortcuts`]); an empty
//! value removes the shortcut. Unknown lines are kept as they are.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::theme::Appearance;

const FILE: &str = "settings";
const KEY_PREFIX: &str = "key.";

/// Seconds between background fetches unless the user picks another interval.
pub const DEFAULT_FETCH_INTERVAL: u64 = 5;
/// Height of the bottom panel until it is dragged.
pub const DEFAULT_PANEL_HEIGHT: f32 = 240.;
/// Shortest interval accepted, so a typo can't hammer the remote.
pub const MIN_FETCH_INTERVAL: u64 = 2;
/// The steps the interval control in Settings moves through.
pub const FETCH_INTERVALS: &[u64] = &[5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTab {
    Output,
    Terminal,
}

impl PanelTab {
    pub fn name(self) -> &'static str {
        match self {
            PanelTab::Output => "output",
            PanelTab::Terminal => "terminal",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "output" => Some(PanelTab::Output),
            "terminal" => Some(PanelTab::Terminal),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Check for a new release once a day.
    pub check_updates: bool,
    /// Wrap long lines in the diff instead of scrolling sideways.
    pub word_wrap: bool,
    /// The history graph section at the bottom of the changes pane is open.
    pub show_graph: bool,
    /// Fetch from the remotes in the background every `auto_fetch_interval` seconds.
    pub auto_fetch: bool,
    pub auto_fetch_interval: u64,
    /// The bottom panel is open, on which tab, and how tall.
    pub panel_open: bool,
    pub panel_tab: PanelTab,
    pub panel_height: f32,
    /// The terminal's shell as a command line ("/bin/bash -l"); `None` starts
    /// the user's login shell (PowerShell on Windows).
    pub terminal_shell: Option<String>,
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
            word_wrap: false,
            show_graph: false,
            auto_fetch: true,
            auto_fetch_interval: DEFAULT_FETCH_INTERVAL,
            panel_open: false,
            panel_tab: PanelTab::Output,
            panel_height: DEFAULT_PANEL_HEIGHT,
            terminal_shell: None,
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

/// "5s", "2m", "1h" for a fetch interval in seconds.
pub fn interval_label(secs: u64) -> String {
    match secs {
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// The next (or previous) step of [`FETCH_INTERVALS`] from `secs`.
pub fn step_interval(secs: u64, up: bool) -> u64 {
    if up {
        FETCH_INTERVALS
            .iter()
            .copied()
            .find(|&s| s > secs)
            .unwrap_or(secs)
    } else {
        FETCH_INTERVALS
            .iter()
            .rev()
            .copied()
            .find(|&s| s < secs)
            .unwrap_or(secs)
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
            "word_wrap" => s.word_wrap = value == "true",
            "show_graph" => s.show_graph = value == "true",
            "auto_fetch" => s.auto_fetch = value != "false",
            "auto_fetch_interval" => {
                s.auto_fetch_interval = value
                    .parse::<u64>()
                    .ok()
                    .filter(|&n| n >= MIN_FETCH_INTERVAL)
                    .unwrap_or(DEFAULT_FETCH_INTERVAL)
            }
            "panel_open" => s.panel_open = value == "true",
            "panel_tab" => s.panel_tab = PanelTab::from_name(value).unwrap_or(PanelTab::Output),
            "panel_height" => {
                s.panel_height = value
                    .parse::<f32>()
                    .ok()
                    .filter(|h| h.is_finite() && *h >= 60.)
                    .unwrap_or(DEFAULT_PANEL_HEIGHT)
            }
            "terminal_shell" => {
                s.terminal_shell = Some(value.to_string()).filter(|v| !v.is_empty())
            }
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
        "theme={}\ncheck_updates={}\nword_wrap={}\nshow_graph={}\nauto_fetch={}\nauto_fetch_interval={}\n",
        s.theme.name(),
        s.check_updates,
        s.word_wrap,
        s.show_graph,
        s.auto_fetch,
        s.auto_fetch_interval
    );
    text.push_str(&format!(
        "panel_open={}\npanel_tab={}\npanel_height={}\n",
        s.panel_open,
        s.panel_tab.name(),
        s.panel_height.round()
    ));
    if let Some(shell) = &s.terminal_shell {
        text.push_str(&format!("terminal_shell={shell}\n"));
    }
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
        let text = "theme=dark\ncheck_updates=false\nword_wrap=true\nshow_graph=true\nauto_fetch=false\nauto_fetch_interval=30\npanel_open=true\npanel_tab=terminal\npanel_height=300\nterminal_shell=/bin/bash -l\nkey.Refresh=f5\nkey.Quit=\nfuture=1\n";
        let s = parse(text);
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert!(!s.check_updates);
        assert!(s.word_wrap && s.show_graph && !s.auto_fetch);
        assert_eq!(s.auto_fetch_interval, 30);
        assert!(s.panel_open);
        assert_eq!(s.panel_tab, PanelTab::Terminal);
        assert_eq!(s.panel_height, 300.);
        assert_eq!(s.terminal_shell.as_deref(), Some("/bin/bash -l"));
        assert_eq!(s.keys.get("Refresh").map(String::as_str), Some("f5"));
        assert_eq!(s.keys.get("Quit").map(String::as_str), Some(""));
        assert_eq!(parse(&format(&s)), s);
        assert!(format(&s).contains("future=1"));
    }

    #[test]
    fn interval_steps() {
        assert_eq!(interval_label(5), "5s");
        assert_eq!(interval_label(120), "2m");
        assert_eq!(interval_label(3600), "1h");
        assert_eq!(step_interval(5, true), 10);
        assert_eq!(step_interval(5, false), 5);
        assert_eq!(step_interval(7, false), 5);
        assert_eq!(step_interval(3600, true), 3600);
    }

    #[test]
    fn bad_values_fall_back() {
        let s = parse("theme=purple\ncheck_updates=maybe\nauto_fetch_interval=1\n");
        assert_eq!(s.theme, ThemeChoice::System);
        assert!(s.check_updates);
        assert!(s.auto_fetch);
        assert_eq!(s.auto_fetch_interval, DEFAULT_FETCH_INTERVAL);
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

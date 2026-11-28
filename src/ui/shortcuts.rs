//! Keyboard shortcuts: the actions that can have one, their defaults, and the
//! user's overrides from [`Settings`]. [`apply`] rebinds them while the app runs.

use gpui::{App, KeyBinding, Keystroke};

use super::workspace::*;
use crate::settings::Settings;

/// An action that can be given a keyboard shortcut.
pub struct Shortcut {
    /// The action's name, used as the key in the settings file.
    pub id: &'static str,
    pub title: &'static str,
    /// Default keystrokes, in gpui syntax (`secondary` is Cmd on macOS, Ctrl elsewhere).
    pub defaults: &'static [&'static str],
}

macro_rules! shortcuts {
    ($($action:ident => $title:literal, [$($key:literal),*];)*) => {
        pub static SHORTCUTS: &[Shortcut] = &[
            $(Shortcut { id: stringify!($action), title: $title, defaults: &[$($key),*] }),*
        ];

        /// A binding of `keys` to the action named `id`. `keys` must be valid.
        fn binding(id: &str, keys: &str) -> Option<KeyBinding> {
            match id {
                $(stringify!($action) => Some(KeyBinding::new(keys, $action, context(id))),)*
                _ => None,
            }
        }
    };
}

shortcuts! {
    CommandPalette => "Show Command Palette", ["secondary-shift-p", "f1"];
    QuickOpen => "Go to File", ["secondary-p"];
    OpenSettings => "Open Settings", ["secondary-,"];
    Commit => "Commit", ["secondary-enter"];
    Refresh => "Refresh", ["secondary-r", "f5"];
    NextChange => "Next Change", ["f7"];
    PreviousChange => "Previous Change", ["shift-f7"];
    SwitchBranch => "Switch Branch", ["secondary-shift-b"];
    Fetch => "Fetch", [];
    Pull => "Pull", [];
    Push => "Push", [];
    Sync => "Sync", [];
    Save => "Save", ["secondary-s"];
    CloseTab => "Close Tab", ["secondary-w"];
    CloseWindow => "Close Window", ["secondary-shift-w"];
    NextTab => "Next Tab", ["ctrl-tab"];
    PreviousTab => "Previous Tab", ["ctrl-shift-tab"];
    ToggleLayout => "Toggle Inline / Side by Side", [];
    ToggleWordWrap => "Toggle Word Wrap", ["alt-z"];
    ToggleGraph => "Toggle History Graph", ["secondary-shift-g"];
    TogglePanel => "Toggle Panel", ["secondary-j"];
    ToggleTerminal => "Toggle Terminal", ["ctrl-`"];
    NewTerminal => "New Terminal", ["ctrl-shift-`", "ctrl-~"];
    ShowOutput => "Show Output", ["secondary-shift-u"];
    ClearOutput => "Clear Output", [];
    ToggleMaximizedPanel => "Toggle Maximized Panel", [];
    KillTerminal => "Kill Terminal", [];
    FocusNextTerminal => "Focus Next Terminal", ["secondary-shift-]", "ctrl-pagedown"];
    FocusPreviousTerminal => "Focus Previous Terminal", ["secondary-shift-[", "ctrl-pageup"];
    NextWindowTab => "Show Next Window Tab", ["secondary-shift-]"];
    PreviousWindowTab => "Show Previous Window Tab", ["secondary-shift-["];
    ToggleTheme => "Toggle Light / Dark Theme", [];
    OpenRepository => "Open Repository", ["secondary-o"];
    NewWindow => "New Window", ["secondary-shift-n"];
    CheckForUpdates => "Check for Updates", [];
    Quit => "Quit", ["secondary-q"];
}

/// Where a binding applies: everywhere, or only in a terminal. In a terminal,
/// Cmd+Shift+] and [ move between terminals; elsewhere between window tabs.
fn context(id: &str) -> Option<&'static str> {
    match id {
        "FocusNextTerminal" | "FocusPreviousTerminal" => Some("Terminal"),
        _ => None,
    }
}

/// Whether the action exists on this platform: only macOS has window tabs.
pub fn available(id: &str) -> bool {
    cfg!(target_os = "macos") || !matches!(id, "NextWindowTab" | "PreviousWindowTab")
}

/// Whether the same keys on `a` and `b` would be a clash: both exist here and
/// apply in the same place.
pub fn can_clash(a: &str, b: &str) -> bool {
    a != b && available(a) && available(b) && context(a) == context(b)
}

pub fn find(id: &str) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|s| s.id == id)
}

/// Whether `keys` is a keystroke (or a sequence of them) gpui can parse.
pub fn valid(keys: &str) -> bool {
    !keys.trim().is_empty()
        && keys
            .split_whitespace()
            .all(|k| Keystroke::parse(k).is_ok_and(|k| !k.key.is_empty()))
}

/// The shortcuts of `s` after the user's override, if any. An invalid
/// override is ignored and the defaults stay.
pub fn keys_for(s: &Shortcut, settings: &Settings) -> Vec<String> {
    match settings.keys.get(s.id) {
        Some(k) if k.trim().is_empty() => Vec::new(),
        Some(k) if valid(k) => vec![k.trim().to_string()],
        _ => s.defaults.iter().map(|k| k.to_string()).collect(),
    }
}

/// Whether two keystroke sequences are the same keys (`secondary-p` and
/// `cmd-p` are, on macOS).
pub fn same(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        s.split_whitespace()
            .map(|k| Keystroke::parse(k).ok().map(|k| (k.modifiers, k.key)))
            .collect::<Option<Vec<_>>>()
    };
    matches!((parse(a), parse(b)), (Some(x), Some(y)) if x == y)
}

/// Whether the user changed the shortcut of `s`.
pub fn is_custom(s: &Shortcut, settings: &Settings) -> bool {
    settings.keys.contains_key(s.id)
}

/// Actions whose keys still reach Drift while a terminal has focus, as in
/// VS Code's `commandsToSkipShell`. Every other shortcut goes to the shell.
const TERMINAL_PASS_THROUGH: &[&str] = &[
    "TogglePanel",
    "ToggleTerminal",
    "NewTerminal",
    "ShowOutput",
    "ClearOutput",
    "ToggleMaximizedPanel",
    "KillTerminal",
    "FocusNextTerminal",
    "FocusPreviousTerminal",
    "CommandPalette",
    "QuickOpen",
    "Quit",
];

/// The key bindings of every action, with the user's overrides.
pub fn bindings(settings: &Settings) -> Vec<KeyBinding> {
    let mut out: Vec<KeyBinding> = SHORTCUTS
        .iter()
        .filter(|s| available(s.id))
        .flat_map(|s| {
            keys_for(s, settings)
                .into_iter()
                .filter_map(move |k| binding(s.id, &k))
        })
        .collect();
    // In a terminal, keys the shell needs (Ctrl+R, Ctrl+W, F5…) go to the
    // shell. Cmd shortcuts on macOS never reach a shell, so they stay.
    for s in SHORTCUTS {
        if TERMINAL_PASS_THROUGH.contains(&s.id) || !available(s.id) {
            continue;
        }
        for k in keys_for(s, settings) {
            let reaches_shell = k
                .split_whitespace()
                .next()
                .and_then(|k| Keystroke::parse(k).ok())
                .is_some_and(|k| !k.modifiers.platform);
            if reaches_shell {
                out.push(KeyBinding::new(&k, gpui::NoAction, Some("Terminal")));
            }
        }
    }
    out
}

/// Rebinds every key from the current settings.
pub fn apply(cx: &mut App) {
    let bindings = bindings(cx.global::<Settings>());
    cx.clear_key_bindings();
    cx.bind_keys(super::text_input::key_bindings());
    cx.bind_keys(super::terminal_view::key_bindings());
    cx.bind_keys(super::panel::key_bindings());
    cx.bind_keys(bindings);
}

/// The first shortcut of the action `id`, for display (e.g. "⌘⇧P" or "Ctrl+Shift+P").
pub fn label(id: &str, cx: &App) -> Option<String> {
    let s = find(id)?;
    keys_for(s, cx.global::<Settings>())
        .first()
        .map(|k| display(k))
}

/// `" (⌘⇧B)"` after a tooltip, or nothing if the action has no shortcut.
pub fn hint(id: &str, cx: &App) -> String {
    label(id, cx).map(|l| format!(" ({l})")).unwrap_or_default()
}

/// Whether a key pressed on its own is a modifier (not a shortcut yet).
pub fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "shift"
            | "control"
            | "ctrl"
            | "alt"
            | "option"
            | "platform"
            | "cmd"
            | "super"
            | "win"
            | "function"
            | "fn"
            | "capslock"
    )
}

/// Whether `k` can be a shortcut: it needs Ctrl, Alt or Cmd, or a function key,
/// so plain typing in the message box is never taken over.
pub fn usable(k: &Keystroke) -> bool {
    let m = &k.modifiers;
    m.control || m.alt || m.platform || m.function || is_function_key(&k.key)
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

/// A keystroke sequence the way the platform shows it: "⌘⇧P" on macOS,
/// "Ctrl+Shift+P" elsewhere.
pub fn display(keys: &str) -> String {
    keys.split_whitespace()
        .map(|k| match Keystroke::parse(k) {
            Ok(k) => display_one(&k, cfg!(target_os = "macos")),
            Err(_) => k.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_one(k: &Keystroke, mac: bool) -> String {
    let m = &k.modifiers;
    let key = key_name(&k.key, mac);
    if mac {
        let mut s = String::new();
        for (on, sym) in [
            (m.control, "⌃"),
            (m.alt, "⌥"),
            (m.shift, "⇧"),
            (m.platform, "⌘"),
            (m.function, "fn "),
        ] {
            if on {
                s.push_str(sym);
            }
        }
        s + &key
    } else {
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [
            (m.control, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.platform, if cfg!(windows) { "Win" } else { "Super" }),
            (m.function, "Fn"),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&key);
        parts.join("+")
    }
}

fn key_name(key: &str, mac: bool) -> String {
    let named = match key {
        "enter" => Some(if mac { "↩" } else { "Enter" }),
        "tab" => Some(if mac { "⇥" } else { "Tab" }),
        "escape" => Some(if mac { "⎋" } else { "Esc" }),
        "backspace" => Some(if mac { "⌫" } else { "Backspace" }),
        "delete" => Some(if mac { "⌦" } else { "Delete" }),
        "space" => Some("Space"),
        "up" => Some("↑"),
        "down" => Some("↓"),
        "left" => Some("←"),
        "right" => Some("→"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"),
        _ => None,
    };
    match named {
        Some(n) => n.to_string(),
        None if is_function_key(key) => key.to_uppercase(),
        None => {
            let mut c = key.chars();
            match c.next() {
                Some(first) => first.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_unique() {
        for s in SHORTCUTS {
            for k in s.defaults {
                assert!(valid(k), "{} has a bad default {k}", s.id);
                assert!(binding(s.id, k).is_some());
            }
        }
        let mut ids: Vec<_> = SHORTCUTS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SHORTCUTS.len());
    }

    #[test]
    fn overrides() {
        let mut settings = Settings::default();
        let palette = find("CommandPalette").unwrap();
        assert_eq!(keys_for(palette, &settings).len(), 2);
        settings
            .keys
            .insert("CommandPalette".into(), "ctrl-k".into());
        assert_eq!(keys_for(palette, &settings), vec!["ctrl-k".to_string()]);
        settings.keys.insert("CommandPalette".into(), String::new());
        assert!(keys_for(palette, &settings).is_empty());
        settings
            .keys
            .insert("CommandPalette".into(), "ctrl-".into());
        assert_eq!(
            keys_for(palette, &settings).len(),
            2,
            "bad override ignored"
        );
    }

    #[test]
    fn display_names() {
        let k = Keystroke::parse("ctrl-shift-p").unwrap();
        assert_eq!(display_one(&k, false), "Ctrl+Shift+P");
        let k = Keystroke::parse("cmd-shift-p").unwrap();
        assert_eq!(display_one(&k, true), "⇧⌘P");
        let k = Keystroke::parse("shift-f7").unwrap();
        assert_eq!(display_one(&k, false), "Shift+F7");
        let k = Keystroke::parse("ctrl-,").unwrap();
        assert_eq!(display_one(&k, false), "Ctrl+,");
    }

    #[test]
    fn usable_keys() {
        assert!(usable(&Keystroke::parse("ctrl-k").unwrap()));
        assert!(usable(&Keystroke::parse("f5").unwrap()));
        assert!(!usable(&Keystroke::parse("shift-k").unwrap()));
        assert!(!usable(&Keystroke::parse("k").unwrap()));
    }
}

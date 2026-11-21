//! Light and dark palettes, taken from the approved mock.

use gpui::{App, Global, Hsla, Rgba, WindowAppearance};

use crate::syntax::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl From<WindowAppearance> for Appearance {
    fn from(a: WindowAppearance) -> Self {
        match a {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            _ => Appearance::Light,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub appearance: Appearance,
    pub chrome: Hsla,
    pub panel: Hsla,
    pub editor: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub border: Hsla,
    pub hover: Hsla,
    pub sel: Hsla,
    pub sel_fg: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub on_accent: Hsla,
    pub input: Hsla,
    pub badge: Hsla,
    pub badge_fg: Hsla,
    pub add_line: Hsla,
    pub add_chg: Hsla,
    pub del_line: Hsla,
    pub del_chg: Hsla,
    pub filler: Hsla,
    pub cursor_line: Hsla,
    pub hunk: Hsla,
    pub hunk_fg: Hsla,
    pub st_modified: Hsla,
    pub st_added: Hsla,
    pub st_deleted: Hsla,
    pub st_untracked: Hsla,
    pub error: Hsla,
    pub syn_keyword: Hsla,
    pub syn_string: Hsla,
    pub syn_comment: Hsla,
    pub syn_type: Hsla,
    pub syn_number: Hsla,
    pub syn_function: Hsla,
    pub ghost: Hsla,
}

fn c(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

fn ca(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a,
    }
    .into()
}

impl Theme {
    pub fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            chrome: c(0xe4e4e4),
            panel: c(0xf3f3f3),
            editor: c(0xffffff),
            fg: c(0x3b3b3b),
            muted: c(0x767676),
            faint: c(0xa5a5a5),
            border: c(0xdedede),
            hover: c(0xe6e6e6),
            sel: c(0xdbe6f6),
            sel_fg: c(0x1f1f1f),
            accent: c(0x3d79c8),
            accent_hover: c(0x3469b0),
            on_accent: c(0xffffff),
            input: c(0xffffff),
            badge: c(0xc4c4c4),
            badge_fg: c(0x333333),
            add_line: ca(46, 160, 67, 0.13),
            add_chg: ca(46, 160, 67, 0.32),
            del_line: ca(215, 58, 73, 0.10),
            del_chg: ca(215, 58, 73, 0.30),
            filler: ca(0, 0, 0, 0.06),
            cursor_line: ca(0, 0, 0, 0.045),
            hunk: c(0xf0f4fa),
            hunk_fg: c(0x5a6b85),
            st_modified: c(0x895503),
            st_added: c(0x587c0c),
            st_deleted: c(0xad0707),
            st_untracked: c(0x587c0c),
            error: c(0xad0707),
            syn_keyword: c(0x0000ff),
            syn_string: c(0xa31515),
            syn_comment: c(0x008000),
            syn_type: c(0x267f99),
            syn_number: c(0x098658),
            syn_function: c(0x795e26),
            ghost: c(0x9a9a9a),
        }
    }

    pub fn dark() -> Self {
        Self {
            appearance: Appearance::Dark,
            chrome: c(0x181818),
            panel: c(0x1f1f1f),
            editor: c(0x1e1e1e),
            fg: c(0xcccccc),
            muted: c(0x9d9d9d),
            faint: c(0x6e6e6e),
            border: c(0x2b2b2b),
            hover: c(0x2a2d2e),
            sel: c(0x04395e),
            sel_fg: c(0xffffff),
            accent: c(0x0078d4),
            accent_hover: c(0x026ec1),
            on_accent: c(0xffffff),
            input: c(0x313131),
            badge: c(0x616161),
            badge_fg: c(0xf0f0f0),
            add_line: ca(46, 160, 67, 0.15),
            add_chg: ca(46, 160, 67, 0.38),
            del_line: ca(248, 81, 73, 0.13),
            del_chg: ca(248, 81, 73, 0.36),
            filler: ca(255, 255, 255, 0.05),
            cursor_line: ca(255, 255, 255, 0.05),
            hunk: c(0x1f2a37),
            hunk_fg: c(0x8aa2c2),
            st_modified: c(0xe2c08d),
            st_added: c(0x81b88b),
            st_deleted: c(0xc74e39),
            st_untracked: c(0x73c991),
            error: c(0xf48771),
            syn_keyword: c(0x569cd6),
            syn_string: c(0xce9178),
            syn_comment: c(0x6a9955),
            syn_type: c(0x4ec9b0),
            syn_number: c(0xb5cea8),
            syn_function: c(0xdcdcaa),
            ghost: c(0x6b6b6b),
        }
    }

    pub fn for_appearance(a: Appearance) -> Self {
        match a {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
        }
    }

    pub fn token(&self, t: Token) -> Hsla {
        match t {
            Token::Keyword => self.syn_keyword,
            Token::String => self.syn_string,
            Token::Comment => self.syn_comment,
            Token::Type => self.syn_type,
            Token::Number => self.syn_number,
            Token::Function => self.syn_function,
        }
    }
}

/// The app-wide theme plus the fonts picked for this machine.
pub struct ThemeState {
    pub theme: Theme,
    /// `None` follows the OS; `Some` is a manual override.
    pub forced: Option<Appearance>,
    pub ui_font: String,
    pub mono_font: String,
}

impl Global for ThemeState {}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        &self.global::<ThemeState>().theme
    }
}

/// Picks the first installed font from a preference list.
pub fn pick_font(installed: &[String], prefs: &[&str], fallback: &str) -> String {
    prefs
        .iter()
        .find(|p| installed.iter().any(|f| f == *p))
        .map(|s| s.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

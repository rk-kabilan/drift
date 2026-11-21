//! Icons compiled into the binary, so the app is a single self-contained file.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

static ICONS: &[(&str, &[u8])] = icons![
    "arrow-down",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "branch",
    "check",
    "chevron-down",
    "chevron-right",
    "close",
    "cloud",
    "commit",
    "discard",
    "file",
    "inline",
    "maximize",
    "minus",
    "moon",
    "more",
    "open",
    "plus",
    "refresh",
    "repo",
    "search",
    "split",
    "sun",
    "stash",
    "sync",
    "tag",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

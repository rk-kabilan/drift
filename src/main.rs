//! Drift: a fast Git desktop client.

// Release builds on Windows are GUI apps: no console window opens next to Drift.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod diff;
mod fuzzy;
mod git;
mod merge;
mod recent;
mod settings;
mod syntax;
mod theme;
mod ui;
mod update;

use std::path::PathBuf;

use gpui::{App, Application};

use crate::theme::{pick_font, Theme, ThemeState};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    git::limit_memory();
    // `drift <path>` opens that repository. Otherwise the repository around the
    // current folder opens if there is one, else the welcome screen shows.
    let start = std::env::args_os()
        .nth(1)
        .map(|p| (PathBuf::from(p), true))
        .or_else(|| std::env::current_dir().ok().map(|p| (p, false)))
        .map(|(p, named)| (std::fs::canonicalize(&p).unwrap_or(p), named));

    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_global(settings::Settings::load());
            let installed = cx.text_system().all_font_names();
            let ui_font = pick_font(
                &installed,
                &["Inter", "Segoe UI", "Ubuntu", "Cantarell"],
                ".SystemUIFont",
            );
            let mono_font = pick_font(
                &installed,
                &[
                    "JetBrains Mono",
                    "SF Mono",
                    "Menlo",
                    "Cascadia Mono",
                    "Consolas",
                    "DejaVu Sans Mono",
                    "Ubuntu Mono",
                    "Liberation Mono",
                    "Noto Sans Mono",
                ],
                "monospace",
            );
            let ui_font = if cfg!(target_os = "macos") {
                ".SystemUIFont".to_string()
            } else {
                ui_font
            };
            cx.set_global(ThemeState {
                theme: Theme::light(),
                forced: None,
                ui_font,
                mono_font,
            });

            // macOS and Windows show native dialogs; elsewhere draw one in the app's theme.
            if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
                cx.set_prompt_builder(ui::prompt::themed_prompt);
            }
            ui::shortcuts::apply(cx);

            update::start(cx);
            ui::open_window(start, None, cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}

//! drift: a fast, git-only desktop client.

mod assets;
mod diff;
mod fuzzy;
mod git;
mod merge;
mod syntax;
mod theme;
mod ui;

use std::path::PathBuf;

use gpui::{
    px, size, App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds,
    WindowOptions,
};

use crate::theme::{pick_font, Appearance, Theme, ThemeState};
use crate::ui::Workspace;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    git::limit_memory();
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let root = std::fs::canonicalize(&root).unwrap_or(root);

    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
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
            cx.bind_keys(ui::text_input::key_bindings());
            cx.bind_keys(ui::actions_bindings());

            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("drift".into()),
                            appears_transparent: cfg!(target_os = "macos"),
                            traffic_light_position: Some(gpui::point(px(12.), px(11.))),
                        }),
                        window_min_size: Some(size(px(760.), px(420.))),
                        app_id: Some("drift".into()),
                        ..Default::default()
                    },
                    |window, cx| {
                        // DRIFT_THEME=light|dark pins the theme; otherwise it follows the OS.
                        let forced = match std::env::var("DRIFT_THEME").as_deref() {
                            Ok("dark") => Some(Appearance::Dark),
                            Ok("light") => Some(Appearance::Light),
                            _ => None,
                        };
                        let appearance =
                            forced.unwrap_or_else(|| Appearance::from(window.appearance()));
                        let state = cx.global_mut::<ThemeState>();
                        state.theme = Theme::for_appearance(appearance);
                        state.forced = forced;
                        cx.new(|cx| Workspace::new(root.clone(), window, cx))
                    },
                )
                .expect("failed to open the main window");
            window
                .update(cx, |ws, window, cx| {
                    use gpui::Focusable as _;
                    let handle = ws.message_focus(cx);
                    window.focus(&handle);
                    let _ = ws.focus_handle(cx);
                })
                .ok();
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}

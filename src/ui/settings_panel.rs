//! The Settings panel (Cmd/Ctrl+,): theme, update checks and keyboard shortcuts.
//! Changes apply at once and are saved to the settings file.

use gpui::{
    div, prelude::*, px, AnyElement, Context, FocusHandle, Focusable as _, Hsla, KeyDownEvent,
    Keystroke, SharedString, Stateful, Window,
};

use super::shortcuts::{self, Shortcut, SHORTCUTS};
use super::widgets::icon_button;
use super::workspace::Workspace;
use crate::settings::{Settings, ThemeChoice};
use crate::theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Shortcuts,
}

pub struct SettingsView {
    page: Page,
    /// The shortcut waiting for a new keystroke.
    recording: Option<&'static str>,
    /// A note under the shortcut list, and whether it is a warning.
    note: Option<(String, bool)>,
    focus: FocusHandle,
    _recorder: Option<gpui::Subscription>,
}

impl Workspace {
    pub fn open_settings(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(window, cx);
        self.menu = None;
        match &mut self.settings {
            Some(view) => view.page = page,
            None => {
                self.settings = Some(SettingsView {
                    page,
                    recording: None,
                    note: None,
                    focus: cx.focus_handle(),
                    _recorder: None,
                })
            }
        }
        if let Some(view) = &self.settings {
            window.focus(&view.focus);
        }
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            if self.repo.is_some() {
                window.focus(&self.message.focus_handle(cx));
            } else {
                window.focus(&self.focus_handle(cx));
            }
            cx.notify();
        }
    }

    /// "Color Theme…" in the command palette: System, Light or Dark.
    pub fn pick_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = cx.global::<Settings>().theme;
        let choices = [
            (ThemeChoice::System, "System", "follows your computer"),
            (ThemeChoice::Light, "Light", ""),
            (ThemeChoice::Dark, "Dark", ""),
        ];
        let muted = cx.theme().muted;
        let items = choices
            .iter()
            .map(|&(choice, name, note)| super::picker::PickerItem {
                haystack: name.to_string(),
                label_start: 0,
                note: note.to_string(),
                icon: "",
                icon_color: None,
                badge: (choice == current).then(|| ("current".to_string(), muted)),
                section: None,
            })
            .collect();
        self.pick_list(
            "Select a color theme",
            "No matching themes.",
            items,
            move |this, ix, window, cx| {
                if let Some(&(choice, _, _)) = choices.get(ix) {
                    this.set_theme(choice, window, cx)
                }
            },
            window,
            cx,
        );
    }

    /// Waits for the next keystroke in this window and makes it the shortcut of `id`.
    fn start_recording(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let this_window = window.window_handle();
        // Interceptors run before key bindings, so the keystroke records instead
        // of running whatever it is bound to now.
        let recorder = cx.intercept_keystrokes(move |e, window, cx| {
            if window.window_handle() != this_window {
                return;
            }
            cx.stop_propagation();
            let keystroke = e.keystroke.clone();
            weak.update(cx, |this, cx| this.record(keystroke, cx)).ok();
        });
        if let Some(view) = &mut self.settings {
            view.recording = Some(id);
            view.note = None;
            view._recorder = Some(recorder);
            window.focus(&view.focus);
        }
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &mut self.settings {
            view.recording = None;
            // Dropped later: this may run inside the interceptor itself.
            let recorder = view._recorder.take();
            cx.defer(move |_| drop(recorder));
        }
        cx.notify();
    }

    fn record(&mut self, k: Keystroke, cx: &mut Context<Self>) {
        let Some(id) = self.settings.as_ref().and_then(|v| v.recording) else {
            return;
        };
        let m = &k.modifiers;
        let plain = !(m.control || m.alt || m.platform || m.shift || m.function);
        if k.key == "escape" && plain {
            self.stop_recording(cx);
            return;
        }
        if shortcuts::is_modifier_key(&k.key) {
            return; // Still being pressed together; wait for the key itself.
        }
        if !shortcuts::usable(&k) {
            if let Some(view) = &mut self.settings {
                view.note = Some((
                    "Use the key with Ctrl, Alt or Cmd, or pick a function key (F1 to F24).".into(),
                    true,
                ));
            }
            cx.notify();
            return;
        }
        let keys = Keystroke {
            key_char: None,
            ..k
        }
        .unparse();
        self.stop_recording(cx);
        self.set_shortcut(id, Some(keys), cx);
    }

    /// Sets the shortcut of `id`: `Some("")` removes it, `None` restores the default.
    fn set_shortcut(&mut self, id: &'static str, keys: Option<String>, cx: &mut Context<Self>) {
        let Some(s) = shortcuts::find(id) else { return };
        // Choosing exactly the default is the same as resetting.
        let keys = keys.filter(|k| !(s.defaults.len() == 1 && shortcuts::same(k, s.defaults[0])));
        crate::settings::update(cx, |settings| match &keys {
            Some(k) => {
                settings.keys.insert(id.to_string(), k.clone());
            }
            None => {
                settings.keys.remove(id);
            }
        });
        shortcuts::apply(cx);
        let note = keys.filter(|k| !k.is_empty()).and_then(|k| {
            let settings = cx.global::<Settings>();
            let other = SHORTCUTS.iter().find(|o| {
                o.id != id
                    && shortcuts::keys_for(o, settings)
                        .iter()
                        .any(|x| shortcuts::same(x, &k))
            })?;
            Some((
                format!(
                    "{} is also the shortcut for {}. Change one of them.",
                    shortcuts::display(&k),
                    other.title
                ),
                true,
            ))
        });
        if let Some(view) = &mut self.settings {
            view.note = note;
        }
        cx.notify();
    }

    fn reset_all_shortcuts(&mut self, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.keys.clear());
        shortcuts::apply(cx);
        if let Some(view) = &mut self.settings {
            view.note = Some(("All shortcuts are back to their defaults.".into(), false));
        }
        cx.notify();
    }

    pub(super) fn render_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (page, focus) = self.settings.as_ref().map(|v| (v.page, v.focus.clone()))?;
        let t = cx.theme().clone();

        let tab = |id: &'static str, label: &'static str, target: Page, cx: &mut Context<Self>| {
            let on = page == target;
            div()
                .id(id)
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .cursor_pointer()
                .border_b_2()
                .border_color(if on {
                    t.accent
                } else {
                    gpui::transparent_black()
                })
                .text_color(if on { t.fg } else { t.muted })
                .when(on, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                .hover(|s| s.text_color(t.fg))
                .child(label)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open_settings(target, window, cx)),
                )
        };

        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .px(px(8.))
            .border_b_1()
            .border_color(t.border)
            .child(tab("settings-tab-general", "General", Page::General, cx))
            .child(tab(
                "settings-tab-keys",
                "Keyboard Shortcuts",
                Page::Shortcuts,
                cx,
            ))
            .child(div().flex_1())
            .child(icon_button(
                "settings-close",
                "close",
                t.muted,
                cx,
                cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
            ));

        let body = match page {
            Page::General => self.render_general(&t, cx),
            Page::Shortcuts => self.render_shortcuts(&t, cx),
        };

        let panel = div()
            .id("settings-panel")
            .track_focus(&focus)
            .key_context("Settings")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    this.close_settings(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(680.))
            .max_w_full()
            .h_full()
            .flex()
            .flex_col()
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .pb(px(16.))
                    .child(body),
            );

        Some(
            div()
                .id("settings-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .justify_center()
                .px(px(16.))
                .pt(px(48.))
                .pb(px(40.))
                .bg(gpui::black().opacity(0.25))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    fn render_general(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let env_theme = std::env::var("DRIFT_THEME").is_ok_and(|v| v == "light" || v == "dark");
        let env_updates = std::env::var("DRIFT_UPDATE_CHECK").is_ok_and(|v| v == "0" || v == "1");

        let seg =
            |id: &'static str, label: &'static str, choice: ThemeChoice, cx: &mut Context<Self>| {
                let on = settings.theme == choice;
                div()
                    .id(id)
                    .h(px(26.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.sel).text_color(t.sel_fg))
                    .when(!on, |d| d.text_color(t.muted).hover(|s| s.bg(t.hover)))
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.set_theme(choice, window, cx)),
                    )
            };
        let themes = div()
            .flex()
            .flex_none()
            .border_1()
            .border_color(t.border)
            .rounded(px(5.))
            .overflow_hidden()
            .child(seg("theme-system", "System", ThemeChoice::System, cx))
            .child(seg("theme-light", "Light", ThemeChoice::Light, cx))
            .child(seg("theme-dark", "Dark", ThemeChoice::Dark, cx));

        let auto = settings.check_updates;
        let toggle = div()
            .id("updates-auto")
            .w(px(34.))
            .h(px(18.))
            .flex_none()
            .rounded(px(9.))
            .p(px(2.))
            .flex()
            .when(auto, |d| d.justify_end())
            .bg(if auto { t.accent } else { t.border })
            .cursor_pointer()
            .child(div().size(px(14.)).rounded(px(7.)).bg(gpui::white()))
            .on_click(cx.listener(|_, _, _, cx| {
                crate::settings::update(cx, |s| s.check_updates = !s.check_updates);
            }));

        let current = env!("CARGO_PKG_VERSION");
        let update = cx.global::<crate::update::Available>().0.clone();
        let (status, status_color) = if self.checking_updates {
            ("Checking for updates…".to_string(), t.muted)
        } else if let Some(u) = &update {
            (format!("Drift {} is available.", u.version), t.accent)
        } else {
            ("Up to date as of the last check.".to_string(), t.muted)
        };
        let check_button = match update {
            Some(u) if !self.checking_updates => button("updates-download", "Download", true, t)
                .on_click(move |_, _, cx| cx.open_url(&u.url)),
            _ => button("updates-check", "Check Now", false, t)
                .when(self.checking_updates, |d| d.opacity(0.5))
                .when(!self.checking_updates, |d| {
                    d.on_click(
                        cx.listener(|this, _, window, cx| this.check_for_updates(window, cx)),
                    )
                }),
        };

        div()
            .child(section("APPEARANCE", t))
            .child(setting_row(
                "Theme",
                if env_theme {
                    "DRIFT_THEME is set, so new windows start with that theme."
                } else {
                    "System follows your computer's light or dark mode."
                },
                themes,
                t,
            ))
            .child(section("UPDATES", t))
            .child(setting_row(
                "Check for updates automatically",
                if env_updates {
                    "DRIFT_UPDATE_CHECK is set, which overrides this setting."
                } else if cfg!(debug_assertions) {
                    "Once a day. Development builds only check when you ask."
                } else {
                    "Once a day, Drift looks for a new release on GitHub."
                },
                toggle,
                t,
            ))
            .child(
                div()
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("Version {current}"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(status_color)
                                    .child(status),
                            ),
                    )
                    .child(check_button),
            )
            .into_any_element()
    }

    fn render_shortcuts(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let settings = cx.global::<Settings>().clone();
        let view = self.settings.as_ref();
        let recording = view.and_then(|v| v.recording);
        let note = view.and_then(|v| v.note.clone());
        let any_custom = SHORTCUTS.iter().any(|s| shortcuts::is_custom(s, &settings));

        let top = div()
            .pt(px(14.))
            .pb(px(6.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(t.muted)
                    .child("Click Change, then press the new shortcut. Esc cancels."),
            )
            .child(
                button("keys-reset-all", "Reset All", false, t)
                    .when(!any_custom, |d| d.opacity(0.5))
                    .when(any_custom, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.reset_all_shortcuts(cx)))
                    }),
            );
        let note = note.map(|(text, warn)| {
            div()
                .mb(px(6.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(4.))
                .text_size(px(12.))
                .when(warn, |d| d.bg(t.error.opacity(0.08)).text_color(t.error))
                .when(!warn, |d| d.bg(t.hover).text_color(t.fg))
                .child(text)
        });

        let rows: Vec<AnyElement> = SHORTCUTS
            .iter()
            .map(|s| self.shortcut_row(s, &settings, recording == Some(s.id), t, cx))
            .collect();

        div()
            .child(top)
            .children(note)
            .child(div().border_t_1().border_color(t.border).children(rows))
            .into_any_element()
    }

    fn shortcut_row(
        &mut self,
        s: &'static Shortcut,
        settings: &Settings,
        recording: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let keys = shortcuts::keys_for(s, settings);
        let custom = shortcuts::is_custom(s, settings);
        let clash = keys.iter().find_map(|k| {
            SHORTCUTS.iter().find(|o| {
                o.id != s.id
                    && shortcuts::keys_for(o, settings)
                        .iter()
                        .any(|x| shortcuts::same(x, k))
            })
        });
        let id = s.id;

        let chips: AnyElement = if recording {
            div()
                .px(px(8.))
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(t.accent)
                .text_size(px(12.))
                .text_color(t.accent)
                .child("Press a shortcut…")
                .into_any_element()
        } else if keys.is_empty() {
            div()
                .text_size(px(12.))
                .text_color(t.faint)
                .child("None")
                .into_any_element()
        } else {
            div()
                .flex()
                .gap(px(6.))
                .children(keys.iter().map(|k| key_chip(shortcuts::display(k), t)))
                .into_any_element()
        };

        let change = if recording {
            button(
                SharedString::from(format!("key-cancel-{id}")),
                "Cancel",
                false,
                t,
            )
            .on_click(cx.listener(|this, _, _, cx| this.stop_recording(cx)))
        } else {
            button(
                SharedString::from(format!("key-change-{id}")),
                "Change",
                false,
                t,
            )
            .on_click(cx.listener(move |this, _, window, cx| this.start_recording(id, window, cx)))
        };

        let detail = match clash {
            Some(o) => Some((format!("Same shortcut as {}", o.title), t.error)),
            None if custom => Some(("Changed".to_string(), t.faint)),
            None => None,
        };
        div()
            .id(SharedString::from(format!("key-row-{id}")))
            .min_h(px(40.))
            .py(px(6.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.border)
            .when(recording, |d| d.bg(t.hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().overflow_hidden().text_ellipsis().child(s.title))
                    .children(detail.map(|(text, color)| {
                        div()
                            .text_size(px(11.))
                            .text_color(color)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(text)
                    })),
            )
            .child(div().w(px(170.)).flex_none().flex().child(chips))
            .child(
                div()
                    .w(px(176.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .child(change)
                    .when(!keys.is_empty() && !recording, |d| {
                        d.child(
                            button(
                                SharedString::from(format!("key-remove-{id}")),
                                "Remove",
                                false,
                                t,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set_shortcut(id, Some(String::new()), cx)
                                },
                            )),
                        )
                    })
                    .when(custom && !recording, |d| {
                        d.child(
                            button(
                                SharedString::from(format!("key-reset-{id}")),
                                "Reset",
                                false,
                                t,
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_shortcut(id, None, cx)),
                            ),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn section(title: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .pt(px(18.))
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(t.muted)
        .child(title)
}

fn setting_row(
    title: &'static str,
    detail: &'static str,
    control: impl IntoElement,
    t: &Theme,
) -> impl IntoElement {
    div()
        .py(px(10.))
        .flex()
        .items_center()
        .gap(px(12.))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(title)
                .child(div().text_size(px(12.)).text_color(t.muted).child(detail)),
        )
        .child(control)
}

fn button(
    id: impl Into<SharedString>,
    label: &'static str,
    primary: bool,
    t: &Theme,
) -> Stateful<gpui::Div> {
    let (bg, fg, border, hover): (Hsla, Hsla, Hsla, Hsla) = if primary {
        (t.accent, t.on_accent, t.accent, t.accent_hover)
    } else {
        (t.input, t.fg, t.border, t.hover)
    };
    div()
        .id(id.into())
        .h(px(24.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
}

fn key_chip(text: String, t: &Theme) -> impl IntoElement {
    div()
        .h(px(22.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(t.border)
        .bg(t.input)
        .text_size(px(12.))
        .child(text)
}

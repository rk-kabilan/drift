//! The bottom panel under the diff, as in VS Code: an Output tab with the log of
//! git commands, errors and warnings, and a Terminal tab with shells started on
//! first use. Cmd/Ctrl+J toggles it, Ctrl+` the terminal, Cmd/Ctrl+Shift+U
//! shows Output.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, uniform_list, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    MouseButton, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window,
};

use super::terminal_view::{TerminalEvent, TerminalView};
use super::widgets::{icon_button, icon_sized, tooltip};
use super::workspace::Workspace;
use crate::output::{self, Level};
pub use crate::settings::PanelTab;
use crate::theme::{ActiveTheme as _, Theme};

pub const DEFAULT_HEIGHT: f32 = crate::settings::DEFAULT_PANEL_HEIGHT;
const MIN_HEIGHT: f32 = 100.;
/// Height the diff keeps above the panel.
const DIFF_KEEP: f32 = 120.;
const HEADER: f32 = 34.;
const ROW: f32 = 19.;
/// How long a warning or error stays in the status bar.
const ALERT_SECS: u64 = 5;

gpui::actions!(output, [CopyLine]);

pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    vec![gpui::KeyBinding::new(
        "secondary-c",
        CopyLine,
        Some("Output"),
    )]
}

pub struct Panel {
    pub open: bool,
    pub tab: PanelTab,
    pub height: f32,
    pub maximized: bool,
    /// While the top edge is dragged: the pointer y and the height at the start.
    pub drag: Option<(f32, f32)>,
    /// Output shows only errors and warnings.
    errors_only: bool,
    output_focus: FocusHandle,
    output_scroll: UniformListScrollHandle,
    /// Output rows as (entry seq, line of its text), rebuilt each frame.
    rows: Vec<(u64, u16)>,
    /// The Output list was scrolled to its end in the last frame.
    at_end: bool,
    /// The clicked Output row.
    selected: Option<(u64, u16)>,
    /// Errors before this seq have been seen in the Output tab.
    seen: u64,
    pub terminals: Vec<Entity<TerminalView>>,
    pub active_terminal: usize,
    /// Why the last terminal failed to start.
    terminal_error: Option<String>,
    /// A terminal had focus when the panel was last drawn.
    terminal_had_focus: bool,
    terminal_subs: Vec<Subscription>,
    /// A warning or error shown in the status bar for a few seconds.
    pub alert: Option<(Level, SharedString)>,
    alert_generation: u64,
    /// The last status read failure, so the 3-second refresh reports it once.
    pub refresh_error: Option<String>,
}

impl Panel {
    pub fn new(cx: &mut gpui::App) -> Self {
        let s = cx.global::<crate::settings::Settings>();
        Self {
            open: s.panel_open,
            tab: s.panel_tab,
            height: s.panel_height,
            maximized: false,
            drag: None,
            errors_only: false,
            output_focus: cx.focus_handle(),
            output_scroll: Default::default(),
            rows: Vec::new(),
            at_end: true,
            selected: None,
            seen: output::lock().next_seq(),
            terminals: Vec::new(),
            active_terminal: 0,
            terminal_error: None,
            terminal_had_focus: false,
            terminal_subs: Vec::new(),
            alert: None,
            alert_generation: 0,
            refresh_error: None,
        }
    }

    fn save(&self, cx: &mut gpui::App) {
        let (open, tab, height) = (self.open, self.tab, self.height);
        crate::settings::update(cx, |s| {
            s.panel_open = open;
            s.panel_tab = tab;
            s.panel_height = height;
        });
    }
}

impl Workspace {
    // ---- reporting ----

    /// Logs a failed operation in Output, opens the panel there (without
    /// taking focus) and shows a note in the status bar.
    pub(super) fn report_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        output::push(Level::Error, self.root_path(), &message);
        // Don't pull the panel away from a terminal being typed in.
        let typing = self.panel.open
            && self.panel.tab == PanelTab::Terminal
            && self.panel.terminal_had_focus;
        if !typing {
            self.panel.open = true;
            self.panel.tab = PanelTab::Output;
            self.panel.maximized = false;
        }
        self.alert(Level::Error, first_line(&message), cx);
    }

    /// Logs something the user asked for that can't go ahead ("Nothing to
    /// commit"), with a note in the status bar.
    pub(super) fn report_warning(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        let message = message.into();
        output::push(Level::Warning, self.root_path(), &message);
        self.alert(Level::Warning, first_line(&message), cx);
    }

    fn root_path(&self) -> Option<&std::path::Path> {
        self.repo.is_some().then_some(self.root.as_path())
    }

    fn alert(&mut self, level: Level, text: String, cx: &mut Context<Self>) {
        self.panel.alert_generation += 1;
        let generation = self.panel.alert_generation;
        self.panel.alert = Some((level, text.into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(ALERT_SECS))
                .await;
            this.update(cx, |this, cx| {
                if this.panel.alert_generation == generation {
                    this.panel.alert = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Errors in Output the user hasn't looked at yet.
    pub(super) fn unseen_errors(&self) -> usize {
        let log = output::lock();
        log.entries()
            .iter()
            .rev()
            .take_while(|e| e.seq >= self.panel.seen)
            .filter(|e| e.level == Level::Error && output::belongs(e, &self.root))
            .count()
    }

    // ---- panel actions ----

    fn terminal_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.panel
            .terminals
            .iter()
            .any(|t| t.read(cx).focus_handle(cx).is_focused(window))
    }

    fn set_panel_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !open {
            // Focus goes back to the commit message when it was in the panel.
            let in_panel =
                self.terminal_focused(window, cx) || self.panel.output_focus.is_focused(window);
            self.panel.maximized = false;
            if in_panel {
                window.focus(&self.message.focus_handle(cx));
            }
        }
        self.panel.open = open;
        self.panel.save(cx);
        cx.notify();
    }

    pub(super) fn toggle_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        if self.panel.open {
            self.set_panel_open(false, window, cx);
        } else {
            self.set_panel_open(true, window, cx);
            match self.panel.tab {
                PanelTab::Terminal => self.focus_terminal(window, cx),
                PanelTab::Output => window.focus(&self.panel.output_focus),
            }
        }
    }

    /// Ctrl+`: opens the terminal, focuses it, or hides the panel when the
    /// terminal already has focus.
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        if self.panel.open
            && self.panel.tab == PanelTab::Terminal
            && self.terminal_focused(window, cx)
        {
            self.set_panel_open(false, window, cx);
            return;
        }
        self.panel.tab = PanelTab::Terminal;
        self.set_panel_open(true, window, cx);
        self.focus_terminal(window, cx);
    }

    pub(super) fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        self.panel.tab = PanelTab::Terminal;
        self.set_panel_open(true, window, cx);
        if self.add_terminal(window, cx) {
            self.focus_terminal(window, cx);
        }
    }

    pub(super) fn show_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() {
            return;
        }
        self.panel.tab = PanelTab::Output;
        self.set_panel_open(true, window, cx);
        self.panel.at_end = true;
        window.focus(&self.panel.output_focus);
    }

    pub(super) fn clear_output(&mut self, cx: &mut Context<Self>) {
        output::clear(&self.root);
        self.panel.selected = None;
        cx.notify();
    }

    pub(super) fn toggle_maximized_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panel.open {
            self.set_panel_open(true, window, cx);
        }
        self.panel.maximized = !self.panel.maximized;
        cx.notify();
    }

    pub(super) fn kill_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.terminals.is_empty() {
            return;
        }
        let ix = self.panel.active_terminal;
        self.remove_terminal(ix, window, cx);
    }

    pub(super) fn cycle_terminal(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.panel.terminals.len();
        if n < 2 {
            return;
        }
        self.panel.active_terminal =
            (self.panel.active_terminal as isize + delta).rem_euclid(n as isize) as usize;
        self.focus_terminal(window, cx);
    }

    fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.terminals.is_empty() && !self.add_terminal(window, cx) {
            return;
        }
        if let Some(t) = self.panel.terminals.get(self.panel.active_terminal) {
            window.focus(&t.read(cx).focus_handle(cx));
        }
        cx.notify();
    }

    /// Starts a shell in the repository root. False (and the reason in the
    /// panel and Output) when it can't start.
    fn add_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let shell = cx
            .global::<crate::settings::Settings>()
            .terminal_shell
            .clone();
        let view = match TerminalView::start(&self.root, shell.as_deref(), window, cx) {
            Ok(view) => view,
            Err(e) => {
                let message = format!("The terminal could not start: {e:#}");
                self.panel.terminal_error = Some(message.clone());
                output::push(Level::Error, self.root_path(), &message);
                cx.notify();
                return false;
            }
        };
        self.panel.terminal_error = None;
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, view, e: &TerminalEvent, window, cx| match e {
                TerminalEvent::Exited => {
                    if let Some(ix) = this.panel.terminals.iter().position(|t| t == view) {
                        this.remove_terminal(ix, window, cx);
                    }
                }
                TerminalEvent::TitleChanged => cx.notify(),
            },
        );
        self.panel.terminals.push(view);
        self.panel.terminal_subs.push(sub);
        self.panel.active_terminal = self.panel.terminals.len() - 1;
        cx.notify();
        true
    }

    fn remove_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.panel.terminals.len() {
            return;
        }
        let had_focus = self.terminal_focused(window, cx);
        // Dropping the view ends the shell and frees its screen and scrollback.
        self.panel.terminals.remove(ix);
        drop(self.panel.terminal_subs.remove(ix));
        let n = self.panel.terminals.len();
        if n == 0 {
            self.panel.active_terminal = 0;
            if self.panel.tab == PanelTab::Terminal && self.panel.open {
                self.set_panel_open(false, window, cx);
            }
            if had_focus {
                window.focus(&self.message.focus_handle(cx));
            }
        } else {
            self.panel.active_terminal = self.panel.active_terminal.min(n - 1);
            if had_focus {
                self.focus_terminal(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn drag_panel(&mut self, y: f32, window: &Window, cx: &mut Context<Self>) {
        let Some((start_y, start_h)) = self.panel.drag else {
            return;
        };
        let max = self.max_panel_height(window);
        self.panel.height = (start_h - (y - start_y)).clamp(MIN_HEIGHT, max);
        self.panel.maximized = false;
        cx.notify();
    }

    pub(super) fn end_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.panel.drag.take().is_some() {
            self.panel.save(cx);
            cx.notify();
        }
    }

    fn max_panel_height(&self, window: &Window) -> f32 {
        // Title bar, status bar and the diff's share.
        (f32::from(window.viewport_size().height) - 70. - DIFF_KEEP).max(MIN_HEIGHT)
    }

    // ---- rendering ----

    pub(super) fn render_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = cx.theme().clone();
        self.panel.terminal_had_focus = self.terminal_focused(window, cx);
        if self.panel.tab == PanelTab::Output {
            self.panel.seen = output::lock().next_seq();
        } else if self.panel.terminals.is_empty() && self.panel.terminal_error.is_none() {
            // The saved tab was Terminal: start the shell now that it shows.
            self.add_terminal(window, cx);
        }
        let height = self.panel.height.min(self.max_panel_height(window));
        let body = match self.panel.tab {
            PanelTab::Output => self.render_output(&t, cx).into_any_element(),
            PanelTab::Terminal => self.render_terminals(&t, cx).into_any_element(),
        };
        div()
            .id("panel")
            .relative()
            .flex()
            .flex_col()
            .when(self.panel.maximized, |d| d.flex_1().min_h_0())
            .when(!self.panel.maximized, |d| d.h(px(height)).flex_none())
            .bg(t.editor)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .overflow_hidden()
            .child(self.render_panel_header(&t, cx))
            .child(div().flex_1().min_h_0().flex().child(body))
            .into_any_element()
    }

    /// The strip above the panel that drags its height.
    pub(super) fn render_panel_grip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let line = cx.theme().accent;
        let active = self.panel.drag.is_some();
        div()
            .id("panel-grip")
            .group("panel-grip")
            .h(px(6.))
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .justify_center()
            .cursor_row_resize()
            .child(
                div()
                    .h(px(2.))
                    .w_full()
                    .rounded(px(1.))
                    .when(active, |d| d.bg(line))
                    .group_hover("panel-grip", |s| s.bg(line.opacity(0.6))),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if e.click_count >= 2 {
                        this.panel.height = DEFAULT_HEIGHT;
                        this.panel.maximized = false;
                        this.panel.drag = None;
                        this.panel.save(cx);
                    } else {
                        let start = if this.panel.maximized {
                            // Dragging a maximized panel starts from its full height.
                            this.panel.maximized = false;
                            this.panel.height.max(DEFAULT_HEIGHT)
                        } else {
                            this.panel.height
                        };
                        this.panel.drag = Some((f32::from(e.position.y), start));
                    }
                    cx.notify();
                }),
            )
    }

    fn render_panel_header(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let unseen = if self.panel.tab == PanelTab::Output {
            0
        } else {
            self.unseen_errors()
        };
        let tab = |id: &'static str, label: &'static str, which: PanelTab, hint: String| {
            let on = self.panel.tab == which;
            div()
                .id(id)
                .h(px(HEADER))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if on { t.fg } else { t.muted })
                .border_b_1()
                .border_color(if on { t.fg } else { gpui::transparent_black() })
                .hover(|s| s.text_color(t.fg))
                .child(label)
                .tooltip(tooltip(hint))
        };
        let maximized = self.panel.maximized;
        let fg = t.muted;
        let mut actions = div().flex().items_center().gap(px(2.));
        match self.panel.tab {
            PanelTab::Output => {
                let errors_only = self.panel.errors_only;
                actions = actions
                    .child(
                        icon_button(
                            "out-filter",
                            "filter",
                            if errors_only { t.accent } else { fg },
                            cx,
                            {
                                let ws = cx.entity().downgrade();
                                move |_, _, cx| {
                                    ws.update(cx, |this, cx| {
                                        this.panel.errors_only = !this.panel.errors_only;
                                        cx.notify();
                                    })
                                    .ok();
                                }
                            },
                        )
                        .when(errors_only, |d| d.bg(t.sel))
                        .tooltip(tooltip(if errors_only {
                            "Showing errors and warnings. Click to show everything."
                        } else {
                            "Show only errors and warnings"
                        })),
                    )
                    .child(
                        icon_button("out-copy", "copy", fg, cx, {
                            let ws = cx.entity().downgrade();
                            move |_, _, cx| {
                                ws.update(cx, |this, cx| this.copy_output(None, cx)).ok();
                            }
                        })
                        .tooltip(tooltip("Copy Output")),
                    )
                    .child(
                        icon_button("out-clear", "trash", fg, cx, {
                            let ws = cx.entity().downgrade();
                            move |_, _, cx| {
                                ws.update(cx, |this, cx| this.clear_output(cx)).ok();
                            }
                        })
                        .tooltip(tooltip("Clear Output")),
                    );
            }
            PanelTab::Terminal => {
                actions = actions
                    .child(
                        icon_button("term-new", "plus", fg, cx, {
                            let ws = cx.entity().downgrade();
                            move |_, window, cx| {
                                ws.update(cx, |this, cx| this.new_terminal(window, cx)).ok();
                            }
                        })
                        .tooltip(tooltip(format!(
                            "New Terminal{}",
                            super::shortcuts::hint("NewTerminal", cx)
                        ))),
                    )
                    .when(!self.panel.terminals.is_empty(), |d| {
                        d.child(
                            icon_button("term-kill", "trash", fg, cx, {
                                let ws = cx.entity().downgrade();
                                move |_, window, cx| {
                                    ws.update(cx, |this, cx| this.kill_terminal(window, cx))
                                        .ok();
                                }
                            })
                            .tooltip(tooltip("Kill Terminal")),
                        )
                    });
            }
        }
        actions = actions
            .child(div().w(px(1.)).h(px(14.)).mx(px(4.)).bg(t.border))
            .child(
                icon_button(
                    "panel-max",
                    if maximized {
                        "chevron-down"
                    } else {
                        "chevron-up"
                    },
                    fg,
                    cx,
                    {
                        let ws = cx.entity().downgrade();
                        move |_, window, cx| {
                            ws.update(cx, |this, cx| this.toggle_maximized_panel(window, cx))
                                .ok();
                        }
                    },
                )
                .tooltip(tooltip(if maximized {
                    "Restore Panel Size"
                } else {
                    "Maximize Panel"
                })),
            )
            .child(
                icon_button("panel-close", "close", fg, cx, {
                    let ws = cx.entity().downgrade();
                    move |_, window, cx| {
                        ws.update(cx, |this, cx| this.set_panel_open(false, window, cx))
                            .ok();
                    }
                })
                .tooltip(tooltip(format!(
                    "Hide Panel{}",
                    super::shortcuts::hint("TogglePanel", cx)
                ))),
            );

        let badge_bg = t.error;
        let badge_fg = t.editor;
        div()
            .h(px(HEADER))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(6.))
            .pr(px(6.))
            .child(
                tab(
                    "tab-output",
                    "OUTPUT",
                    PanelTab::Output,
                    format!("Output{}", super::shortcuts::hint("ShowOutput", cx)),
                )
                .when(unseen > 0, |d| {
                    d.child(
                        div()
                            .px(px(5.))
                            .rounded(px(8.))
                            .bg(badge_bg)
                            .text_color(badge_fg)
                            .text_size(px(10.))
                            .line_height(px(15.))
                            .child(unseen.min(99).to_string()),
                    )
                })
                .on_click(cx.listener(|this, _, window, cx| this.show_output(window, cx))),
            )
            .child(
                tab(
                    "tab-terminal",
                    "TERMINAL",
                    PanelTab::Terminal,
                    format!("Terminal{}", super::shortcuts::hint("ToggleTerminal", cx)),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.panel.tab = PanelTab::Terminal;
                    this.panel.save(cx);
                    this.focus_terminal(window, cx);
                })),
            )
            .child(div().flex_1())
            .child(actions)
    }

    /// Copies one Output entry, or all shown entries with `None`.
    fn copy_output(&mut self, seq: Option<u64>, cx: &mut Context<Self>) {
        let log = output::lock();
        let errors_only = self.panel.errors_only;
        let text = log
            .entries()
            .iter()
            .filter(|e| output::belongs(e, &self.root))
            .filter(|e| match seq {
                Some(s) => e.seq == s,
                None => !errors_only || e.level != Level::Info,
            })
            .map(|e| format!("[{}] {}", output::clock(e.time), e.text))
            .collect::<Vec<_>>()
            .join("\n");
        drop(log);
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn render_output(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Which rows show: one per line of each entry of this repository.
        let mut rows = std::mem::take(&mut self.panel.rows);
        rows.clear();
        {
            let log = output::lock();
            for e in log.entries() {
                if !output::belongs(e, &self.root)
                    || (self.panel.errors_only && e.level == Level::Info)
                {
                    continue;
                }
                let lines = e.text.lines().count().clamp(1, u16::MAX as usize);
                rows.extend((0..lines).map(|l| (e.seq, l as u16)));
            }
        }
        let count = rows.len();
        // Follow new lines while the list is at its end.
        let scroll = self.panel.output_scroll.clone();
        {
            let state = scroll.0.borrow();
            let base = &state.base_handle;
            let max = f32::from(base.max_offset().height);
            let at = -f32::from(base.offset().y);
            if max > 0. {
                self.panel.at_end = at >= max - ROW;
            }
        }
        if self.panel.at_end && count > 0 {
            scroll.scroll_to_item(count - 1, ScrollStrategy::Bottom);
        }
        self.panel.rows = rows;

        if count == 0 {
            let note = if self.panel.errors_only {
                "No errors or warnings."
            } else {
                "Git commands Drift runs, and any errors, show here."
            };
            return div()
                .id("output-empty")
                .key_context("Output")
                .track_focus(&self.panel.output_focus)
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.muted)
                .child(note)
                .into_any_element();
        }

        let mono = cx.global::<crate::theme::ThemeState>().mono_font.clone();
        div()
            .id("output")
            .key_context("Output")
            .track_focus(&self.panel.output_focus)
            .on_action(cx.listener(|this, _: &CopyLine, _, cx| {
                if let Some((seq, _)) = this.panel.selected {
                    this.copy_output(Some(seq), cx);
                }
            }))
            .flex_1()
            .min_w_0()
            .h_full()
            .font_family(mono)
            .text_size(px(12.))
            .child(
                uniform_list(
                    "output-rows",
                    count,
                    cx.processor(move |this, range: Range<usize>, _window, cx| {
                        let t = cx.theme().clone();
                        this.output_rows(range, &t, cx)
                    }),
                )
                .track_scroll(scroll)
                .size_full()
                .pb(px(6.)),
            )
            .into_any_element()
    }

    fn output_rows(
        &mut self,
        range: Range<usize>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let log = output::lock();
        let entries = log.entries();
        let mut out = Vec::with_capacity(range.len());
        for ix in range {
            let Some(&(seq, line)) = self.panel.rows.get(ix) else {
                break;
            };
            let Ok(pos) = entries.binary_search_by_key(&seq, |e| e.seq) else {
                out.push(div().h(px(ROW)).into_any_element());
                continue;
            };
            let e = &entries[pos];
            let text = e.text.lines().nth(line as usize).unwrap_or("");
            let color = match e.level {
                Level::Info => t.fg,
                Level::Warning => t.st_modified,
                Level::Error => t.error,
            };
            let first = line == 0;
            let selected = self.panel.selected == Some((seq, line));
            let (icon, rest) = match e.level {
                Level::Error if first => (Some("error"), text),
                Level::Warning if first => (Some("warning"), text),
                _ => (None, text),
            };
            // "> git status [38 ms]": the timing is quieter than the command.
            let (body, timing) = match (e.level, rest.rfind(" [")) {
                (Level::Info, Some(i)) if rest.ends_with(']') => (&rest[..i], Some(&rest[i..])),
                _ => (rest, None),
            };
            out.push(
                div()
                    .id(("out-row", ix))
                    .h(px(ROW))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .whitespace_nowrap()
                    .when(selected, |d| d.bg(t.sel))
                    .child(
                        div()
                            .w(px(60.))
                            .flex_none()
                            .text_color(t.faint)
                            .child(if first {
                                output::clock(e.time)
                            } else {
                                String::new()
                            }),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .children(icon.map(|i| icon_sized(i, 13., color))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .text_color(color)
                            .child(body.to_string())
                            .children(
                                timing
                                    .map(|s| div().text_color(t.faint).child(s.trim().to_string())),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.panel.selected = Some((seq, line));
                        window.focus(&this.panel.output_focus);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    fn render_terminals(&mut self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = self
            .panel
            .terminals
            .get(self.panel.active_terminal)
            .cloned()
        else {
            let note = self
                .panel
                .terminal_error
                .clone()
                .unwrap_or_else(|| "Starting the shell…".to_string());
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.muted)
                .child(note)
                .into_any_element();
        };
        let list = (self.panel.terminals.len() > 1).then(|| {
            div()
                .w(px(150.))
                .flex_none()
                .border_l_1()
                .border_color(t.border)
                .py(px(4.))
                .children(self.panel.terminals.iter().enumerate().map(|(i, view)| {
                    let on = i == self.panel.active_terminal;
                    let title = view.read(cx).title.clone();
                    div()
                        .id(("term-item", i))
                        .group("term-item")
                        .h(px(22.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .when(on, |d| d.bg(t.sel))
                        .hover(|s| s.bg(t.hover))
                        .child(icon_sized("terminal", 13., t.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .child(title.clone()),
                        )
                        .tooltip(tooltip(title))
                        .child(
                            div()
                                .invisible()
                                .group_hover("term-item", |s| s.visible())
                                .child(icon_button(("term-x", i), "close", t.muted, cx, {
                                    let ws = cx.entity().downgrade();
                                    move |_, window, cx| {
                                        ws.update(cx, |this, cx| {
                                            this.remove_terminal(i, window, cx)
                                        })
                                        .ok();
                                    }
                                })),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.panel.active_terminal = i;
                            this.focus_terminal(window, cx);
                        }))
                }))
        });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .child(div().flex_1().min_w_0().pb(px(4.)).child(active))
            .children(list)
            .into_any_element()
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

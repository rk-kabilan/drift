//! The Terminal tab's view: draws a [`Terminal`]'s screen, and turns keys,
//! mouse selection, scrolling and paste into input for the shell.
//!
//! Each visible row becomes one line of styled text (cells of the same colours
//! joined into runs), so only what is on screen is laid out. Nothing is kept
//! between frames besides the terminal's own grid.

use std::cell::Cell;
use std::rc::Rc;

use alacritty_terminal::event::Event;
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{point_to_viewport, viewport_to_point, TermMode};
use alacritty_terminal::vte::ansi::{
    ClearMode, Color as AnsiColor, CursorShape, Handler as _, NamedColor, Rgb,
};
use gpui::{
    actions, canvas, div, prelude::*, px, App, Bounds, ClipboardItem, Context, EventEmitter,
    FocusHandle, Focusable, FontStyle, FontWeight, Hsla, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, ScrollWheelEvent, SharedString, StrikethroughStyle,
    StyledText, TextRun, UnderlineStyle, Window,
};

use crate::terminal::{window_size, Terminal};
use crate::theme::{ActiveTheme as _, Appearance, Theme, ThemeState};

actions!(
    terminal,
    [
        Copy,
        Paste,
        Clear,
        ScrollToTop,
        ScrollToBottom,
        ScrollPageUp,
        ScrollPageDown
    ]
);

/// Key bindings that only apply while a terminal has focus.
pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    let ctx = Some("Terminal");
    let mut keys = vec![
        KeyBinding::new("shift-pageup", ScrollPageUp, ctx),
        KeyBinding::new("shift-pagedown", ScrollPageDown, ctx),
    ];
    if cfg!(target_os = "macos") {
        keys.extend([
            KeyBinding::new("cmd-c", Copy, ctx),
            KeyBinding::new("cmd-v", Paste, ctx),
            KeyBinding::new("cmd-k", Clear, ctx),
            KeyBinding::new("cmd-up", ScrollToTop, ctx),
            KeyBinding::new("cmd-down", ScrollToBottom, ctx),
        ]);
    } else {
        keys.extend([
            KeyBinding::new("ctrl-shift-c", Copy, ctx),
            KeyBinding::new("ctrl-shift-v", Paste, ctx),
            KeyBinding::new("ctrl-home", ScrollToTop, ctx),
            KeyBinding::new("ctrl-end", ScrollToBottom, ctx),
        ]);
    }
    keys
}

pub const FONT_SIZE: f32 = 13.;
/// Space left of the first column.
const PAD: f32 = 10.;

pub enum TerminalEvent {
    /// The shell exited; the view can be closed.
    Exited,
    TitleChanged,
}

pub struct TerminalView {
    terminal: Terminal,
    /// The shell's own title (OSC 0/2), or the shell's name.
    pub title: SharedString,
    default_title: SharedString,
    focus: FocusHandle,
    /// Where the grid was painted, measured each frame.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// One cell's width and height.
    cell: (f32, f32),
    /// The left button is down and dragging a selection.
    selecting: bool,
    /// Scroll distance not yet a whole line.
    scroll_rest: f32,
    _events: gpui::Task<()>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    /// Starts a shell in `cwd`. Fails when the pty or the shell can't start.
    pub fn start(
        cwd: &std::path::Path,
        shell: Option<&str>,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<gpui::Entity<Self>> {
        let mono = cx.global::<ThemeState>().mono_font.clone();
        let ts = window.text_system();
        let font_id = ts.resolve_font(&gpui::font(mono));
        let cell_w = ts
            .advance(font_id, px(FONT_SIZE), 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(FONT_SIZE * 0.6);
        let cell_h = (FONT_SIZE * 1.4).round();
        let terminal = Terminal::spawn(cwd, shell, window_size(80, 24, cell_w, cell_h))?;
        let shell = shell.map(str::to_string);
        Ok(cx.new(|cx| Self::new(terminal, shell.as_deref(), (cell_w, cell_h), cx)))
    }

    fn new(
        terminal: Terminal,
        shell: Option<&str>,
        cell: (f32, f32),
        cx: &mut Context<Self>,
    ) -> Self {
        let rx = terminal.events.clone();
        let events = cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                // Handle everything that arrived together, then draw once.
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let alive = this
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.handle_event(event, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });

        let default_title: SharedString = shell
            .and_then(|s| s.split_whitespace().next())
            .map(str::to_string)
            .or_else(|| {
                if cfg!(windows) {
                    Some("powershell".to_string())
                } else {
                    std::env::var("SHELL").ok()
                }
            })
            .map(|s| {
                let name = s.rsplit(['/', '\\']).next().unwrap_or(&s);
                name.trim_end_matches(".exe").to_string()
            })
            .unwrap_or_else(|| "shell".into())
            .into();
        Self {
            terminal,
            title: default_title.clone(),
            default_title,
            focus: cx.focus_handle(),
            bounds: Default::default(),
            cell,
            selecting: false,
            scroll_rest: 0.,
            _events: events,
        }
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Title(title) => {
                self.title = title.into();
                cx.emit(TerminalEvent::TitleChanged);
            }
            Event::ResetTitle => {
                self.title = self.default_title.clone();
                cx.emit(TerminalEvent::TitleChanged);
            }
            Event::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            Event::ClipboardLoad(_, format) => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .unwrap_or_default();
                self.terminal.write(format(&text).into_bytes());
            }
            Event::ColorRequest(index, format) => {
                let theme = cx.theme().clone();
                let rgb = to_rgb(index_color(index, &theme));
                self.terminal.write(format(rgb).into_bytes());
            }
            Event::PtyWrite(text) => self.terminal.write(text.into_bytes()),
            Event::TextAreaSizeRequest(format) => {
                self.terminal
                    .write(format(self.terminal.size()).into_bytes());
            }
            Event::ChildExit(_) | Event::Exit => cx.emit(TerminalEvent::Exited),
            Event::Wakeup | Event::Bell | Event::MouseCursorDirty | Event::CursorBlinkingChange => {
            }
        }
    }

    fn write(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        {
            let mut term = self.terminal.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.terminal.write(bytes);
    }

    fn copy(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.terminal.term.lock().selection_to_string();
        match text {
            Some(text) if !text.is_empty() => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            _ => false,
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let bracketed = self
            .terminal
            .term
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        let bytes = if bracketed {
            // An escape inside the text could end the paste early.
            let clean = text.replace('\x1b', "");
            format!("\x1b[200~{clean}\x1b[201~").into_bytes()
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
        };
        self.write(bytes);
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        {
            let mut term = self.terminal.term.lock();
            term.clear_screen(ClearMode::Saved);
        }
        // Ctrl+L: the shell clears the screen and redraws its prompt.
        self.write(vec![0x0c]);
        cx.notify();
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.terminal.term.lock().scroll_display(scroll);
        cx.notify();
    }

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        let m = &ks.modifiers;
        // Windows terminals copy with Ctrl+C when text is selected, and paste with Ctrl+V.
        if cfg!(windows) && m.control && !m.shift && !m.alt {
            if ks.key == "c" && self.copy(cx) {
                self.terminal.term.lock().selection = None;
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if ks.key == "v" {
                self.paste(cx);
                cx.stop_propagation();
                return;
            }
        }
        let mode = *self.terminal.term.lock().mode();
        if let Some(bytes) = keystroke_bytes(ks, mode) {
            self.write(bytes);
            cx.stop_propagation();
            window.prevent_default();
            cx.notify();
        }
    }

    /// The grid point under a window position.
    fn point_at(&self, pos: gpui::Point<Pixels>) -> Option<(Point, Side)> {
        let bounds = self.bounds.get()?;
        let term = self.terminal.term.lock();
        let (cw, ch) = self.cell;
        let x = (f32::from(pos.x - bounds.origin.x) - PAD).max(0.);
        let y = f32::from(pos.y - bounds.origin.y).max(0.);
        let cols = term.columns();
        let col = ((x / cw) as usize).min(cols.saturating_sub(1));
        let row = ((y / ch) as usize).min(term.screen_lines().saturating_sub(1));
        let side = if x / cw - col as f32 > 0.5 {
            Side::Right
        } else {
            Side::Left
        };
        let offset = term.grid().display_offset();
        Some((
            viewport_to_point(offset, Point::new(row, Column(col))),
            side,
        ))
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let Some((point, side)) = self.point_at(e.position) else {
            return;
        };
        let ty = match e.click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let mut term = self.terminal.term.lock();
        if e.modifiers.shift && term.selection.is_some() {
            if let Some(sel) = term.selection.as_mut() {
                sel.update(point, side);
            }
        } else {
            let mut sel = Selection::new(ty, point, side);
            if ty != SelectionType::Simple {
                sel.include_all();
            }
            term.selection = Some(sel);
        }
        drop(term);
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            self.selecting = false;
            return;
        }
        if let Some((point, side)) = self.point_at(e.position) {
            if let Some(sel) = self.terminal.term.lock().selection.as_mut() {
                sel.update(point, side);
            }
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.selecting = false;
        // A click without a drag leaves an empty selection; drop it.
        let mut term = self.terminal.term.lock();
        if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
            term.selection = None;
        }
        drop(term);
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let ch = self.cell.1;
        self.scroll_rest += f32::from(e.delta.pixel_delta(px(ch)).y);
        let lines = (self.scroll_rest / ch).trunc();
        if lines == 0. {
            return;
        }
        self.scroll_rest -= lines * ch;
        let mode = *self.terminal.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs (less, vim) get arrow keys instead.
            let key: &[u8] = match (lines > 0., mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes = key.repeat(lines.abs() as usize);
            self.terminal.write(bytes);
        } else {
            self.terminal
                .term
                .lock()
                .scroll_display(Scroll::Delta(lines as i32));
        }
        cx.notify();
    }

    /// Fits the terminal to the space it was last painted in.
    fn fit(&mut self) {
        let Some(bounds) = self.bounds.get() else {
            return;
        };
        let (cw, ch) = self.cell;
        let cols = ((f32::from(bounds.size.width) - PAD * 2.) / cw).floor() as usize;
        let rows = (f32::from(bounds.size.height) / ch).floor() as usize;
        self.terminal.resize(window_size(cols, rows, cw, ch));
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit();
        let t = cx.theme().clone();
        let mono: SharedString = cx.global::<ThemeState>().mono_font.clone().into();
        let focused = self.focus.is_focused(window);
        let (cw, ch) = self.cell;

        let base = gpui::font(mono.clone());
        let bold = gpui::Font {
            weight: FontWeight::BOLD,
            ..base.clone()
        };
        let italic = gpui::Font {
            style: FontStyle::Italic,
            ..base.clone()
        };
        let bold_italic = gpui::Font {
            weight: FontWeight::BOLD,
            ..italic.clone()
        };

        let mut term = self.terminal.term.lock();
        term.is_focused = focused;
        let content = term.renderable_content();
        let offset = content.display_offset;
        let screen_rows = self.terminal.size().num_lines as usize;
        let mut rows: Vec<(String, Vec<TextRun>)> = vec![Default::default(); screen_rows];
        let cursor = content.cursor;
        let cursor_at =
            point_to_viewport(offset, cursor.point).filter(|_| cursor.shape != CursorShape::Hidden);
        let block_cursor = focused && cursor.shape == CursorShape::Block;
        let selection = content.selection;
        let colors = content.colors;
        for item in content.display_iter {
            let cell = item.cell;
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let Some(vp) = point_to_viewport(offset, item.point) else {
                continue;
            };
            let Some((text, runs)) = rows.get_mut(vp.line) else {
                continue;
            };
            let mut fg = resolve(cell.fg, colors, &t, true);
            let mut bg = match cell.bg {
                AnsiColor::Named(NamedColor::Background) => None,
                c => Some(resolve(c, colors, &t, false)),
            };
            if cell.flags.contains(Flags::DIM) {
                fg = fg.opacity(0.7);
            }
            if cell.flags.contains(Flags::INVERSE) {
                let back = bg.unwrap_or(t.editor);
                bg = Some(fg);
                fg = back;
            }
            if selection.is_some_and(|s| s.contains(item.point)) {
                bg = Some(t.sel);
                fg = if t.appearance == Appearance::Dark {
                    t.sel_fg
                } else {
                    fg
                };
            }
            if block_cursor && cursor_at == Some(vp) {
                let back = bg.unwrap_or(t.editor);
                bg = Some(t.fg);
                fg = back;
            }
            let font = match (
                cell.flags.contains(Flags::BOLD),
                cell.flags.contains(Flags::ITALIC),
            ) {
                (false, false) => &base,
                (true, false) => &bold,
                (false, true) => &italic,
                (true, true) => &bold_italic,
            };
            let underline = cell
                .flags
                .intersects(Flags::ALL_UNDERLINES)
                .then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(fg),
                    wavy: cell.flags.contains(Flags::UNDERCURL),
                });
            let strikethrough = cell
                .flags
                .contains(Flags::STRIKEOUT)
                .then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(fg),
                });
            let start = text.len();
            if cell.flags.contains(Flags::HIDDEN) || cell.c == '\t' || cell.c < ' ' {
                text.push(' ');
            } else {
                text.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra.iter());
                }
            }
            let len = text.len() - start;
            match runs.last_mut() {
                Some(last)
                    if last.color == fg
                        && last.background_color == bg
                        && last.font == *font
                        && last.underline == underline
                        && last.strikethrough == strikethrough =>
                {
                    last.len += len
                }
                _ => runs.push(TextRun {
                    len,
                    font: font.clone(),
                    color: fg,
                    background_color: bg,
                    underline,
                    strikethrough,
                }),
            }
        }
        let cursor_shape = cursor.shape;
        drop(term);

        // The cursor when it isn't drawn as an inverted cell: a hollow box when
        // the terminal isn't focused, or a bar or underline.
        let cursor_overlay = cursor_at.filter(|_| !block_cursor).map(|vp| {
            let x = PAD + vp.column.0 as f32 * cw;
            let y = vp.line as f32 * ch;
            let d = div().absolute().left(px(x)).top(px(y));
            match (focused, cursor_shape) {
                (true, CursorShape::Beam) => d.w(px(2.)).h(px(ch)).bg(t.fg),
                (true, CursorShape::Underline) => {
                    d.top(px(y + ch - 2.)).w(px(cw)).h(px(2.)).bg(t.fg)
                }
                _ => d.w(px(cw)).h(px(ch)).border_1().border_color(t.muted),
            }
        });

        let bounds = self.bounds.clone();
        let (last_cols, last_rows) = {
            let s = self.terminal.size();
            (s.num_cols as usize, s.num_lines as usize)
        };
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .cursor_text()
            .font_family(mono)
            .text_size(px(FONT_SIZE))
            .line_height(px(ch))
            .text_color(t.fg)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.on_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.on_mouse_up(cx)),
            )
            .on_scroll_wheel(cx.listener(|this, e, _, cx| this.on_scroll(e, cx)))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy(cx);
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &Clear, _, cx| this.clear(cx)))
            .on_action(cx.listener(|this, _: &ScrollToTop, _, cx| this.scroll(Scroll::Top, cx)))
            .on_action(
                cx.listener(|this, _: &ScrollToBottom, _, cx| this.scroll(Scroll::Bottom, cx)),
            )
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| this.scroll(Scroll::PageUp, cx)))
            .on_action(
                cx.listener(|this, _: &ScrollPageDown, _, cx| this.scroll(Scroll::PageDown, cx)),
            )
            .child(
                canvas(
                    move |b, window, _| {
                        let old = bounds.replace(Some(b));
                        // A new size needs a new frame to lay the grid out again.
                        let cols = ((f32::from(b.size.width) - PAD * 2.) / cw).floor() as usize;
                        let rows = (f32::from(b.size.height) / ch).floor() as usize;
                        if old.is_none_or(|o| o.size != b.size)
                            && (cols.max(2), rows.max(1)) != (last_cols, last_rows)
                        {
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(rows.into_iter().map(|(text, runs)| {
                div()
                    .h(px(ch))
                    .pl(px(PAD))
                    .whitespace_nowrap()
                    .child(StyledText::new(text).with_runs(runs))
            }))
            .children(cursor_overlay)
    }
}

/// The bytes a key sends to the shell, as xterm does. `None` for keys the app
/// handles (Cmd shortcuts) and keys that send nothing.
pub fn keystroke_bytes(ks: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform {
        return None;
    }
    let modn = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let cursor_key = |c: char| {
        if modn > 1 {
            format!("\x1b[1;{modn}{c}")
        } else if app_cursor {
            format!("\x1bO{c}")
        } else {
            format!("\x1b[{c}")
        }
    };
    let tilde = |n: u8| {
        if modn > 1 {
            format!("\x1b[{n};{modn}~")
        } else {
            format!("\x1b[{n}~")
        }
    };
    let ss3 = |c: char| {
        if modn > 1 {
            format!("\x1b[1;{modn}{c}")
        } else {
            format!("\x1bO{c}")
        }
    };
    let esc = |s: &str| {
        if m.alt {
            format!("\x1b{s}")
        } else {
            s.to_string()
        }
    };
    let named = match ks.key.as_str() {
        "up" => Some(cursor_key('A')),
        "down" => Some(cursor_key('B')),
        "right" => Some(cursor_key('C')),
        "left" => Some(cursor_key('D')),
        "home" => Some(cursor_key('H')),
        "end" => Some(cursor_key('F')),
        "pageup" => Some(tilde(5)),
        "pagedown" => Some(tilde(6)),
        "insert" => Some(tilde(2)),
        "delete" => Some(tilde(3)),
        "f1" => Some(ss3('P')),
        "f2" => Some(ss3('Q')),
        "f3" => Some(ss3('R')),
        "f4" => Some(ss3('S')),
        "f5" => Some(tilde(15)),
        "f6" => Some(tilde(17)),
        "f7" => Some(tilde(18)),
        "f8" => Some(tilde(19)),
        "f9" => Some(tilde(20)),
        "f10" => Some(tilde(21)),
        "f11" => Some(tilde(23)),
        "f12" => Some(tilde(24)),
        "enter" => Some(esc("\r")),
        "tab" if m.shift => Some("\x1b[Z".to_string()),
        "tab" => Some(esc("\t")),
        "backspace" if m.control => Some(esc("\x08")),
        "backspace" => Some(esc("\x7f")),
        "escape" => Some("\x1b".to_string()),
        "space" if m.control => Some(esc("\0")),
        _ => None,
    };
    if let Some(s) = named {
        return Some(s.into_bytes());
    }

    // Ctrl+key: the control character. AltGr on Windows reports Ctrl+Alt
    // together with the character it typed, which goes through as text.
    let altgr = cfg!(windows) && m.control && m.alt && ks.key_char.is_some();
    if m.control && !altgr {
        let mut chars = ks.key.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            let code = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
                '@' | '2' | ' ' => Some(0),
                '[' | '3' => Some(0x1b),
                '\\' | '4' => Some(0x1c),
                ']' | '5' => Some(0x1d),
                '^' | '6' => Some(0x1e),
                '_' | '-' | '7' | '/' => Some(0x1f),
                '8' | '?' => Some(0x7f),
                _ => None,
            };
            if let Some(code) = code {
                let mut out = Vec::with_capacity(2);
                if m.alt {
                    out.push(0x1b);
                }
                out.push(code);
                return Some(out);
            }
        }
        return None;
    }

    let text = match &ks.key_char {
        Some(text) if !text.is_empty() => text.clone(),
        _ => match ks.key.as_str() {
            "space" => " ".to_string(),
            k if k.chars().count() == 1 => {
                if m.shift {
                    k.to_uppercase()
                } else {
                    k.to_string()
                }
            }
            _ => return None,
        },
    };
    // macOS's Option key types characters (ä, ∑); elsewhere Alt means Meta.
    if m.alt && !altgr && !cfg!(target_os = "macos") {
        return Some(format!("\x1b{text}").into_bytes());
    }
    Some(text.into_bytes())
}

/// ANSI colours, as in VS Code's default themes: black, red, green, yellow,
/// blue, magenta, cyan, white, then the bright ones.
const ANSI_DARK: [u32; 16] = [
    0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5, 0x666666,
    0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5,
];
const ANSI_LIGHT: [u32; 16] = [
    0x000000, 0xcd3131, 0x00bc00, 0x949800, 0x0451a5, 0xbc05bc, 0x0598bc, 0x555555, 0x666666,
    0xcd3131, 0x14ce14, 0xb5ba00, 0x0451a5, 0xbc05bc, 0x0598bc, 0xa5a5a5,
];

fn hex(rgb: u32) -> Hsla {
    gpui::rgb(rgb).into()
}

/// One of the 256 indexed colours.
fn index_color(index: usize, t: &Theme) -> Hsla {
    let palette = if t.appearance == Appearance::Dark {
        &ANSI_DARK
    } else {
        &ANSI_LIGHT
    };
    match index {
        0..=15 => hex(palette[index]),
        16..=231 => {
            let i = index - 16;
            let level = |n: usize| if n == 0 { 0 } else { 55 + n as u32 * 40 };
            hex((level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6))
        }
        232..=255 => {
            let g = 8 + (index as u32 - 232) * 10;
            hex((g << 16) | (g << 8) | g)
        }
        256 => t.fg,     // foreground
        257 => t.editor, // background
        _ => t.fg,
    }
}

fn resolve(
    color: AnsiColor,
    overrides: &alacritty_terminal::term::color::Colors,
    t: &Theme,
    is_fg: bool,
) -> Hsla {
    let from_rgb = |c: Rgb| hex(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32);
    match color {
        AnsiColor::Spec(rgb) => from_rgb(rgb),
        AnsiColor::Indexed(i) => overrides[i as usize]
            .map(from_rgb)
            .unwrap_or_else(|| index_color(i as usize, t)),
        AnsiColor::Named(named) => {
            if let Some(c) = overrides[named] {
                return from_rgb(c);
            }
            match named {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => t.fg,
                NamedColor::Background => {
                    if is_fg {
                        t.editor
                    } else {
                        t.fg
                    }
                }
                NamedColor::DimForeground => t.muted,
                n if (n as usize) < 16 => index_color(n as usize, t),
                // The dim variants: their normal colour, faded.
                n => index_color(n.to_bright() as usize, t).opacity(0.7),
            }
        }
    }
}

fn to_rgb(c: Hsla) -> Rgb {
    let c = c.to_rgb();
    Rgb {
        r: (c.r * 255.).round() as u8,
        g: (c.g * 255.).round() as u8,
        b: (c.b * 255.).round() as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(keys: &str) -> Option<Vec<u8>> {
        let ks = Keystroke::parse(keys).unwrap();
        keystroke_bytes(&ks, TermMode::empty())
    }

    #[test]
    fn keys_become_xterm_sequences() {
        assert_eq!(bytes("enter"), Some(b"\r".to_vec()));
        assert_eq!(bytes("ctrl-c"), Some(vec![3]));
        assert_eq!(bytes("ctrl-d"), Some(vec![4]));
        assert_eq!(bytes("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes("ctrl-left"), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(bytes("shift-tab"), Some(b"\x1b[Z".to_vec()));
        assert_eq!(bytes("backspace"), Some(b"\x7f".to_vec()));
        assert_eq!(bytes("pageup"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(bytes("f5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(bytes("a"), Some(b"a".to_vec()));
        assert_eq!(bytes("shift-a"), Some(b"A".to_vec()));
        assert_eq!(bytes("cmd-c"), None);
        let ks = Keystroke::parse("up").unwrap();
        assert_eq!(
            keystroke_bytes(&ks, TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn colour_cube() {
        let t = Theme::dark();
        assert_eq!(to_rgb(index_color(16, &t)), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            to_rgb(index_color(231, &t)),
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(to_rgb(index_color(196, &t)), Rgb { r: 255, g: 0, b: 0 });
    }
}

//! A multi-line, soft-wrapping text box for the commit message.
//! Based on GPUI's `input` example, extended to multiple lines.

use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window, WrappedLine,
};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::theme::ActiveTheme as _;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        Newline,
        Paste,
        Cut,
        Copy,
        ShowCharacterPalette,
    ]
);

pub const MIN_LINES: usize = 2;
pub const MAX_LINES: usize = 10;

pub enum TextInputEvent {
    Changed,
}

impl EventEmitter<TextInputEvent> for TextInput {}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    layout: Option<InputLayout>,
    visual_lines: usize,
    is_selecting: bool,
}

/// Shaped text from the last paint, used for hit testing and IME.
#[derive(Clone)]
struct InputLayout {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    /// Each hard line with its byte offset and top y (relative to `bounds`).
    lines: Vec<(usize, Pixels, WrappedLine)>,
}

impl InputLayout {
    fn position_for(&self, offset: usize) -> Point<Pixels> {
        for (ix, (start, y, line)) in self.lines.iter().enumerate() {
            let end = start + line.len();
            let is_last = ix + 1 == self.lines.len();
            if offset <= end || is_last {
                let local = offset.saturating_sub(*start).min(line.len());
                let p = line
                    .position_for_index(local, self.line_height)
                    .unwrap_or_default();
                return point(p.x, p.y + *y);
            }
        }
        point(px(0.), px(0.))
    }

    fn offset_for(&self, p: Point<Pixels>) -> usize {
        let mut chosen = None;
        for (start, y, line) in &self.lines {
            let h = line.size(self.line_height).height.max(self.line_height);
            if p.y < *y + h {
                chosen = Some((start, y, line));
                break;
            }
        }
        let Some((start, y, line)) =
            chosen.or_else(|| self.lines.last().map(|(s, y, l)| (s, y, l)))
        else {
            return 0;
        };
        let local = point(p.x.max(px(0.)), (p.y - *y).max(px(0.)));
        let ix = match line.closest_index_for_position(local, self.line_height) {
            Ok(ix) | Err(ix) => ix,
        };
        start + ix.min(line.len())
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            layout: None,
            visual_lines: MIN_LINES,
            is_selecting: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let placeholder = placeholder.into();
        if placeholder != self.placeholder {
            self.placeholder = placeholder;
            cx.notify();
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn vertical_target(&self, delta: f32) -> usize {
        let Some(layout) = &self.layout else {
            return self.cursor_offset();
        };
        let p = layout.position_for(self.cursor_offset());
        let y = p.y + layout.line_height * delta + layout.line_height / 2.;
        if y < px(0.) {
            return 0;
        }
        layout.offset_for(point(p.x, y))
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.vertical_target(-1.), cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.vertical_target(1.), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical_target(-1.), cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical_target(1.), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.content[..self.cursor_offset()]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let c = self.cursor_offset();
        let end = self.content[c..]
            .find('\n')
            .map_or(self.content.len(), |i| c + i);
        self.move_to(end, cx);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        let ix = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(ix, cx);
        } else {
            self.move_to(ix, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace("\r\n", "\n"), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let Some(layout) = &self.layout else { return 0 };
        if self.content.is_empty() {
            return 0;
        }
        layout.offset_for(position - layout.bounds.origin)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position_for(range.start);
        let end = layout.position_for(range.end);
        Some(Bounds::from_corners(
            point(bounds.left() + start.x, bounds.top() + start.y),
            point(
                bounds.left() + end.x,
                bounds.top() + end.y + layout.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let local = p - layout.bounds.origin;
        Some(self.offset_to_utf16(layout.offset_for(local)))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: InputLayout,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let lines = self.input.read(cx).visual_lines.clamp(MIN_LINES, MAX_LINES);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * lines as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let theme = cx.theme();
        let style = window.text_style();
        let line_height = window.line_height();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.to_string(), theme.muted)
        } else {
            (input.content.clone(), style.color)
        };
        let base = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked_range.as_ref() {
            Some(m) if !input.content.is_empty() => vec![
                TextRun {
                    len: m.start,
                    ..base.clone()
                },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base.clone()
                },
                TextRun {
                    len: text.len() - m.end,
                    ..base
                },
            ],
            _ => vec![base],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let shaped = window
            .text_system()
            .shape_text(
                SharedString::from(text.clone()),
                font_size,
                &runs,
                Some(bounds.size.width),
                None,
            )
            .unwrap_or_default();

        let mut lines = Vec::new();
        let mut offset = 0;
        let mut y = px(0.);
        let mut visual = 0;
        for line in shaped {
            let len = line.len();
            let rows = line.wrap_boundaries().len() + 1;
            lines.push((offset, y, line));
            y += line_height * rows as f32;
            visual += rows;
            offset += len + 1; // the '\n' separator
        }
        let layout = InputLayout {
            bounds,
            line_height,
            lines,
        };

        let mut selections = Vec::new();
        let mut cursor = None;
        if !input.content.is_empty() || input.selected_range.is_empty() {
            let sel = input.selected_range.clone();
            if sel.is_empty() {
                let p = if input.content.is_empty() {
                    point(px(0.), px(0.))
                } else {
                    layout.position_for(input.cursor_offset())
                };
                cursor = Some(fill(
                    Bounds::new(
                        point(bounds.left() + p.x, bounds.top() + p.y),
                        size(px(1.5), line_height),
                    ),
                    theme.accent,
                ));
            } else {
                let a = layout.position_for(sel.start);
                let b = layout.position_for(sel.end);
                let sel_color = theme.sel;
                let mut row_y = a.y;
                while row_y <= b.y {
                    let x0 = if row_y == a.y { a.x } else { px(0.) };
                    let x1 = if row_y == b.y { b.x } else { bounds.size.width };
                    selections.push(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, bounds.top() + row_y),
                            point(
                                bounds.left() + x1.max(x0 + px(4.)),
                                bounds.top() + row_y + line_height,
                            ),
                        ),
                        sel_color,
                    ));
                    row_y += line_height;
                }
            }
        }

        if visual.max(1) != input.visual_lines {
            let visual = visual.max(1);
            let entity = self.input.clone();
            window.on_next_frame(move |_, cx| {
                entity.update(cx, |input, cx| {
                    input.visual_lines = visual;
                    cx.notify();
                })
            });
        }

        PrepaintState {
            layout,
            cursor,
            selections,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let focused = focus_handle.is_focused(window);
        if focused {
            for quad in prepaint.selections.drain(..) {
                window.paint_quad(quad);
            }
        }
        let line_height = prepaint.layout.line_height;
        for (_, y, line) in &prepaint.layout.lines {
            line.paint(
                point(bounds.left(), bounds.top() + *y),
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
        }
        if focused {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        let layout = prepaint.layout.clone();
        self.input
            .update(cx, |input, _| input.layout = Some(layout));
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-input")
            .flex_1()
            .min_w_0()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Key bindings for the text box; `secondary` is Cmd on macOS and Ctrl elsewhere.
pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    use gpui::KeyBinding;
    let ctx = Some("TextInput");
    vec![
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
    ]
}

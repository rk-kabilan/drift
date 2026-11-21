//! A quick-pick popup (like VS Code's Cmd+P): a filter box over a list of
//! items, with fuzzy matching, keyboard navigation and optional sections.

use gpui::{
    div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    HighlightStyle, Hsla, SharedString, StyledText, UniformListScrollHandle, Window,
};

use super::text_input::{TextInput, TextInputEvent};
use super::widgets::icon;
use crate::fuzzy::fuzzy_match;
use crate::theme::ActiveTheme as _;

#[derive(Clone, Debug)]
pub struct PickerItem {
    /// The text matched against the query, e.g. a full path.
    pub haystack: String,
    /// Where the bold label starts inside `haystack`; the part before it is shown as detail.
    pub label_start: usize,
    /// Extra muted text after the label (e.g. a commit summary).
    pub note: String,
    pub icon: &'static str,
    pub icon_color: Option<Hsla>,
    /// A short right-aligned marker such as a status letter or "current".
    pub badge: Option<(String, Hsla)>,
    /// Items with the same section are grouped under that heading.
    pub section: Option<&'static str>,
}

pub enum PickerEvent {
    Confirm(usize),
    Dismiss,
}

enum ListRow {
    Header(&'static str),
    Item { item: usize, positions: Vec<usize> },
}

pub struct Picker {
    input: Entity<TextInput>,
    items: Vec<PickerItem>,
    rows: Vec<ListRow>,
    /// Row index of the highlighted item.
    selected: Option<usize>,
    scroll: UniformListScrollHandle,
    empty_text: SharedString,
    _subscription: gpui::Subscription,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Focusable for Picker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

const ROW_HEIGHT: f32 = 26.;
const MAX_VISIBLE: usize = 12;

impl Picker {
    pub fn new(
        placeholder: impl Into<SharedString>,
        empty_text: impl Into<SharedString>,
        items: Vec<PickerItem>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::single_line(placeholder, cx));
        let subscription = cx.subscribe(
            &input,
            |this: &mut Self, _, event: &TextInputEvent, cx| match event {
                TextInputEvent::Changed => this.filter(cx),
                TextInputEvent::Submit => this.confirm(cx),
                TextInputEvent::MoveUp => this.move_selection(-1, cx),
                TextInputEvent::MoveDown => this.move_selection(1, cx),
                TextInputEvent::Cancel => cx.emit(PickerEvent::Dismiss),
            },
        );
        let mut this = Self {
            input,
            items,
            rows: vec![],
            selected: None,
            scroll: UniformListScrollHandle::new(),
            empty_text: empty_text.into(),
            _subscription: subscription,
        };
        this.filter(cx);
        this
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_string();
        let mut matches: Vec<(usize, i64, Vec<usize>)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                fuzzy_match(&query, &item.haystack, item.label_start)
                    .map(|m| (i, m.score, m.positions))
            })
            .collect();
        // Keep sections in their original order; best matches first inside each.
        let section_rank = |i: usize| {
            let s = self.items[i].section;
            self.items
                .iter()
                .position(|it| it.section == s)
                .unwrap_or(0)
        };
        matches.sort_by(|a, b| {
            section_rank(a.0)
                .cmp(&section_rank(b.0))
                .then(b.1.cmp(&a.1))
                .then(a.0.cmp(&b.0))
        });

        self.rows.clear();
        let mut current: Option<&'static str> = None;
        for (item, _, positions) in matches {
            if let Some(section) = self.items[item].section {
                if current != Some(section) {
                    self.rows.push(ListRow::Header(section));
                    current = Some(section);
                }
            }
            self.rows.push(ListRow::Item { item, positions });
        }
        self.selected = self
            .rows
            .iter()
            .position(|r| matches!(r, ListRow::Item { .. }));
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, ListRow::Item { .. }))
            .map(|(i, _)| i)
            .collect();
        if items.is_empty() {
            return;
        }
        let pos = self
            .selected
            .and_then(|s| items.iter().position(|&i| i == s))
            .unwrap_or(0) as isize;
        let next = (pos + delta).rem_euclid(items.len() as isize) as usize;
        self.selected = Some(items[next]);
        self.scroll
            .scroll_to_item(items[next], gpui::ScrollStrategy::Center);
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(ListRow::Item { item, .. }) = self.selected.and_then(|s| self.rows.get(s)) {
            cx.emit(PickerEvent::Confirm(*item));
        }
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme().clone();
        match &self.rows[ix] {
            ListRow::Header(title) => div()
                .id(ix)
                .h(px(ROW_HEIGHT))
                .px(px(10.))
                .flex()
                .items_end()
                .pb(px(3.))
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(title.to_uppercase())
                .into_any_element(),
            ListRow::Item { item, positions } => {
                let it = &self.items[*item];
                let selected = self.selected == Some(ix);
                let label = it.haystack[it.label_start..].to_string();
                let detail = it.haystack[..it.label_start]
                    .trim_end_matches('/')
                    .to_string();
                let highlight = HighlightStyle {
                    color: Some(if selected { t.sel_fg } else { t.accent }),
                    font_weight: Some(gpui::FontWeight::BOLD),
                    ..Default::default()
                };
                let label_hl: Vec<_> = positions
                    .iter()
                    .filter(|&&p| p >= it.label_start)
                    .map(|&p| {
                        let start = p - it.label_start;
                        let len = label[start..].chars().next().map_or(1, char::len_utf8);
                        (start..start + len, highlight)
                    })
                    .collect();
                let hover = t.hover;
                let item_ix = *item;
                div()
                    .id(ix)
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.sel).text_color(t.sel_fg))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(icon(it.icon, it.icon_color.unwrap_or(t.muted)))
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .child(StyledText::new(label).with_highlights(label_hl)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(if selected {
                                t.sel_fg.opacity(0.75)
                            } else {
                                t.muted
                            })
                            .child(if it.note.is_empty() {
                                detail
                            } else if detail.is_empty() {
                                it.note.clone()
                            } else {
                                format!("{detail} · {}", it.note)
                            }),
                    )
                    .children(it.badge.clone().map(|(text, color)| {
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(text)
                    }))
                    .on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(PickerEvent::Confirm(item_ix))),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let count = self.rows.len();
        let list_height = px(ROW_HEIGHT * count.clamp(1, MAX_VISIBLE) as f32);
        div()
            .w(px(560.))
            .flex()
            .flex_col()
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .m(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .flex()
                    .bg(t.input)
                    .border_1()
                    .border_color(t.accent)
                    .rounded(px(4.))
                    .line_height(px(18.))
                    .child(self.input.clone()),
            )
            .child(if count == 0 {
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_color(t.muted)
                    .child(self.empty_text.clone())
                    .into_any_element()
            } else {
                uniform_list(
                    "picker-rows",
                    count,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .w_full()
                .h(list_height)
                .px(px(4.))
                .pb(px(4.))
                .into_any_element()
            })
    }
}

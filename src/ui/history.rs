//! The history graph at the bottom of the changes pane: commits of HEAD and
//! its upstream with their branch lines, and each commit's changed files.
//!
//! Commits load one page at a time as the list scrolls, only while the section
//! is open, and are dropped when it closes. Only one commit's file list is kept.

use std::ops::Range;

use gpui::{
    canvas, div, point, prelude::*, px, uniform_list, Bounds, Context, Hsla, Pixels, SharedString,
    UniformListScrollHandle,
};

use super::sidebar::{file_badge, status_color};
use super::widgets::{icon, icon_button, icon_sized, tooltip};
use super::workspace::{CommitTab, Workspace};
use crate::git::{CommitFile, GraphCommit, GraphCursor, RefKind};
use crate::theme::{ActiveTheme as _, Theme};

const ROW: f32 = 22.;
const LANE_W: f32 = 12.;
/// Load the next page when the list is scrolled this close to its end.
const PREFETCH: usize = 20;

#[derive(Default)]
pub struct History {
    commits: Vec<GraphCommit>,
    cursor: GraphCursor,
    /// The commits the graph was loaded from; reloaded when they move.
    tips: Vec<git2::Oid>,
    loaded: bool,
    /// The highest lane used so far, which sets the graph column's width.
    max_lane: u8,
    /// The commit whose files are shown, and those files.
    expanded: Option<(git2::Oid, Vec<CommitFile>)>,
    loading_more: bool,
    error: Option<String>,
    scroll: UniformListScrollHandle,
}

/// One line of the list.
enum Item<'a> {
    Commit(usize, &'a GraphCommit),
    File(&'a GraphCommit, &'a CommitFile),
}

impl History {
    fn len(&self) -> usize {
        self.commits.len() + self.expanded.as_ref().map_or(0, |(_, f)| f.len())
    }

    fn expanded_ix(&self) -> Option<usize> {
        let (oid, _) = self.expanded.as_ref()?;
        self.commits.iter().position(|c| c.oid == *oid)
    }

    fn item(&self, ix: usize) -> Option<Item<'_>> {
        match (self.expanded_ix(), &self.expanded) {
            (Some(e), Some((_, files))) if ix > e => {
                if ix <= e + files.len() {
                    Some(Item::File(&self.commits[e], &files[ix - e - 1]))
                } else {
                    let c = ix - files.len();
                    self.commits.get(c).map(|commit| Item::Commit(c, commit))
                }
            }
            _ => self.commits.get(ix).map(|commit| Item::Commit(ix, commit)),
        }
    }
}

impl Workspace {
    pub(super) fn graph_open(&self, cx: &gpui::App) -> bool {
        cx.global::<crate::settings::Settings>().show_graph
    }

    /// Shows or hides the history graph, and remembers the choice.
    pub fn toggle_graph(&mut self, cx: &mut Context<Self>) {
        crate::settings::update(cx, |s| s.show_graph = !s.show_graph);
        self.refresh_history(cx);
        cx.notify();
    }

    /// Loads the graph if it is open and HEAD or its upstream moved. Cheap when
    /// nothing changed, so it runs on every refresh.
    pub(super) fn refresh_history(&mut self, cx: &mut Context<Self>) {
        if !self.graph_open(cx) {
            if self.history.loaded {
                // Closed: free the commits.
                self.history = History::default();
            }
            return;
        }
        let Some(repo) = &self.repo else { return };
        let tips = repo.graph_tips();
        if self.history.loaded && tips == self.history.tips {
            return;
        }
        self.reload_history();
    }

    /// Reads the first page of the graph again, keeping the open commit open.
    pub(super) fn reload_history(&mut self) {
        let Some(repo) = &self.repo else { return };
        let h = &mut self.history;
        h.tips = repo.graph_tips();
        h.cursor = GraphCursor::default();
        h.loaded = true;
        h.loading_more = false;
        h.max_lane = 0;
        match repo.graph_page(&mut h.cursor) {
            Ok(commits) => {
                h.commits = commits;
                h.error = None;
            }
            Err(e) => {
                h.commits.clear();
                h.error = Some(format!("{e:#}"));
            }
        }
        h.max_lane = h.commits.iter().map(max_lane).max().unwrap_or(0);
        if let Some((oid, _)) = &h.expanded {
            if !h.commits.iter().any(|c| c.oid == *oid) {
                h.expanded = None;
            }
        }
    }

    fn load_more_history(&mut self, cx: &mut Context<Self>) {
        self.history.loading_more = false;
        let Some(repo) = &self.repo else { return };
        let h = &mut self.history;
        if h.cursor.done || !h.loaded {
            return;
        }
        match repo.graph_page(&mut h.cursor) {
            Ok(more) => {
                h.max_lane = more.iter().map(max_lane).fold(h.max_lane, u8::max);
                h.commits.extend(more);
            }
            Err(e) => {
                h.cursor.done = true;
                h.error = Some(format!("{e:#}"));
            }
        }
        cx.notify();
    }

    /// Shows a commit's changed files, or hides them if they are showing.
    fn toggle_commit(&mut self, oid: git2::Oid, cx: &mut Context<Self>) {
        if self
            .history
            .expanded
            .as_ref()
            .is_some_and(|(o, _)| *o == oid)
        {
            self.history.expanded = None;
        } else if let Some(repo) = &self.repo {
            match repo.commit_files(oid) {
                Ok(files) => self.history.expanded = Some((oid, files)),
                Err(e) => self.error = Some(format!("{e:#}")),
            }
        }
        cx.notify();
    }

    pub(super) fn render_history(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let open = self.graph_open(cx);
        let fg = t.fg;
        let hover = t.hover;
        let header = div()
            .id("graph-h")
            .group("graph-h")
            .h(px(28.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(6.))
            .pr(px(6.))
            .border_t_1()
            .border_color(t.border)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_size(px(11.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tooltip(format!(
                "{} History Graph{}",
                if open { "Hide" } else { "Show" },
                super::shortcuts::hint("ToggleGraph", cx)
            )))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_graph(cx)))
            .child(icon(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                fg,
            ))
            .child(icon_sized("graph", 14., t.muted))
            .child(div().flex_1().child("GRAPH"))
            .child({
                div()
                    .flex()
                    .flex_none()
                    .gap(px(1.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .when(open, |d| {
                        d.child(
                            icon_button(
                                "graph-reload",
                                "refresh",
                                fg,
                                cx,
                                cx.listener(|this, _, _, cx| {
                                    this.reload_history();
                                    cx.notify();
                                }),
                            )
                            .tooltip(tooltip("Reload History")),
                        )
                    })
            });
        let section = div().flex().flex_col().min_h_0().child(header);
        if !open {
            return section.flex_none();
        }
        let section = section.flex_1().min_h(px(120.));
        let h = &self.history;
        if let Some(e) = &h.error {
            if h.commits.is_empty() {
                return section.child(note(e.clone(), t.error));
            }
        }
        if h.commits.is_empty() {
            return section.child(note("No commits yet.".into(), t.muted));
        }

        let count = h.len();
        section.child(
            uniform_list(
                "graph-rows",
                count,
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    let t = cx.theme().clone();
                    let h = &this.history;
                    // Near the end: fetch the next page after this frame.
                    if range.end + PREFETCH >= h.len() && !h.cursor.done && !h.loading_more {
                        this.history.loading_more = true;
                        cx.spawn(async move |this, cx| {
                            this.update(cx, |this, cx| this.load_more_history(cx)).ok();
                        })
                        .detach();
                    }
                    let h = &this.history;
                    let graph_w = (h.max_lane as f32 + 1.) * LANE_W + 6.;
                    let head = h.tips.first().copied();
                    range
                        .filter_map(|ix| {
                            let el = match h.item(ix)? {
                                Item::Commit(i, c) => {
                                    let expanded =
                                        h.expanded.as_ref().is_some_and(|(o, _)| *o == c.oid);
                                    this.render_commit_row(
                                        i,
                                        c,
                                        expanded,
                                        Some(c.oid) == head,
                                        graph_w,
                                        &t,
                                        cx,
                                    )
                                }
                                Item::File(c, f) => this.render_commit_file(c, f, graph_w, &t, cx),
                            };
                            Some(el)
                        })
                        .collect()
                }),
            )
            .track_scroll(h.scroll.clone())
            .flex_1()
            .min_h_0()
            .pb(px(4.)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_commit_row(
        &self,
        ix: usize,
        c: &GraphCommit,
        expanded: bool,
        is_head: bool,
        graph_w: f32,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let hover = t.hover;
        let oid = c.oid;
        let age = crate::git::relative_time_since(c.time);
        let tip = format!("{} · {} · {}\n{}", c.short_sha(), c.author, age, c.summary);
        let mut labels = div()
            .flex()
            .flex_shrink()
            .min_w_0()
            .overflow_hidden()
            .gap(px(3.));
        for r in c.refs.iter().take(2) {
            labels = labels.child(ref_pill(&r.name, r.kind, t));
        }
        if c.refs.len() > 2 {
            labels = labels.child(
                div()
                    .text_size(px(10.))
                    .text_color(t.muted)
                    .child(format!("+{}", c.refs.len() - 2)),
            );
        }
        div()
            .id(("commit", ix))
            .w_full()
            .h(px(ROW))
            .flex()
            .items_center()
            .pr(px(8.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .when(expanded, |d| d.bg(t.hover))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_commit(oid, cx)))
            .child(graph_cell(c, is_head, expanded, graph_w, t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_shrink()
                            .min_w(px(40.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .when(is_head, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(SharedString::from(c.summary.to_string())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(t.muted)
                            .child(SharedString::from(c.author.to_string())),
                    ),
            )
            .child(labels)
            .into_any_element()
    }

    fn render_commit_file(
        &self,
        c: &GraphCommit,
        f: &CommitFile,
        graph_w: f32,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let selected = self.active_tab().is_some_and(|tab| {
            tab.path == f.path && tab.commit.as_ref().is_some_and(|tc| tc.oid == c.oid)
        });
        let (bg, fg) = if selected {
            (Some(t.sel), t.sel_fg)
        } else {
            (None, t.fg)
        };
        let hover = t.hover;
        let commit = CommitTab {
            oid: c.oid,
            short_sha: c.short_sha(),
            author: c.author.to_string(),
            summary: c.summary.to_string(),
            time: c.time,
            old_path: f.old_path.clone(),
            kind: f.kind,
        };
        let path = f.path.clone();
        let (badge, badge_color) = file_badge(f.file_name());
        // Lanes that continue past this commit run down beside its files.
        let lanes: Vec<(u8, Hsla)> = c
            .bottom
            .iter()
            .map(|&(_, b)| (b, lane_color(b, t)))
            .collect();
        div()
            .id(SharedString::from(format!("cf-{}-{}", c.oid, f.path)))
            .w_full()
            .h(px(ROW))
            .flex()
            .items_center()
            .gap(px(6.))
            .pr(px(10.))
            .cursor_pointer()
            .text_color(fg)
            .when_some(bg, |d, bg| d.bg(bg))
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                this.open_commit_file(commit.clone(), path.clone(), e.click_count() >= 2, cx)
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        for (lane, color) in &lanes {
                            let x = lane_x(bounds, *lane);
                            vline(window, *color, x, bounds.top(), bounds.bottom());
                        }
                    },
                )
                .w(px(graph_w + 8.))
                .h_full()
                .flex_none(),
            )
            .child(
                div()
                    .w(px(16.))
                    .flex_none()
                    .text_size(px(8.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(badge_color)
                    .child(badge),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .when(f.kind == crate::git::ChangeKind::Deleted, |d| {
                        d.line_through()
                    })
                    .child(f.file_name().to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .text_color(if selected { fg.opacity(0.7) } else { t.muted })
                    .child(f.dir().to_string()),
            )
            .child(
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_center()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(status_color(f.kind, t))
                    .child(f.kind.letter()),
            )
            .into_any_element()
    }
}

fn max_lane(c: &GraphCommit) -> u8 {
    c.top
        .iter()
        .chain(c.bottom.iter())
        .map(|&(a, b)| a.max(b))
        .fold(c.lane, u8::max)
}

fn note(text: String, color: Hsla) -> gpui::Div {
    div()
        .px(px(24.))
        .py(px(4.))
        .text_size(px(12.))
        .text_color(color)
        .child(text)
}

fn lane_color(lane: u8, t: &Theme) -> Hsla {
    let palette = [
        t.accent,
        t.st_added,
        t.st_modified,
        t.syn_type,
        t.st_deleted,
        t.syn_keyword,
        t.syn_function,
    ];
    palette[lane as usize % palette.len()]
}

fn lane_x(bounds: Bounds<Pixels>, lane: u8) -> Pixels {
    bounds.left() + px(10. + lane as f32 * LANE_W)
}

/// Width of the graph lines.
const LINE: f32 = 1.5;

// Lines are drawn as plain quads (a filled strip, or a bordered box with one
// rounded corner for a bend). Vector paths would make the renderer keep two
// window-sized offscreen textures, tens of megabytes, just for the graph.

/// A straight vertical line.
fn vline(window: &mut gpui::Window, color: Hsla, x: Pixels, y0: Pixels, y1: Pixels) {
    window.paint_quad(gpui::fill(
        Bounds::new(point(x - px(LINE / 2.), y0), gpui::size(px(LINE), y1 - y0)),
        color,
    ));
}

/// A line that runs down at `xa` from `y0`, then turns to reach `xb` at `y1`
/// (`down_first`), or leaves sideways from `xa` at `y0` and turns down to `xb`.
fn elbow(
    window: &mut gpui::Window,
    color: Hsla,
    (xa, xb): (Pixels, Pixels),
    (y0, y1): (Pixels, Pixels),
    down_first: bool,
) {
    if xa == xb {
        return vline(window, color, xa, y0, y1);
    }
    let h = px(LINE / 2.);
    let (w, z) = (px(LINE), px(0.));
    let right = xb > xa;
    let (left_x, right_x) = if right { (xa, xb) } else { (xb, xa) };
    let r = (right_x - left_x).min(y1 - y0);
    let (bounds, edges, corners) = if down_first {
        // Vertical edge at xa, horizontal edge along the bottom (y1).
        let b = Bounds::from_corners(point(left_x - h, y0), point(right_x + h, y1 + h));
        let e = if right {
            gpui::Edges {
                top: z,
                right: z,
                bottom: w,
                left: w,
            }
        } else {
            gpui::Edges {
                top: z,
                right: w,
                bottom: w,
                left: z,
            }
        };
        let c = if right {
            gpui::Corners {
                top_left: z,
                top_right: z,
                bottom_right: z,
                bottom_left: r,
            }
        } else {
            gpui::Corners {
                top_left: z,
                top_right: z,
                bottom_right: r,
                bottom_left: z,
            }
        };
        (b, e, c)
    } else {
        // Horizontal edge along the top (y0), vertical edge at xb.
        let b = Bounds::from_corners(point(left_x - h, y0 - h), point(right_x + h, y1));
        let e = if right {
            gpui::Edges {
                top: w,
                right: w,
                bottom: z,
                left: z,
            }
        } else {
            gpui::Edges {
                top: w,
                right: z,
                bottom: z,
                left: w,
            }
        };
        let c = if right {
            gpui::Corners {
                top_left: z,
                top_right: r,
                bottom_right: z,
                bottom_left: z,
            }
        } else {
            gpui::Corners {
                top_left: r,
                top_right: z,
                bottom_right: z,
                bottom_left: z,
            }
        };
        (b, e, c)
    };
    window.paint_quad(gpui::quad(
        bounds,
        corners,
        gpui::transparent_black(),
        edges,
        color,
        Default::default(),
    ));
}

/// The lane lines and the commit's dot.
fn graph_cell(
    c: &GraphCommit,
    is_head: bool,
    expanded: bool,
    graph_w: f32,
    t: &Theme,
) -> impl IntoElement {
    let top: Vec<(u8, u8, Hsla)> = c
        .top
        .iter()
        .map(|&(a, b)| (a, b, lane_color(a, t)))
        .collect();
    let bottom: Vec<(u8, u8, Hsla)> = c
        .bottom
        .iter()
        .map(|&(a, b)| (a, b, lane_color(b, t)))
        .collect();
    let lane = c.lane;
    let dot = lane_color(lane, t);
    let hollow_bg = t.panel;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let mid = bounds.top() + px(ROW / 2.);
            for &(a, b, color) in &top {
                elbow(
                    window,
                    color,
                    (lane_x(bounds, a), lane_x(bounds, b)),
                    (bounds.top(), mid),
                    true,
                );
            }
            for &(a, b, color) in &bottom {
                elbow(
                    window,
                    color,
                    (lane_x(bounds, a), lane_x(bounds, b)),
                    (mid, bounds.bottom()),
                    false,
                );
            }
            let r = if is_head || expanded { 4.5 } else { 3.5 };
            let dot_bounds = Bounds::new(
                point(lane_x(bounds, lane) - px(r), mid - px(r)),
                gpui::size(px(r * 2.), px(r * 2.)),
            );
            if is_head {
                window.paint_quad(gpui::quad(
                    dot_bounds,
                    px(r),
                    hollow_bg,
                    px(2.),
                    dot,
                    Default::default(),
                ));
            } else {
                window.paint_quad(gpui::fill(dot_bounds, dot).corner_radii(px(r)));
            }
        },
    )
    .w(px(graph_w))
    .h_full()
    .flex_none()
}

fn ref_pill(name: &str, kind: RefKind, t: &Theme) -> impl IntoElement {
    let (bg, fg, border, icon_name) = match kind {
        RefKind::Head => (t.accent, t.on_accent, t.accent, "branch"),
        RefKind::Local => (t.badge, t.badge_fg, t.border, "branch"),
        RefKind::Remote => (t.panel, t.fg, t.border, "cloud"),
        RefKind::Tag => (t.panel, t.fg, t.border, "tag"),
    };
    div()
        .h(px(16.))
        .flex_none()
        .max_w(px(100.))
        .px(px(5.))
        .flex()
        .items_center()
        .gap(px(3.))
        .rounded(px(8.))
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_color(fg)
        .text_size(px(10.))
        .overflow_hidden()
        .child(icon_sized(icon_name, 10., fg))
        .child(
            div()
                .overflow_hidden()
                .text_ellipsis()
                .child(name.to_string()),
        )
}

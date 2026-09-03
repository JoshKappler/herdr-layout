use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width, truncate_end};
use crate::app::detail_panel::{ExchangeView, Timeline, TimelineItem};
use crate::app::state::Palette;
use crate::app::AppState;

/// Right-side drill-in for the focused tab (Josh 2026-08-27): one aggregate
/// line, then the task timeline: done in green with per-task time, in
/// flight in yellow with a running clock, inferred next steps in purple.
/// The bottom third holds the prompt viewer cycling recent exchanges;
/// clicking a task row pins it to that exchange.
pub(super) fn render_detail_panel(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let p = &app.palette;
    let sep_style = Style::default().fg(p.text);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(area.x, y)].set_symbol("│");
        buf[(area.x, y)].set_style(sep_style);
    }

    let (list, viewer) = split_content(content_rect(area));
    let lines = detail_panel_lines(app, list.width);
    let scroll = app.detail_panel_scroll.min(max_scroll_for(&lines, list)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), list);
    if viewer.height > 0 {
        let lines = viewer_lines(app, viewer.width, viewer.height);
        frame.render_widget(Paragraph::new(lines), viewer);
    }
}

fn content_rect(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(3),
        area.height.saturating_sub(1),
    )
}

const VIEWER_MIN_CONTENT_ROWS: u16 = 9;

/// The list keeps the top two thirds; the prompt viewer takes the bottom
/// third once the content area is tall enough for both.
fn split_content(content: Rect) -> (Rect, Rect) {
    if content.height < VIEWER_MIN_CONTENT_ROWS {
        return (content, Rect::default());
    }
    let viewer_h = content.height / 3;
    let list_h = content.height - viewer_h;
    (
        Rect::new(content.x, content.y, content.width, list_h),
        Rect::new(content.x, content.y + list_h, content.width, viewer_h),
    )
}

pub(crate) fn detail_panel_max_scroll(app: &AppState, area: Rect) -> usize {
    if area.width < 4 || area.height < 2 {
        return 0;
    }
    let (list, _) = split_content(content_rect(area));
    max_scroll_for(&detail_panel_lines(app, list.width), list)
}

fn max_scroll_for(lines: &[Line<'_>], content: Rect) -> usize {
    lines.len().saturating_sub(content.height as usize)
}

/// What a click on a panel line should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DetailPanelHit {
    /// pin the viewer to exchange `u`; `query` also jumps the feed
    ViewExchange { u: String, query: Option<String> },
    ToggleSub { path: String },
    /// a click anywhere in the viewer unpins it and resumes cycling
    ViewerResume,
    /// pin the viewer to the previous exchange; None at the oldest
    ViewerPrev { u: Option<String> },
    /// pin the viewer to the next exchange; None at the newest
    ViewerNext { u: Option<String> },
    /// scroll the terminal feed to the shown exchange
    ViewerJump { query: Option<String> },
}

pub(crate) fn detail_panel_hit(
    app: &AppState,
    area: Rect,
    col: u16,
    row: u16,
) -> Option<DetailPanelHit> {
    let (list, viewer) = split_content(content_rect(area));
    if viewer.height > 0 && row >= viewer.y && row < viewer.y + viewer.height {
        return Some(viewer_hit(app, viewer, col, row));
    }
    if row < list.y || row >= list.y + list.height {
        return None;
    }
    let (lines, hits) = detail_panel_lines_with_hits(app, list.width);
    let scroll = app.detail_panel_scroll.min(max_scroll_for(&lines, list));
    let line_idx = (row - list.y) as usize + scroll;
    hits.into_iter().nth(line_idx).flatten()
}

/// Buttons on the meta row claim their columns; everywhere else resumes.
fn viewer_hit(app: &AppState, viewer: Rect, col: u16, row: u16) -> DetailPanelHit {
    if row == viewer.y + 1 && col >= viewer.x {
        if let Some((idx, views)) = viewer_state(app) {
            let x = (col - viewer.x) as usize;
            let (_, buttons) = viewer_meta(app, idx, &views);
            if let Some((_, hit)) = buttons.into_iter().find(|(range, _)| range.contains(&x)) {
                return hit;
            }
        }
    }
    DetailPanelHit::ViewerResume
}

/// The first words of a prompt, short enough to sit on one feed line.
fn feed_query(head: &str) -> Option<String> {
    let first = head.lines().next().unwrap_or("").trim();
    let mut query: String = first.chars().take(30).collect();
    if query.len() < first.len() {
        if let Some(cut) = query.rfind(' ') {
            query.truncate(cut);
        }
    }
    let query = query.trim().to_string();
    (query.chars().count() >= 8).then_some(query)
}

/// Jump text for an item: the longer `_exch` head when present (better
/// scrollback odds), else the item's clipped copy.
fn jump_head<'a>(tl: &'a Timeline, item: &'a TimelineItem) -> &'a str {
    tl.exch
        .iter()
        .find(|e| e.u == item.u && !e.head.is_empty())
        .map_or(item.head.as_str(), |e| e.head.as_str())
}

fn detail_panel_lines(app: &AppState, width: u16) -> Vec<Line<'static>> {
    detail_panel_lines_with_hits(app, width).0
}

type PanelLines = (Vec<Line<'static>>, Vec<Option<DetailPanelHit>>);

fn detail_panel_lines_with_hits(app: &AppState, width: u16) -> PanelLines {
    let p = &app.palette;
    let dim = Style::default().fg(p.subtext0);
    let mut lines = Vec::new();
    let mut hits: Vec<Option<DetailPanelHit>> = Vec::new();

    let Some(cache) = &app.detail_panel else {
        lines.push(Line::from(Span::styled("no agent session in this pane", dim)));
        return (lines, vec![None]);
    };
    let Some(tl) = &cache.timeline else {
        lines.push(Line::from(Span::styled("timeline pending", dim)));
        return (lines, vec![None]);
    };

    lines.push(Line::from(Span::styled(aggregate_line(tl), dim)));
    hits.push(None);
    let now = now_epoch();
    if !(tl.done.is_empty() && tl.current.is_empty() && tl.next.is_empty()) {
        lines.push(Line::default());
        hits.push(None);
        lines.push(section_header("tasks", width, p));
        hits.push(None);
    }
    for item in &tl.done {
        lines.push(item_line(
            "✓",
            &item.label,
            item.secs.map(fmt_secs),
            p.green,
            width,
            p,
        ));
        hits.push(Some(DetailPanelHit::ViewExchange {
            u: item.u.clone(),
            query: feed_query(jump_head(tl, item)),
        }));
    }
    for item in &tl.current {
        let running = (item.ts > 0.0).then(|| fmt_secs(now - item.ts));
        lines.push(item_line("●", &item.label, running, p.yellow, width, p));
        hits.push(Some(DetailPanelHit::ViewExchange {
            u: item.u.clone(),
            query: feed_query(jump_head(tl, item)),
        }));
    }
    for label in &tl.next {
        lines.push(item_line("◇", label, None, p.mauve, width, p));
        hits.push(None);
    }

    if !tl.subs.is_empty() {
        lines.push(Line::default());
        hits.push(None);
        lines.push(section_header("subagents", width, p));
        hits.push(None);
        for sub in &tl.subs {
            // finished lanes stay listed: green like a done task, total runtime
            let (color, clock) = if sub.status == "done" {
                let total = (sub.started > 0.0 && sub.ended > sub.started)
                    .then(|| fmt_secs(sub.ended - sub.started));
                (p.green, total)
            } else {
                (
                    p.yellow,
                    (sub.started > 0.0).then(|| fmt_secs(now - sub.started)),
                )
            };
            lines.push(item_line("●", &sub.label, clock, color, width, p));
            hits.push(Some(DetailPanelHit::ToggleSub {
                path: sub.path.clone(),
            }));
            if app.detail_panel_expanded.contains(&sub.path) {
                for event in &sub.events {
                    lines.push(Line::from(Span::styled(format!("   {event}"), dim)));
                    hits.push(None);
                }
            }
        }
    }
    (lines, hits)
}

const CYCLE_WINDOW: usize = 8;

/// The exchange the viewer shows: the pinned one when it resolves, else
/// the cycler's pick over the last CYCLE_WINDOW, oldest to newest.
/// Returns the shown index plus every exchange, oldest first.
fn viewer_state(app: &AppState) -> Option<(usize, Vec<ExchangeView>)> {
    let tl = app.detail_panel.as_ref()?.timeline.as_ref()?;
    let views = tl.exchange_views();
    if views.is_empty() {
        return None;
    }
    let total = views.len();
    let idx = app
        .detail_panel_pinned
        .as_deref()
        .and_then(|u| views.iter().position(|v| v.u == u))
        .unwrap_or_else(|| {
            let window = total.min(CYCLE_WINDOW);
            total - window + app.detail_panel_cycle % window
        });
    Some((idx, views))
}

fn viewer_exchange(app: &AppState) -> Option<(usize, usize, ExchangeView)> {
    let (idx, mut views) = viewer_state(app)?;
    let total = views.len();
    Some((idx + 1, total, views.swap_remove(idx)))
}

type MetaButtons = Vec<(std::ops::Range<usize>, DetailPanelHit)>;

fn push_span(spans: &mut Vec<Span<'static>>, x: &mut usize, text: String, style: Style) {
    *x += display_width(&text);
    spans.push(Span::styled(text, style));
}

/// The viewer's meta row: time and duration, then the step and jump
/// buttons with their column ranges; exhausted buttons render dimmed.
fn viewer_meta(app: &AppState, idx: usize, views: &[ExchangeView]) -> (Line<'static>, MetaButtons) {
    let p = &app.palette;
    let dim = Style::default().fg(p.subtext0);
    let btn = Style::default().fg(p.text);
    let off = Style::default().fg(p.overlay0);
    let exch = &views[idx];
    let mut spans = Vec::new();
    let mut buttons = MetaButtons::new();
    let mut x = 0;

    let mut lead = format!("{} · ", fmt_clock(exch.ts));
    if let Some(secs) = exch.secs {
        lead.push_str(&format!("{} · ", fmt_secs(secs)));
    }
    push_span(&mut spans, &mut x, lead, dim);
    let prev = (idx > 0).then(|| views[idx - 1].u.clone());
    let start = x;
    push_span(&mut spans, &mut x, "[‹]".into(), if prev.is_some() { btn } else { off });
    buttons.push((start..x, DetailPanelHit::ViewerPrev { u: prev }));
    push_span(&mut spans, &mut x, format!(" {}/{} ", idx + 1, views.len()), dim);
    let next = (idx + 1 < views.len()).then(|| views[idx + 1].u.clone());
    let start = x;
    push_span(&mut spans, &mut x, "[›]".into(), if next.is_some() { btn } else { off });
    buttons.push((start..x, DetailPanelHit::ViewerNext { u: next }));
    push_span(&mut spans, &mut x, " · ".into(), dim);
    let query = feed_query(&exch.head);
    let start = x;
    push_span(&mut spans, &mut x, "[jump]".into(), if query.is_some() { btn } else { off });
    buttons.push((start..x, DetailPanelHit::ViewerJump { query }));
    if app.detail_panel_pinned.as_deref() == Some(exch.u.as_str()) {
        push_span(&mut spans, &mut x, " · pinned".into(), dim);
    }
    (Line::from(spans), buttons)
}

fn viewer_lines(app: &AppState, width: u16, height: u16) -> Vec<Line<'static>> {
    let p = &app.palette;
    let mut lines = vec![section_header("prompts", width, p)];
    let Some((idx, views)) = viewer_state(app) else {
        let dim = Style::default().fg(p.subtext0);
        lines.push(Line::from(Span::styled("no prompts yet", dim)));
        return lines;
    };
    lines.push(viewer_meta(app, idx, &views).0);
    let exch = &views[idx];
    let (glyph, color) = if exch.done {
        ("✓", p.green)
    } else {
        ("●", p.yellow)
    };
    lines.push(item_line(glyph, &exch.label, None, color, width, p));
    let body = viewer_body(exch, width, p);
    let room = (height as usize).saturating_sub(lines.len());
    let scroll = app.detail_panel_viewer_scroll.min(body.len().saturating_sub(room));
    lines.extend(body.into_iter().skip(scroll).take(room));
    lines
}

/// The wrapped prompt then response text: the scrollable region.
fn viewer_body(exch: &ExchangeView, width: u16, p: &Palette) -> Vec<Line<'static>> {
    let wrap_w = width.max(8) as usize;
    let text = Style::default().fg(p.text);
    let dim = Style::default().fg(p.subtext0);
    let mut lines: Vec<Line<'static>> = wrap_text(&exch.head, wrap_w)
        .into_iter()
        .map(|t| Line::from(Span::styled(t, text)))
        .collect();
    for t in wrap_text(&exch.rhead, wrap_w) {
        lines.push(Line::from(Span::styled(t, dim)));
    }
    lines
}

/// header, meta, label rows sit above the scrolling body
const VIEWER_FIXED_ROWS: usize = 3;

pub(crate) fn detail_panel_viewer_rect(area: Rect) -> Rect {
    if area.width < 4 || area.height < 2 {
        return Rect::default();
    }
    split_content(content_rect(area)).1
}

pub(crate) fn detail_panel_viewer_max_scroll(app: &AppState, area: Rect) -> usize {
    let viewer = detail_panel_viewer_rect(area);
    let Some((idx, views)) = viewer_state(app) else {
        return 0;
    };
    if viewer.height == 0 {
        return 0;
    }
    let room = (viewer.height as usize).saturating_sub(VIEWER_FIXED_ROWS);
    viewer_body(&views[idx], viewer.width, &app.palette)
        .len()
        .saturating_sub(room)
}

/// Resets the viewer's scroll when the shown exchange changes.
pub(super) fn sync_viewer_scroll(app: &mut AppState) {
    let shown = viewer_exchange(app).map(|(_, _, view)| view.u);
    if app.detail_panel_viewer_shown != shown {
        app.detail_panel_viewer_shown = shown;
        app.detail_panel_viewer_scroll = 0;
    }
}

#[cfg(test)]
pub(crate) fn test_viewer_buttons(app: &AppState, area: Rect) -> Vec<(u16, u16, DetailPanelHit)> {
    let viewer = detail_panel_viewer_rect(area);
    let Some((idx, views)) = viewer_state(app) else {
        return Vec::new();
    };
    viewer_meta(app, idx, &views)
        .1
        .into_iter()
        .map(|(range, hit)| (viewer.x + range.start as u16, viewer.y + 1, hit))
        .collect()
}

fn aggregate_line(tl: &Timeline) -> String {
    let mut parts = vec![
        fmt_secs(tl.total_secs),
        format!("{} tok", fmt_tokens(tl.out_tokens)),
    ];
    if !tl.status.is_empty() {
        parts.push(tl.status.clone());
    }
    parts.join(" · ")
}

fn item_line(
    glyph: &str,
    label: &str,
    right: Option<String>,
    color: ratatui::style::Color,
    width: u16,
    p: &Palette,
) -> Line<'static> {
    let dim = Style::default().fg(p.subtext0);
    // an unlabeled item is still being summarized; never show raw prompt text
    let (label, style) = if label.is_empty() {
        ("summarizing…", dim)
    } else {
        (label, Style::default().fg(color))
    };
    let right = right.unwrap_or_default();
    let right_w = display_width(&right);
    let reserve = 2 + if right_w > 0 { right_w + 1 } else { 0 };
    let label_w = (width as usize).saturating_sub(reserve).max(4);
    let label = truncate_end(label, label_w);
    let used = 2 + display_width(&label);
    let pad = (width as usize).saturating_sub(used + right_w).max(1);
    let mut spans = vec![Span::styled(format!("{glyph} {label}"), style)];
    if right_w > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(right, dim));
    }
    Line::from(spans)
}

fn section_header(title: &str, width: u16, p: &Palette) -> Line<'static> {
    let style = Style::default().fg(p.overlay0);
    let label = format!("─ {title} ");
    let fill = (width as usize).saturating_sub(display_width(&label));
    Line::from(Span::styled(format!("{label}{}", "─".repeat(fill)), style))
}

fn now_epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn fmt_secs(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    if secs >= 3600 {
        format!("{}h{:02}", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Epoch seconds as the local wall clock, "14:22".
fn fmt_clock(ts: f64) -> String {
    let (hour, min) = local_hour_min(ts.max(0.0) as i64);
    format!("{hour:02}:{min:02}")
}

#[cfg(unix)]
fn local_hour_min(epoch: i64) -> (i32, i32) {
    let time = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time, &mut tm) };
    (tm.tm_hour, tm.tm_min)
}

#[cfg(not(unix))]
fn local_hour_min(epoch: i64) -> (i32, i32) {
    let day = epoch.rem_euclid(86_400) as i32;
    (day / 3600, day % 3600 / 60)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            if !matches!(out.last(), Some(last) if last.is_empty()) && !out.is_empty() {
                out.push(String::new());
            }
            continue;
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            if line.is_empty() {
                line = word.to_string();
            } else if display_width(&line) + 1 + display_width(word) <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    while matches!(out.last(), Some(last) if last.is_empty()) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::detail_panel::{Exchange, TimelineItem};

    #[test]
    fn durations_and_tokens_format_compactly() {
        assert_eq!(fmt_secs(42.0), "42s");
        assert_eq!(fmt_secs(754.0), "12m");
        assert_eq!(fmt_secs(6135.0), "1h42");
        assert_eq!(fmt_tokens(96_512), "96k");
        assert_eq!(fmt_tokens(1_240_000), "1.2M");
    }

    fn test_timeline() -> Timeline {
        let item = |u: &str, label: &str, secs| TimelineItem {
            u: u.into(),
            label: label.into(),
            head: format!("please {label} soon"),
            ts: 100.0,
            secs,
            off: 0,
        };
        Timeline {
            status: "working".into(),
            total_secs: 754.0,
            out_tokens: 96_512,
            done: vec![
                item("u1", "restored the configs", Some(300.0)),
                item("u2", "", Some(60.0)),
            ],
            current: vec![item("u3", "building the panel", None)],
            next: vec!["ship the teardown".into()],
            exch: vec![Exchange {
                u: "u1".into(),
                ts: 1_787_937_325.0,
                last_ts: 1_787_937_625.0,
                head: "please restore both configs before anything else breaks".into(),
                rhead: "Restored. Both configs match again.".into(),
                label: String::new(),
                ylabel: "restoring the configs".into(),
            }],
            subs: vec![
                crate::app::detail_panel::SubLane {
                    label: "writing allocation unit tests".into(),
                    status: "working".into(),
                    started: 50.0,
                    ended: 0.0,
                    path: "/tmp/lane.jsonl".into(),
                    events: vec!["18:59 Write sandboxProbe.ts".into()],
                },
                crate::app::detail_panel::SubLane {
                    label: "audited the palette mapping".into(),
                    status: "done".into(),
                    started: 10.0,
                    ended: 52.0,
                    path: "/tmp/lane-done.jsonl".into(),
                    events: vec![],
                },
            ],
        }
    }

    #[test]
    fn panel_hits_map_tasks_and_subs_through_scroll() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.active = Some(0);
        app.detail_panel_open = true;
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 180, 40));
        let rect = app.view.detail_panel_rect;
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );

        let content = content_rect(rect);
        assert_eq!(detail_panel_hit(&app, rect, content.x, content.y), None);
        // blank + tasks header, then the first done row pins its exchange;
        // the jump query prefers the longer `_exch` head over the item's
        assert_eq!(detail_panel_hit(&app, rect, content.x, content.y + 2), None);
        assert_eq!(
            detail_panel_hit(&app, rect, content.x, content.y + 3),
            Some(DetailPanelHit::ViewExchange {
                u: "u1".into(),
                query: Some("please restore both configs".into()),
            })
        );
        // purple prediction row is not clickable
        assert_eq!(detail_panel_hit(&app, rect, content.x, content.y + 6), None);
        // blank + header, then live and finished subagent rows both toggle
        assert_eq!(
            detail_panel_hit(&app, rect, content.x, content.y + 9),
            Some(DetailPanelHit::ToggleSub {
                path: "/tmp/lane.jsonl".into()
            })
        );
        assert_eq!(
            detail_panel_hit(&app, rect, content.x, content.y + 10),
            Some(DetailPanelHit::ToggleSub {
                path: "/tmp/lane-done.jsonl".into()
            })
        );
        // every viewer row off the buttons resumes cycling, never a list hit
        let (list, viewer) = split_content(content);
        assert_eq!(list.height, 26);
        assert_eq!(viewer.height, 13);
        assert_eq!(detail_panel_hit(&app, rect, content.x, viewer.y - 1), None);
        for row in viewer.y..viewer.y + viewer.height {
            assert_eq!(
                detail_panel_hit(&app, rect, viewer.x, row),
                Some(DetailPanelHit::ViewerResume)
            );
        }
    }

    #[test]
    fn viewer_clicks_intercept_even_where_scrolled_rows_would_hit() {
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        // content is 9 rows: 6 list rows plus a 3-row viewer
        let rect = Rect::new(0, 0, 46, 10);
        let (list, viewer) = split_content(content_rect(rect));
        app.detail_panel_scroll = 4;
        // scrolled so a subagent toggle row sits on the last list row
        assert_eq!(
            detail_panel_hit(&app, rect, list.x, list.y + list.height - 1),
            Some(DetailPanelHit::ToggleSub {
                path: "/tmp/lane.jsonl".into()
            })
        );
        // one row lower is the viewer: the next toggle row must not map
        assert_eq!(
            detail_panel_hit(&app, rect, viewer.x, viewer.y),
            Some(DetailPanelHit::ViewerResume)
        );
    }

    #[test]
    fn short_panels_skip_the_viewer_and_keep_the_full_list() {
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        // content is 8 rows, under the 9-row minimum: no viewer anywhere
        let rect = Rect::new(0, 0, 46, 9);
        let content = content_rect(rect);
        assert_eq!(split_content(content).1, Rect::default());
        assert_eq!(
            detail_panel_hit(&app, rect, content.x, content.y + 3),
            Some(DetailPanelHit::ViewExchange {
                u: "u1".into(),
                query: Some("please restore both configs".into()),
            })
        );
        for row in content.y + 6..content.y + content.height {
            assert_ne!(
                detail_panel_hit(&app, rect, content.x, row),
                Some(DetailPanelHit::ViewerResume)
            );
        }
    }

    #[test]
    fn viewer_cycles_oldest_to_newest_and_pins_on_request() {
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        for (cycle, u) in [(0, "u1"), (1, "u2"), (2, "u3"), (3, "u1")] {
            app.detail_panel_cycle = cycle;
            let (pos, total, view) = viewer_exchange(&app).expect("an exchange to show");
            assert_eq!(view.u, u);
            assert_eq!(total, 3);
            assert_eq!(pos, cycle % 3 + 1);
        }
        // u1 rides its _exch entry; u2 has none and falls back to the item
        app.detail_panel_cycle = 0;
        let (_, _, view) = viewer_exchange(&app).expect("an exchange to show");
        assert!(view.head.starts_with("please restore both configs"));
        assert!(!view.rhead.is_empty());
        app.detail_panel_cycle = 1;
        let (_, _, view) = viewer_exchange(&app).expect("an exchange to show");
        assert_eq!(view.head, "please  soon");
        assert!(view.rhead.is_empty());

        app.detail_panel_pinned = Some("u1".into());
        app.detail_panel_cycle = 2;
        let (pos, _, view) = viewer_exchange(&app).expect("an exchange to show");
        assert_eq!((pos, view.u.as_str()), (1, "u1"));
    }

    #[test]
    fn viewer_buttons_hit_their_own_columns_and_clamp_step_targets() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.active = Some(0);
        app.detail_panel_open = true;
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 180, 40));
        let rect = app.view.detail_panel_rect;
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        app.detail_panel_cycle = 1;
        let viewer = detail_panel_viewer_rect(rect);

        // u2 shown: each button maps across its full width, no unpin
        let (idx, views) = viewer_state(&app).expect("a shown exchange");
        let buttons = viewer_meta(&app, idx, &views).1;
        assert_eq!(buttons.len(), 3);
        for (range, hit) in &buttons {
            for x in [range.start, range.end - 1] {
                assert_eq!(
                    detail_panel_hit(&app, rect, viewer.x + x as u16, viewer.y + 1),
                    Some(hit.clone())
                );
            }
        }
        let hits: Vec<_> = buttons.into_iter().map(|(_, hit)| hit).collect();
        assert!(hits.contains(&DetailPanelHit::ViewerPrev { u: Some("u1".into()) }));
        assert!(hits.contains(&DetailPanelHit::ViewerNext { u: Some("u3".into()) }));
        assert!(hits.contains(&DetailPanelHit::ViewerJump {
            query: Some("please  soon".into())
        }));
        // the clock and the body rows still resume cycling
        assert_eq!(
            detail_panel_hit(&app, rect, viewer.x, viewer.y + 1),
            Some(DetailPanelHit::ViewerResume)
        );
        assert_eq!(
            detail_panel_hit(&app, rect, viewer.x + 1, viewer.y + 2),
            Some(DetailPanelHit::ViewerResume)
        );

        // at the ends the exhausted arrow carries no step target
        app.detail_panel_cycle = 0;
        let (idx, views) = viewer_state(&app).expect("a shown exchange");
        let hits: Vec<_> = viewer_meta(&app, idx, &views)
            .1
            .into_iter()
            .map(|(_, hit)| hit)
            .collect();
        assert!(hits.contains(&DetailPanelHit::ViewerPrev { u: None }));
        assert!(hits.contains(&DetailPanelHit::ViewerNext { u: Some("u2".into()) }));
        app.detail_panel_pinned = Some("u3".into());
        let (idx, views) = viewer_state(&app).expect("a shown exchange");
        let hits: Vec<_> = viewer_meta(&app, idx, &views)
            .1
            .into_iter()
            .map(|(_, hit)| hit)
            .collect();
        assert!(hits.contains(&DetailPanelHit::ViewerPrev { u: Some("u2".into()) }));
        assert!(hits.contains(&DetailPanelHit::ViewerNext { u: None }));
    }

    #[test]
    fn jump_query_prefers_the_exchanges_first_head_line() {
        let mut tl = test_timeline();
        tl.exch[0].head =
            "restore the configs from the backup snapshot\nthen tail the logs".into();
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(tl),
        );
        let (_, hits) = detail_panel_lines_with_hits(&app, 43);
        let query_for = |u: &str| {
            hits.iter()
                .flatten()
                .find_map(|hit| match hit {
                    DetailPanelHit::ViewExchange { u: hit_u, query } if hit_u == u => {
                        Some(query.clone())
                    }
                    _ => None,
                })
                .expect("a task row hit")
        };
        // first `_exch` head line only, capped back to the last whole word
        assert_eq!(
            query_for("u1"),
            Some("restore the configs from the".into())
        );
        // u2 has no `_exch` entry and keeps the item's clipped head
        assert_eq!(query_for("u2"), Some("please  soon".into()));
    }

    #[test]
    fn viewer_scroll_offsets_the_body_and_clamps() {
        let mut tl = test_timeline();
        tl.exch[0].head = (1..=12)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(tl),
        );
        // content is 15 rows: a 10-row list and a 5-row viewer (2 body rows)
        let rect = Rect::new(0, 0, 46, 16);
        let viewer = detail_panel_viewer_rect(rect);
        assert_eq!(viewer.height, 5);
        // 12 head lines plus one rhead line, two visible at a time
        assert_eq!(detail_panel_viewer_max_scroll(&app, rect), 11);
        let body_text = |app: &AppState| -> Vec<String> {
            viewer_lines(app, viewer.width, viewer.height)
                .into_iter()
                .skip(3)
                .map(|line| line.spans.iter().map(|s| s.content.clone()).collect())
                .collect()
        };
        assert_eq!(body_text(&app), ["word1", "word2"]);
        app.detail_panel_viewer_scroll = 3;
        assert_eq!(body_text(&app), ["word4", "word5"]);
        app.detail_panel_viewer_scroll = 99;
        assert_eq!(
            body_text(&app),
            ["word12", "Restored. Both configs match again."]
        );
    }

    #[test]
    fn viewer_scroll_resets_only_when_the_shown_exchange_changes() {
        let mut app = crate::app::state::AppState::test_new();
        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        sync_viewer_scroll(&mut app);
        assert_eq!(app.detail_panel_viewer_shown.as_deref(), Some("u1"));
        app.detail_panel_viewer_scroll = 4;
        sync_viewer_scroll(&mut app);
        assert_eq!(app.detail_panel_viewer_scroll, 4);
        // the cycler moving on resets the offset
        app.detail_panel_cycle = 1;
        sync_viewer_scroll(&mut app);
        assert_eq!(app.detail_panel_viewer_scroll, 0);
        // so does a pin landing on a different exchange
        app.detail_panel_viewer_scroll = 2;
        app.detail_panel_pinned = Some("u3".into());
        sync_viewer_scroll(&mut app);
        assert_eq!(app.detail_panel_viewer_scroll, 0);
        assert_eq!(app.detail_panel_viewer_shown.as_deref(), Some("u3"));
    }

    #[test]
    fn open_detail_panel_reserves_a_right_column_and_renders() {
        use ratatui::{backend::TestBackend, Terminal};

        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.active = Some(0);
        app.detail_panel_open = true;
        let area = Rect::new(0, 0, 180, 40);
        crate::ui::compute_view(&mut app, area);

        let rect = app.view.detail_panel_rect;
        assert_eq!(rect.width, 46);
        assert_eq!(rect.x + rect.width, 180);
        assert!(app.view.terminal_area.width >= 60);

        app.detail_panel = Some(
            crate::app::detail_panel::DetailPanelCache::test_with_timeline(test_timeline()),
        );
        let mut terminal =
            Terminal::new(TestBackend::new(180, 40)).expect("test terminal should initialize");
        terminal
            .draw(|frame| render_detail_panel(&app, frame, rect))
            .expect("detail panel should render");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(rect.x, 5)].symbol(), "│");
        let content = content_rect(rect);
        // tasks header sits above the item rows, same shape as subagents
        let header: String = (content.x..content.x + 9)
            .map(|x| buffer[(x, content.y + 2)].symbol())
            .collect();
        assert_eq!(header, "─ tasks ─");
        assert_eq!(
            buffer[(content.x, content.y + 2)].style().fg,
            Some(app.palette.overlay0)
        );
        assert_eq!(buffer[(content.x, content.y + 3)].symbol(), "✓");
        assert_eq!(
            buffer[(content.x, content.y + 3)].style().fg,
            Some(app.palette.green)
        );
        // unlabeled item renders the placeholder, dim, not raw prompt words
        assert_eq!(buffer[(content.x + 2, content.y + 4)].symbol(), "s");
        assert_eq!(
            buffer[(content.x + 2, content.y + 4)].style().fg,
            Some(app.palette.subtext0)
        );
        assert_eq!(
            buffer[(content.x, content.y + 5)].style().fg,
            Some(app.palette.yellow)
        );
        assert_eq!(
            buffer[(content.x, content.y + 6)].style().fg,
            Some(app.palette.mauve)
        );
        // subagent board: live lane yellow, finished lane green with runtime
        assert_eq!(
            buffer[(content.x, content.y + 9)].style().fg,
            Some(app.palette.yellow)
        );
        assert_eq!(
            buffer[(content.x, content.y + 10)].style().fg,
            Some(app.palette.green)
        );
        assert_eq!(
            buffer[(content.x + content.width - 3, content.y + 10)].symbol(),
            "4"
        );
        assert_eq!(
            buffer[(content.x + content.width - 1, content.y + 10)].symbol(),
            "s"
        );
        // prompt viewer: header, meta with position, label, head, dim rhead
        let (_, viewer) = split_content(content);
        let row_text = |y: u16| -> String {
            (viewer.x..viewer.x + viewer.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        };
        assert!(row_text(viewer.y).starts_with("─ prompts ─"));
        assert_eq!(
            buffer[(viewer.x, viewer.y)].style().fg,
            Some(app.palette.overlay0)
        );
        assert!(row_text(viewer.y + 1).contains("5m · [‹] 1/3 [›] · [jump]"));
        // at the oldest exchange the previous arrow renders exhausted-dim
        let (idx, views) = viewer_state(&app).expect("a shown exchange");
        for (range, hit) in viewer_meta(&app, idx, &views).1 {
            let fg = buffer[(viewer.x + range.start as u16, viewer.y + 1)].style().fg;
            match hit {
                DetailPanelHit::ViewerPrev { u } => {
                    assert_eq!((u, fg), (None, Some(app.palette.overlay0)));
                }
                DetailPanelHit::ViewerNext { u } => {
                    assert_eq!((u, fg), (Some("u2".into()), Some(app.palette.text)));
                }
                DetailPanelHit::ViewerJump { query } => {
                    assert_eq!(query.as_deref(), Some("please restore both configs"));
                    assert_eq!(fg, Some(app.palette.text));
                }
                other => panic!("unexpected meta button {other:?}"),
            }
        }
        assert_eq!(buffer[(viewer.x, viewer.y + 2)].symbol(), "✓");
        assert_eq!(
            buffer[(viewer.x, viewer.y + 2)].style().fg,
            Some(app.palette.green)
        );
        assert!(row_text(viewer.y + 3).starts_with("please restore both configs"));
        assert_eq!(
            buffer[(viewer.x, viewer.y + 3)].style().fg,
            Some(app.palette.text)
        );
        assert!(row_text(viewer.y + 5).starts_with("Restored."));
        assert_eq!(
            buffer[(viewer.x, viewer.y + 5)].style().fg,
            Some(app.palette.subtext0)
        );
    }

    #[test]
    fn closed_or_narrow_layouts_reserve_no_detail_column() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        app.active = Some(0);
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 180, 40));
        assert_eq!(app.view.detail_panel_rect, Rect::default());

        app.detail_panel_open = true;
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 100, 40));
        assert_eq!(app.view.detail_panel_rect, Rect::default());
    }
}

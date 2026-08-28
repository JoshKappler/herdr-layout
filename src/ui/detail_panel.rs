use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width, truncate_end};
use crate::app::detail_panel::Timeline;
use crate::app::state::Palette;
use crate::app::AppState;

/// Right-side drill-in for the focused tab (Josh 2026-08-27): one aggregate
/// line, then the task timeline: done in green with per-task time, in
/// flight in yellow with a running clock, inferred next steps in purple.
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

    let content = content_rect(area);
    let lines = detail_panel_lines(app, content.width);
    let scroll = app.detail_panel_scroll.min(max_scroll_for(&lines, content)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content);
}

fn content_rect(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(3),
        area.height.saturating_sub(1),
    )
}

pub(crate) fn detail_panel_max_scroll(app: &AppState, area: Rect) -> usize {
    if area.width < 4 || area.height < 2 {
        return 0;
    }
    let content = content_rect(area);
    max_scroll_for(&detail_panel_lines(app, content.width), content)
}

fn max_scroll_for(lines: &[Line<'_>], content: Rect) -> usize {
    lines.len().saturating_sub(content.height as usize)
}

/// What a click on a panel line should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DetailPanelHit {
    JumpFeed { query: String },
    ToggleSub { path: String },
}

pub(crate) fn detail_panel_hit(app: &AppState, area: Rect, row: u16) -> Option<DetailPanelHit> {
    let content = content_rect(area);
    if row < content.y || row >= content.y + content.height {
        return None;
    }
    let (lines, hits) = detail_panel_lines_with_hits(app, content.width);
    let scroll = app.detail_panel_scroll.min(max_scroll_for(&lines, content));
    let line_idx = (row - content.y) as usize + scroll;
    hits.into_iter().nth(line_idx).flatten()
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
    for item in &tl.done {
        lines.push(item_line(
            "✓",
            &item.label,
            item.secs.map(fmt_secs),
            p.green,
            width,
            p,
        ));
        hits.push(feed_query(&item.head).map(|query| DetailPanelHit::JumpFeed { query }));
    }
    for item in &tl.current {
        let running = (item.ts > 0.0).then(|| fmt_secs(now - item.ts));
        lines.push(item_line("●", &item.label, running, p.yellow, width, p));
        hits.push(feed_query(&item.head).map(|query| DetailPanelHit::JumpFeed { query }));
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
            // finished lanes stay listed: teal like a done tab, total runtime
            let (color, clock) = if sub.status == "done" {
                let total = (sub.started > 0.0 && sub.ended > sub.started)
                    .then(|| fmt_secs(sub.ended - sub.started));
                (p.teal, total)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::detail_panel::TimelineItem;

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
        assert_eq!(detail_panel_hit(&app, rect, content.y), None);
        assert_eq!(
            detail_panel_hit(&app, rect, content.y + 1),
            Some(DetailPanelHit::JumpFeed {
                query: "please restored the configs".into()
            })
        );
        // purple prediction row is not clickable
        assert_eq!(detail_panel_hit(&app, rect, content.y + 4), None);
        // blank + header, then live and finished subagent rows both toggle
        assert_eq!(
            detail_panel_hit(&app, rect, content.y + 7),
            Some(DetailPanelHit::ToggleSub {
                path: "/tmp/lane.jsonl".into()
            })
        );
        assert_eq!(
            detail_panel_hit(&app, rect, content.y + 8),
            Some(DetailPanelHit::ToggleSub {
                path: "/tmp/lane-done.jsonl".into()
            })
        );
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
        assert_eq!(buffer[(content.x, content.y + 1)].symbol(), "✓");
        assert_eq!(
            buffer[(content.x, content.y + 1)].style().fg,
            Some(app.palette.green)
        );
        // unlabeled item renders the placeholder, dim, not raw prompt words
        assert_eq!(
            buffer[(content.x + 2, content.y + 2)].symbol(),
            "s"
        );
        assert_eq!(
            buffer[(content.x + 2, content.y + 2)].style().fg,
            Some(app.palette.subtext0)
        );
        assert_eq!(
            buffer[(content.x, content.y + 3)].style().fg,
            Some(app.palette.yellow)
        );
        assert_eq!(
            buffer[(content.x, content.y + 4)].style().fg,
            Some(app.palette.mauve)
        );
        // subagent board: live lane yellow, finished lane teal with runtime
        assert_eq!(
            buffer[(content.x, content.y + 7)].style().fg,
            Some(app.palette.yellow)
        );
        assert_eq!(
            buffer[(content.x, content.y + 8)].style().fg,
            Some(app.palette.teal)
        );
        assert_eq!(
            buffer[(content.x + content.width - 3, content.y + 8)].symbol(),
            "4"
        );
        assert_eq!(
            buffer[(content.x + content.width - 1, content.y + 8)].symbol(),
            "s"
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

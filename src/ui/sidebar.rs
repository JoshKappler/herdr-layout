mod tokens;

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use self::tokens::{ResolvedToken, ResolvedTokenKind};
use super::scrollbar::should_show_scrollbar;
use super::status::{state_dot, state_label};
use super::text::{display_width, display_width_u16, truncate_end};
use crate::app::state::{AgentPanelSort, Palette};
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

// blank buffer row, three button rows, then the section separator line
const WORKSPACE_SECTION_HEADER_ROWS: u16 = 5;
const AGENT_PANEL_HEADER_ROWS: u16 = 3;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub primary_label: String,
    pub primary_tab_label: Option<String>,
    pub pane_label: Option<String>,
    pub terminal_title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    pub agent_label: Option<String>,
    pub agent_kind_label: Option<String>,
    pub agent: Option<crate::detect::Agent>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub state_labels: std::collections::HashMap<String, String>,
    pub tokens: std::collections::HashMap<String, String>,
}

fn sidebar_section_heights(total_h: u16, split_ratio: f32) -> (u16, u16) {
    if total_h == 0 {
        return (0, 0);
    }

    if total_h < 6 {
        let ws_h = total_h.div_ceil(2);
        return (ws_h, total_h.saturating_sub(ws_h));
    }

    let ratio = split_ratio.clamp(0.1, 0.9);
    let ws_h = ((total_h as f32) * ratio).round() as u16;
    let ws_h = ws_h.clamp(3, total_h.saturating_sub(3));
    let detail_h = total_h.saturating_sub(ws_h);
    (ws_h, detail_h)
}

pub(crate) fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), Rect::default());
    }

    let (ws_h, detail_h) = sidebar_section_heights(content.height, split_ratio);
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h);
    let detail_area = Rect::new(content.x, content.y + ws_h, content.width, detail_h);
    (ws_area, detail_area)
}

pub(crate) fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (ws_h, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + ws_h, content.width, 1)
}

fn agent_panel_sort_label(sort: AgentPanelSort) -> &'static str {
    match sort {
        AgentPanelSort::Spaces => "grouped",
        AgentPanelSort::Priority => "priority",
    }
}

pub(crate) fn agent_panel_toggle_rect(area: Rect, sort: AgentPanelSort) -> Rect {
    agent_panel_header_label_rect(area, agent_panel_sort_label(sort))
}

fn agent_panel_header_label_rect(area: Rect, label: &str) -> Rect {
    if area.width == 0 || area.height < 2 {
        return Rect::default();
    }

    let width = display_width_u16(label).min(area.width);
    Rect::new(
        area.x + area.width.saturating_sub(width),
        area.y + 1,
        width,
        1,
    )
}

fn active_agent_view_label(app: &AppState) -> Option<&str> {
    app.agent_view_override
        .as_ref()
        .map(|view| view.label.as_deref().unwrap_or("filtered"))
}

pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn all_agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    collect_agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, Some(terminal_runtimes))
}

fn agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let mut entries = collect_agent_panel_entries_with_runtimes(app, terminal_runtimes);
    crate::app::agent_view::apply_agent_view(app, &mut entries);
    entries
}

fn collect_agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let empty_runtimes;
    let terminal_runtimes = match terminal_runtimes {
        Some(terminal_runtimes) => terminal_runtimes,
        None => {
            empty_runtimes = TerminalRuntimeRegistry::new();
            &empty_runtimes
        }
    };

    app.workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, ws)| {
            let multi_tab = ws.tabs.len() > 1;
            let workspace_label = ws.display_name_from(&app.terminals, terminal_runtimes);
            ws.pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| {
                    let show_tab = multi_tab
                        || ws
                            .tabs
                            .get(detail.tab_idx)
                            .is_some_and(|tab| !tab.is_auto_named());
                    AgentPanelEntry {
                        ws_idx,
                        tab_idx: detail.tab_idx,
                        pane_id: detail.pane_id,
                        primary_label: workspace_label.clone(),
                        primary_tab_label: show_tab.then_some(detail.tab_label),
                        pane_label: detail.pane_label,
                        terminal_title: detail.terminal_title,
                        terminal_title_stripped: detail.terminal_title_stripped,
                        agent_label: Some(detail.agent_label),
                        agent_kind_label: detail.agent_kind_label,
                        agent: detail.agent,
                        state: detail.state,
                        seen: detail.seen,
                        last_agent_state_change_seq: detail.last_agent_state_change_seq,
                        state_labels: detail.state_labels,
                        tokens: detail.tokens,
                    }
                })
        })
        .collect()
}

pub(super) fn agent_panel_status_key(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Working, _) => "working",
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Unknown, _) => "unknown",
    }
}

/// One rendered line of a tab's dashboard block. `tint_at` is the byte
/// offset where the trailing phase clause starts (tinted toward the dot).
pub(crate) enum TabDashRow {
    Title {
        text: String,
        tint_at: Option<usize>,
        since_ms: Option<u64>,
    },
    TitleCont {
        text: String,
        tint_at: Option<usize>,
    },
    Counts(String),
    Lane(String),
}

pub(crate) struct TabDash {
    pub tab_idx: usize,
    pub state: AgentState,
    pub seen: bool,
    /// live subagents under this tab: the dot gets a blue ring while the
    /// parent works so a subagent wait is visible (Josh 2026-08-27)
    pub subs_live: bool,
    pub rows: Vec<TabDashRow>,
}

const TAB_LANE_KEYS: [&str; 7] = ["l1", "l2", "l3", "l4", "l5", "l6", "lmore"];

fn tab_attention(
    app: &AppState,
    tab: &crate::workspace::Tab,
) -> (AgentState, bool, Option<u64>) {
    tab.panes
        .values()
        .filter_map(|pane| {
            let terminal = app.terminals.get(&pane.attached_terminal_id)?;
            Some((terminal.state, pane.seen, terminal.last_agent_state_change_at))
        })
        .max_by_key(|(state, seen, _)| workspace_attention_priority(*state, *seen))
        .unwrap_or((AgentState::Unknown, true, None))
}

fn fmt_elapsed(since_ms: u64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(since_ms);
    let mins = now_ms.saturating_sub(since_ms) / 60_000;
    if mins >= 60 {
        format!("{}h{:02}", mins / 60, mins % 60)
    } else {
        format!("{mins}m")
    }
}

/// Right-hand cells held clear of the title row for the elapsed time + dot.
const TITLE_STATUS_RESERVE: u16 = 8;
/// Rail + box borders + one cell of padding per side around tab text.
const TAB_BOX_CHROME: u16 = 5;
const TAB_TITLE_MAX_LINES: usize = 3;

/// A wrapped line and the byte offset of its first kept char in the source.
struct WrappedLine {
    text: String,
    start: usize,
}

/// Word-wrap prose into up to `max_lines` lines; the last line elides.
fn wrap_prose(text: &str, first: usize, rest: usize, max_lines: usize) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    while offset < text.len() && lines.len() < max_lines {
        let avail = if lines.is_empty() { first } else { rest };
        let remainder = &text[offset..];
        if display_width(remainder) <= avail || lines.len() + 1 == max_lines {
            lines.push(WrappedLine {
                text: truncate_end(remainder, avail),
                start: offset,
            });
            break;
        }
        let mut break_at = None;
        let mut last_fit = 0;
        let mut used = 0usize;
        for (i, ch) in remainder.char_indices() {
            let w = display_width(ch.encode_utf8(&mut [0u8; 4]));
            if used + w > avail {
                break;
            }
            used += w;
            last_fit = i + ch.len_utf8();
            if ch == ' ' {
                break_at = Some(i);
            }
        }
        let head_end = break_at.unwrap_or(last_fit).max(1);
        lines.push(WrappedLine {
            text: remainder[..head_end].to_string(),
            start: offset,
        });
        let skipped = remainder[head_end..].len() - remainder[head_end..].trim_start().len();
        offset += head_end + skipped;
    }
    if lines.is_empty() {
        lines.push(WrappedLine {
            text: String::new(),
            start: 0,
        });
    }
    lines
}

/// Rail color: the space's color tag when set (the space-colors plugin
/// stamps a `c` metadata token), plain text color otherwise.
fn space_rail_color(
    ws: &crate::workspace::Workspace,
    p: &Palette,
) -> ratatui::style::Color {
    match ws.metadata_tokens.values().get("c").map(String::as_str) {
        Some("🟥") => ratatui::style::Color::Rgb(0xc4, 0x4a, 0x58),
        Some("🟦") => ratatui::style::Color::Rgb(0x4a, 0x74, 0xc4),
        Some("🟩") => ratatui::style::Color::Rgb(0x4a, 0xa8, 0x66),
        Some("🟪") => ratatui::style::Color::Rgb(0x8f, 0x66, 0xc4),
        _ => p.text,
    }
}

/// One prose row inside a tab box: optional bold leading "handle:", then the
/// body, then the phase clause tinted toward the state dot's color.
#[allow(clippy::too_many_arguments)]
fn draw_tab_line(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    w: u16,
    text: &str,
    tint_at: Option<usize>,
    bold_handle: bool,
    base: Style,
    tint: Style,
) {
    let split = tint_at.filter(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
    let (pre, tinted) = text.split_at(split);
    let mut segs: Vec<(&str, Style)> = Vec::new();
    match pre.split_once(": ") {
        Some((head, rest)) if bold_handle && head.len() <= 28 => {
            segs.push((head, base.add_modifier(Modifier::BOLD)));
            segs.push((": ", base.add_modifier(Modifier::BOLD)));
            segs.push((rest, base));
        }
        _ => segs.push((pre, base)),
    }
    if !tinted.is_empty() {
        segs.push((tinted, tint));
    }
    let mut cx = x;
    let end = x.saturating_add(w);
    for (seg, style) in segs {
        if cx >= end {
            break;
        }
        if seg.is_empty() {
            continue;
        }
        let shown = truncate_end(seg, (end - cx) as usize);
        buf.set_string(cx, y, &shown, style);
        cx += display_width(&shown) as u16;
    }
}

/// The phase clause leans toward the dot's color without adopting it fully:
/// mostly-white ink with a tint (Josh 2026-08-26).
fn phase_tint(state: AgentState, seen: bool, p: &Palette) -> Style {
    use ratatui::style::Color;
    let color = match (state, seen) {
        (AgentState::Blocked, _) => Color::Rgb(0xe2, 0xae, 0xae),
        (AgentState::Working, _) => Color::Rgb(0xe2, 0xd8, 0xa6),
        (AgentState::Idle, false) => Color::Rgb(0xaa, 0xd4, 0xd4),
        (AgentState::Idle, true) => Color::Rgb(0xb6, 0xd6, 0xb6),
        (AgentState::Unknown, _) => return Style::default().fg(p.overlay1),
    };
    Style::default().fg(color)
}

fn line_tint_at(line: &WrappedLine, tint_start: Option<usize>) -> Option<usize> {
    let ts = tint_start?;
    let at = ts.saturating_sub(line.start).min(line.text.len());
    if ts >= line.start + line.text.len() || !line.text.is_char_boundary(at) {
        return None;
    }
    Some(at)
}

/// Thin line box normally; a highlighted box swaps to half-block edge glyphs
/// so the frame hugs the fill exactly, with no dark margin inside the lines
/// and no fill bleeding past them (Josh 2026-08-27).
fn draw_tab_box_border(
    buf: &mut ratatui::buffer::Buffer,
    bx: u16,
    bw: u16,
    top: u16,
    bottom: u16,
    border: Style,
    filled: bool,
) {
    let (h, v, tl, tr, bl, br) = if filled {
        ("▀", "", "▛", "▜", "▙", "▟")
    } else {
        ("─", "│", "┌", "┐", "└", "┘")
    };
    let bh = if filled { "▄" } else { "─" };
    for x in bx..bx + bw {
        buf[(x, top)].set_symbol(h);
        buf[(x, top)].set_style(border);
        buf[(x, bottom)].set_symbol(bh);
        buf[(x, bottom)].set_style(border);
    }
    buf[(bx, top)].set_symbol(tl);
    buf[(bx + bw - 1, top)].set_symbol(tr);
    buf[(bx, bottom)].set_symbol(bl);
    buf[(bx + bw - 1, bottom)].set_symbol(br);
    for by in top + 1..bottom {
        for (x, side) in [(bx, if filled { "▌" } else { v }), (bx + bw - 1, if filled { "▐" } else { v })] {
            buf[(x, by)].set_symbol(side);
            buf[(x, by)].set_style(border);
        }
    }
}

pub(crate) fn tab_dashboards(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    width: u16,
) -> Vec<TabDash> {
    ws.tabs
        .iter()
        .enumerate()
        .map(|(tab_idx, tab)| {
            let toks = tab.metadata_tokens.values();
            let (state, seen, since_ms) = tab_attention(app, tab);
            let title = toks
                .get("t")
                .or_else(|| toks.get("sh"))
                .filter(|t| !t.is_empty())
                .cloned()
                .or_else(|| {
                    ws.tab_display_name(tab_idx)
                        .filter(|label| !label.chars().all(|c| c.is_ascii_digit()))
                })
                .or_else(|| {
                    tab.panes.values().find_map(|pane| {
                        app.terminals
                            .get(&pane.attached_terminal_id)
                            .and_then(|t| t.terminal_title_stripped())
                            .filter(|t| !t.is_empty())
                    })
                })
                .unwrap_or_else(|| "shell".to_string());
            let phase = toks.get("ph").map(String::as_str).unwrap_or("").trim().to_string();
            let full = if phase.is_empty() {
                title
            } else {
                format!("{title}, {phase}")
            };
            let tint_start = (!phase.is_empty()).then(|| full.len() - phase.len());
            let inner = (width.saturating_sub(TAB_BOX_CHROME) as usize).max(8);
            let first = inner.saturating_sub(TITLE_STATUS_RESERVE as usize).max(8);
            let lines = wrap_prose(&full, first, inner, TAB_TITLE_MAX_LINES);
            let mut rows = Vec::with_capacity(lines.len() + 1);
            for (i, line) in lines.iter().enumerate() {
                let tint_at = line_tint_at(line, tint_start);
                if i == 0 {
                    rows.push(TabDashRow::Title {
                        text: line.text.clone(),
                        tint_at,
                        since_ms,
                    });
                } else {
                    rows.push(TabDashRow::TitleCont {
                        text: line.text.clone(),
                        tint_at,
                    });
                }
            }
            if let Some(s) = toks.get("hdr").filter(|s| !s.is_empty()) {
                rows.push(TabDashRow::Counts(s.clone()));
            }
            for key in TAB_LANE_KEYS {
                if let Some(s) = toks.get(key).filter(|s| !s.is_empty()) {
                    rows.push(TabDashRow::Lane(s.clone()));
                }
            }
            TabDash {
                tab_idx,
                state,
                seen,
                subs_live: toks.get("hdr").is_some_and(|h| h.contains(" sub")),
                rows,
            }
        })
        .collect()
}

/// Only grouped worktree members keep an identity row; spaces carry no
/// titles at all (Josh 2026-08-26 redesign).
fn workspace_name_row_visible(
    app: &AppState,
    ws_idx: usize,
    _ws: &crate::workspace::Workspace,
    indented: bool,
) -> bool {
    indented || workspace_parent_group_state(app, ws_idx).is_some()
}

fn workspace_row_height(
    app: &AppState,
    ws_idx: usize,
    ws: &crate::workspace::Workspace,
    indented: bool,
    width: u16,
) -> u16 {
    let dashes = tab_dashboards(app, ws, width);
    // each tab is its own bordered box: content rows plus two border rows
    let boxed: usize = dashes.iter().map(|d| d.rows.len() + 2).sum();
    let content = usize::from(workspace_name_row_visible(app, ws_idx, ws, indented)) + boxed;
    content.max(3).min(u16::MAX as usize) as u16
}

fn workspace_row_height_in_body(
    app: &AppState,
    ws_idx: usize,
    workspace: &crate::workspace::Workspace,
    indented: bool,
    body: Rect,
) -> u16 {
    workspace_row_height(app, ws_idx, workspace, indented, body.width).min(body.height)
}

fn workspace_entry_gap(
    app: &AppState,
    entries: &[WorkspaceListEntry],
    entry_idx: usize,
    indented: bool,
) -> u16 {
    if entry_idx + 1 < entries.len()
        && !(indented && next_entry_is_indented_workspace(entries, entry_idx))
    {
        app.sidebar_spaces.row_gap
    } else {
        0
    }
}

fn workspace_attention_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    }
}

fn space_aggregate_state(app: &AppState, key: &str) -> (AgentState, bool) {
    app.workspaces
        .iter()
        .filter(|ws| ws.worktree_space().is_some_and(|space| space.key == key))
        .map(|ws| ws.aggregate_state(&app.terminals))
        .max_by_key(|(state, seen)| workspace_attention_priority(*state, *seen))
        .unwrap_or((AgentState::Unknown, true))
}

pub(crate) fn workspace_parent_group_state(
    app: &AppState,
    ws_idx: usize,
) -> Option<(String, bool)> {
    let space = app.workspaces.get(ws_idx)?.worktree_space()?;
    if space.is_linked_worktree {
        return None;
    }
    let member_count = app
        .workspaces
        .iter()
        .filter(|ws| {
            ws.worktree_space()
                .is_some_and(|member| member.key == space.key)
        })
        .count();
    (member_count >= 2).then(|| {
        (
            space.key.clone(),
            app.collapsed_space_keys.contains(&space.key),
        )
    })
}

pub(crate) fn grouped_child_display_label(
    label: &str,
    branch: Option<&str>,
    has_custom_name: bool,
) -> String {
    if has_custom_name {
        return label.to_string();
    }
    let Some(branch) = branch else {
        return label.to_string();
    };
    branch
        .strip_prefix("worktree/")
        .unwrap_or(branch)
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceListEntry {
    Workspace { ws_idx: usize, indented: bool },
}

pub(crate) fn next_entry_is_indented_workspace(entries: &[WorkspaceListEntry], idx: usize) -> bool {
    matches!(
        entries.get(idx.saturating_add(1)),
        Some(WorkspaceListEntry::Workspace { indented: true, .. })
    )
}

pub(crate) fn normalized_workspace_scroll(app: &AppState, area: Rect, requested: usize) -> usize {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    let body = workspace_list_body_rect(ws_area, false);
    if body.height == 0 {
        return requested;
    }

    if workspace_list_entries(app).is_empty() {
        0
    } else {
        requested.min(workspace_list_bottom_start(app, ws_area))
    }
}

pub(crate) fn workspace_list_entries(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, false)
}

/// Like [`workspace_list_entries`] but always expands worktree groups, ignoring
/// `collapsed_space_keys`. The mobile switcher has no collapse affordance and
/// always shows the full worktree tree.
pub(crate) fn workspace_list_entries_expanded(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, true)
}

fn workspace_list_entries_inner(app: &AppState, force_expanded: bool) -> Vec<WorkspaceListEntry> {
    let mut members_by_key = std::collections::HashMap::<String, Vec<usize>>::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        if let Some(space) = ws.worktree_space() {
            members_by_key
                .entry(space.key.clone())
                .or_default()
                .push(ws_idx);
        }
    }
    let grouped_keys = members_by_key
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().any(|idx| {
                    app.workspaces
                        .get(*idx)
                        .and_then(|ws| ws.worktree_space())
                        .is_some_and(|space| !space.is_linked_worktree)
                })
        })
        .map(|(key, _)| key.clone())
        .collect::<std::collections::HashSet<_>>();

    let visible_group_idx = if matches!(app.mode, Mode::Navigate) {
        Some(app.selected)
    } else {
        app.active
    };
    let active_group = visible_group_idx.and_then(|idx| {
        app.workspaces
            .get(idx)
            .and_then(|ws| ws.worktree_space())
            .map(|space| space.key.clone())
    });

    let mut emitted_groups = std::collections::HashSet::<String>::new();
    let mut entries = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        let Some(space) = ws
            .worktree_space()
            .filter(|space| grouped_keys.contains(&space.key))
        else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };

        if !emitted_groups.insert(space.key.clone()) {
            continue;
        }

        let Some(members) = members_by_key.get(&space.key) else {
            continue;
        };
        let Some(parent_idx) = members.iter().copied().find(|idx| {
            app.workspaces
                .get(*idx)
                .and_then(|member| member.worktree_space())
                .is_some_and(|member_space| !member_space.is_linked_worktree)
        }) else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };
        let collapsed = !force_expanded && app.collapsed_space_keys.contains(&space.key);
        entries.push(WorkspaceListEntry::Workspace {
            ws_idx: parent_idx,
            indented: false,
        });

        if collapsed {
            if let Some(active_idx) = visible_group_idx
                .filter(|idx| *idx != parent_idx)
                .filter(|_| active_group.as_deref() == Some(space.key.as_str()))
            {
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: active_idx,
                    indented: true,
                });
            }
        } else {
            for member_idx in members {
                if *member_idx == parent_idx {
                    continue;
                }
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: *member_idx,
                    indented: true,
                });
            }
        }
    }
    entries
}

pub(crate) fn workspace_list_rect(area: Rect, _split_ratio: f32) -> Rect {
    // unified sidebar (Josh 2026-08-26): the space list owns the full column
    Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height)
}

pub(crate) fn workspace_list_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(WORKSPACE_SECTION_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn workspace_list_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = workspace_list_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        let (row_height, gap) = match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                (
                    workspace_row_height_in_body(app, *ws_idx, ws, *indented, body),
                    workspace_entry_gap(app, &entries, entry_idx, *indented),
                )
            }
        };
        if used_rows.saturating_add(row_height) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(row_height);
        visible += 1;
        used_rows = used_rows.saturating_add(gap).min(body.height);
    }
    visible
}

fn workspace_list_bottom_start(app: &AppState, area: Rect) -> usize {
    let body = workspace_list_body_rect(area, false);
    let entries = workspace_list_entries(app);
    let mut used_rows = 0u16;
    let mut start = entries.len();
    for (entry_idx, entry) in entries.iter().enumerate().rev() {
        let WorkspaceListEntry::Workspace { ws_idx, indented } = entry;
        let Some(workspace) = app.workspaces.get(*ws_idx) else {
            continue;
        };
        let gap = workspace_entry_gap(app, &entries, entry_idx, *indented);
        let needed = workspace_row_height_in_body(app, *ws_idx, workspace, *indented, body)
            .saturating_add(gap);
        if used_rows.saturating_add(needed) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(needed);
        start = entry_idx;
    }
    start.min(entries.len().saturating_sub(1))
}

pub(crate) fn workspace_list_scroll_metrics(
    app: &AppState,
    area: Rect,
) -> crate::pane::ScrollMetrics {
    let max_scroll = workspace_list_bottom_start(app, area);
    let scroll = app.workspace_scroll.min(max_scroll);
    let viewport_rows = workspace_list_visible_count(app, area, scroll);

    crate::pane::ScrollMetrics {
        offset_from_bottom: max_scroll.saturating_sub(scroll),
        max_offset_from_bottom: max_scroll,
        viewport_rows,
    }
}

pub(crate) fn workspace_list_scrollbar_rect(_app: &AppState, _area: Rect) -> Option<Rect> {
    // the list wheel-scrolls with no visible indicator (Josh 2026-08-26)
    None
}

pub(crate) fn agent_panel_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= AGENT_PANEL_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(AGENT_PANEL_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn resolved_agent_rows(app: &AppState, entry: &AgentPanelEntry) -> Vec<Vec<ResolvedToken>> {
    let label = entry
        .state_labels
        .get(agent_panel_status_key(entry.state, entry.seen))
        .map(String::as_str)
        .unwrap_or_else(|| state_label(entry.state, entry.seen));
    tokens::agent_rows(&app.sidebar_agents, entry, label)
}

pub(crate) fn agent_entry_height_in_body(
    app: &AppState,
    entry: &AgentPanelEntry,
    body_height: u16,
) -> u16 {
    (resolved_agent_rows(app, entry)
        .len()
        .max(1)
        .min(u16::MAX as usize) as u16)
        .min(body_height)
}

pub(crate) fn agent_entry_gap(app: &AppState, entry_idx: usize, entry_count: usize) -> u16 {
    if entry_idx + 1 < entry_count {
        app.sidebar_agents.row_gap
    } else {
        0
    }
}

fn agent_panel_visible_count_from(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = agent_panel_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    let entries = agent_panel_entries(app);
    for (index, entry) in entries.iter().enumerate().skip(scroll) {
        let height = agent_entry_height_in_body(app, entry, body.height);
        if used_rows.saturating_add(height) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(height);
        visible += 1;
        used_rows = used_rows
            .saturating_add(agent_entry_gap(app, index, entries.len()))
            .min(body.height);
    }
    visible
}

fn agent_panel_bottom_start(app: &AppState, area: Rect) -> usize {
    let body = agent_panel_body_rect(area, false);
    let entries = agent_panel_entries(app);
    let mut used_rows = 0u16;
    let mut start = entries.len();
    for (index, entry) in entries.iter().enumerate().rev() {
        let gap = agent_entry_gap(app, index, entries.len());
        let needed = agent_entry_height_in_body(app, entry, body.height).saturating_add(gap);
        if used_rows.saturating_add(needed) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(needed);
        start = index;
    }
    start.min(entries.len().saturating_sub(1))
}

pub(crate) fn agent_panel_scroll_for_target(
    app: &AppState,
    area: Rect,
    current_scroll: usize,
    target: usize,
) -> usize {
    let max_scroll = agent_panel_bottom_start(app, area);
    if target < current_scroll {
        return target.min(max_scroll);
    }
    let mut scroll = current_scroll.min(max_scroll);
    while scroll < target {
        let visible = agent_panel_visible_count_from(app, area, scroll);
        if visible > 0 && target < scroll.saturating_add(visible) {
            break;
        }
        scroll += 1;
    }
    scroll.min(max_scroll)
}

pub(crate) fn agent_panel_scroll_metrics(app: &AppState, area: Rect) -> crate::pane::ScrollMetrics {
    let max_scroll = agent_panel_bottom_start(app, area);
    let scroll = app.agent_panel_scroll.min(max_scroll);
    let viewport_rows = agent_panel_visible_count_from(app, area, scroll);

    crate::pane::ScrollMetrics {
        offset_from_bottom: max_scroll.saturating_sub(scroll),
        max_offset_from_bottom: max_scroll,
        viewport_rows,
    }
}

pub(crate) fn agent_panel_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = agent_panel_scroll_metrics(app, area);
    let body = agent_panel_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn compute_workspace_list_areas(
    app: &AppState,
    area: Rect,
) -> (
    Vec<crate::app::state::WorkspaceCardArea>,
    Vec<crate::app::state::SidebarTabRow>,
) {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    if ws_area == Rect::default() {
        return (Vec::new(), Vec::new());
    }

    let body = workspace_list_body_rect(ws_area, false);
    if body.width == 0 || body.height == 0 {
        return (Vec::new(), Vec::new());
    }

    let scroll = app.workspace_scroll;
    let mut row_y = body.y;
    let body_bottom = body.y + body.height;
    let mut cards = Vec::new();
    let mut tab_rows = Vec::new();

    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                let mut row_height =
                    workspace_row_height_in_body(app, *ws_idx, ws, *indented, body);
                let gap = workspace_entry_gap(app, &entries, entry_idx, *indented);
                // a space that only part-fits still fills the remaining rows
                // instead of leaving blank screen (Josh 2026-08-26)
                let clipped = row_y.saturating_add(row_height) > body_bottom;
                if clipped {
                    row_height = body_bottom.saturating_sub(row_y);
                    if row_height == 0 {
                        break;
                    }
                }
                cards.push(crate::app::state::WorkspaceCardArea {
                    ws_idx: *ws_idx,
                    rect: Rect::new(body.x, row_y, body.width, row_height),
                    indented: *indented,
                });
                let content_bottom = row_y.saturating_add(row_height).min(body_bottom);
                let mut tab_y =
                    row_y + u16::from(workspace_name_row_visible(app, *ws_idx, ws, *indented));
                for dash in tab_dashboards(app, ws, body.width).iter() {
                    if tab_y >= content_bottom {
                        break;
                    }
                    let height = (dash.rows.len() as u16 + 2).min(content_bottom - tab_y);
                    tab_rows.push(crate::app::state::SidebarTabRow {
                        ws_idx: *ws_idx,
                        tab_idx: dash.tab_idx,
                        rect: Rect::new(body.x, tab_y, body.width, height),
                    });
                    tab_y = tab_y.saturating_add(height);
                }
                if clipped {
                    break;
                }
                row_y = row_y
                    .saturating_add(row_height)
                    .saturating_add(gap)
                    .min(body_bottom);
            }
        }
    }

    (cards, tab_rows)
}

pub(crate) fn compute_workspace_card_areas(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::WorkspaceCardArea> {
    compute_workspace_list_areas(app, area).0
}

const COLLAPSED_BOX_ROWS: u16 = 3;

pub(crate) fn collapsed_minimize_button_rect(area: Rect) -> Rect {
    let content_w = area.width.saturating_sub(1);
    if content_w < 6 || area.height < 4 {
        return Rect::default();
    }
    // one cell in from the left so it lines up with the dot boxes below,
    // which sit just right of the space rail column
    Rect::new(area.x + 1, area.y + 1, 5, 3)
}

/// Minimized sidebar geometry: one 3-row dot box per tab, boxes touching
/// within a space, the usual gap between spaces (Josh 2026-08-26).
pub(crate) fn collapsed_tab_boxes(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::SidebarTabRow> {
    let content_w = area.width.saturating_sub(1);
    if content_w < 6 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Vec::new();
    }
    let mut y = area.y + WORKSPACE_SECTION_HEADER_ROWS;
    let bottom = area.y + area.height;
    let mut boxes = Vec::new();
    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate() {
        let WorkspaceListEntry::Workspace { ws_idx, indented } = entry;
        let Some(ws) = app.workspaces.get(*ws_idx) else {
            continue;
        };
        for (tab_idx, _) in ws.tabs.iter().enumerate() {
            if y.saturating_add(COLLAPSED_BOX_ROWS) > bottom {
                return boxes;
            }
            boxes.push(crate::app::state::SidebarTabRow {
                ws_idx: *ws_idx,
                tab_idx,
                rect: Rect::new(area.x, y, content_w, COLLAPSED_BOX_ROWS),
            });
            y += COLLAPSED_BOX_ROWS;
        }
        y = y
            .saturating_add(workspace_entry_gap(app, &entries, entry_idx, *indented))
            .min(bottom);
    }
    boxes
}

pub(super) fn render_sidebar_collapsed(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let is_navigating = matches!(app.mode, Mode::Navigate);
    let p = &app.palette;
    let sep_style = Style::default().fg(p.text);
    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let button = collapsed_minimize_button_rect(area);
    if button != Rect::default() {
        let plain = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
        render_header_button(frame, button, "◧", plain, p);
    }
    let content_w = area.width.saturating_sub(1);
    if area.height >= WORKSPACE_SECTION_HEADER_ROWS && content_w > 0 {
        let sep_y = area.y + WORKSPACE_SECTION_HEADER_ROWS - 1;
        let buf = frame.buffer_mut();
        for x in area.x..area.x + content_w {
            buf[(x, sep_y)].set_symbol("─");
            buf[(x, sep_y)].set_style(Style::default().fg(p.text));
        }
    }

    for tab_box in collapsed_tab_boxes(app, area) {
        let Some(ws) = app.workspaces.get(tab_box.ws_idx) else {
            continue;
        };
        let Some(tab) = ws.tabs.get(tab_box.tab_idx) else {
            continue;
        };
        let (state, seen, _) = tab_attention(app, tab);
        let selected = tab_box.ws_idx == app.selected && is_navigating;
        let tab_is_active =
            Some(tab_box.ws_idx) == app.active && tab_box.tab_idx == ws.active_tab;
        let rect = tab_box.rect;
        let bx = rect.x + 1;
        let bw = rect.width.saturating_sub(1);
        if bw < 5 {
            break;
        }
        let buf = frame.buffer_mut();
        let box_bg = if selected {
            Some(p.surface0)
        } else if tab_is_active {
            Some(p.surface_dim)
        } else {
            None
        };
        if let Some(bg) = box_bg {
            // border cells too: their glyphs are thin lines, so an interior-only
            // fill leaves a dark margin inside the box (Josh 2026-08-27)
            for by in rect.y..rect.y + rect.height {
                for x in bx..bx + bw {
                    buf[(x, by)].set_style(Style::default().bg(bg));
                }
            }
        }
        let border = Style::default().fg(p.overlay0);
        let top = rect.y;
        let bottom = rect.y + rect.height - 1;
        draw_tab_box_border(buf, bx, bw, top, bottom, border, box_bg.is_some());
        let rail = Style::default().fg(space_rail_color(ws, p));
        for by in rect.y..rect.y + rect.height {
            buf[(rect.x, by)].set_symbol("▌");
            buf[(rect.x, by)].set_style(rail);
        }
        let (icon, icon_style) = state_dot(state, seen, p);
        let subs_live = tab
            .metadata_tokens
            .values()
            .get("hdr")
            .is_some_and(|h| h.contains(" sub"));
        if subs_live && matches!(state, AgentState::Working) {
            let ring = Style::default().fg(p.blue);
            buf.set_string(bx + 1, top + 1, "(", ring);
            buf.set_string(bx + 2, top + 1, icon, icon_style);
            buf.set_string(bx + 3, top + 1, ")", ring);
        } else {
            buf.set_string(bx + 2, top + 1, icon, icon_style);
        }
    }

    render_sidebar_toggle(app, frame, area, true, p);
}

/// The row a dragged tab would land on: a gap index points at that box's top
/// border, anything else (cross-space or past the end) at the space's last
/// box bottom border.
pub(crate) fn sidebar_tab_drop_indicator_row(
    rows: &[crate::app::state::SidebarTabRow],
    target_ws: usize,
    insert_idx: Option<usize>,
) -> Option<u16> {
    let boxes: Vec<&crate::app::state::SidebarTabRow> =
        rows.iter().filter(|r| r.ws_idx == target_ws).collect();
    let last = boxes.last()?;
    Some(match insert_idx {
        Some(gap) if gap < boxes.len() => boxes[gap].rect.y,
        _ => last.rect.y + last.rect.height.saturating_sub(1),
    })
}

pub(crate) fn workspace_drop_indicator_row(
    cards: &[crate::app::state::WorkspaceCardArea],
    area: Rect,
    insert_idx: usize,
) -> Option<u16> {
    if area.height == 0 {
        return None;
    }
    let list_bottom = area.y + area.height.saturating_sub(1);

    let first = cards.first()?;
    if insert_idx == first.ws_idx {
        return first.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    if let Some(row) = cards
        .last()
        .filter(|card| insert_idx == card.ws_idx.saturating_add(1))
        .map(|card| card.rect.y.saturating_add(card.rect.height))
        .filter(|y| *y < list_bottom)
    {
        return Some(row);
    }

    if let Some(card) = cards.iter().find(|card| card.ws_idx == insert_idx) {
        return card.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    None
}

pub(super) fn render_sidebar(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;
    let is_navigating = matches!(app.mode, Mode::Navigate);
    let sep_style = Style::default().fg(p.text);

    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let ws_area = workspace_list_rect(area, app.sidebar_section_split);

    render_workspace_list(app, terminal_runtimes, frame, ws_area, is_navigating);
    render_sidebar_toggle(app, frame, area, false, p);
}

fn resolved_token_spans(
    resolved: &[ResolvedToken],
    state_icon: (&str, Style),
    state_text_style: Style,
    workspace_style: Style,
    secondary_style: Style,
    custom_style: Style,
    p: &Palette,
    max_width: usize,
) -> Vec<Span<'static>> {
    let fixed_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateIcon => display_width(state_icon.0),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                usize::from(*ahead > 0) * display_width(&format!("↑{ahead}"))
                    + usize::from(*behind > 0) * display_width(&format!("↓{behind}"))
                    + usize::from(*ahead > 0 && *behind > 0)
            }
            _ => 0,
        })
        .collect::<Vec<_>>();
    let flexible_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateText(text)
            | ResolvedTokenKind::Workspace(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::TerminalTitle(text)
            | ResolvedTokenKind::Branch(text)
            | ResolvedTokenKind::Custom(text) => display_width(text),
            _ => 0,
        })
        .collect::<Vec<_>>();
    let minimum_width = |active: &[bool]| {
        let indices = active
            .iter()
            .enumerate()
            .filter_map(|(index, active)| active.then_some(index))
            .collect::<Vec<_>>();
        let content = indices
            .iter()
            .map(|index| fixed_widths[*index] + usize::from(flexible_widths[*index] > 0))
            .sum::<usize>();
        let separators = indices
            .windows(2)
            .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
            .sum::<usize>();
        content + separators
    };
    let mut active = resolved.iter().map(|_| true).collect::<Vec<_>>();
    if minimum_width(&active) > max_width {
        for (index, width) in flexible_widths.iter().enumerate() {
            if *width > 0 {
                active[index] = false;
            }
        }
        for index in (0..resolved.len()).rev() {
            if flexible_widths[index] == 0 {
                continue;
            }
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
    }
    let visible_indices = active
        .iter()
        .enumerate()
        .filter_map(|(index, active)| active.then_some(index))
        .collect::<Vec<_>>();
    let separator_width = visible_indices
        .windows(2)
        .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
        .sum::<usize>();
    let fixed_width = visible_indices
        .iter()
        .map(|index| fixed_widths[*index])
        .sum::<usize>();
    let mut budgets = flexible_widths
        .iter()
        .enumerate()
        .map(|(index, width)| usize::from(active[index] && *width > 0))
        .collect::<Vec<_>>();
    let minimum = budgets.iter().sum::<usize>();
    let mut remaining = max_width
        .saturating_sub(separator_width + fixed_width)
        .saturating_sub(minimum);
    while remaining > 0 {
        let mut grew = false;
        for (budget, width) in budgets.iter_mut().zip(&flexible_widths) {
            if *budget > 0 && *budget < *width {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }
    let mut spans = Vec::new();
    for (position, index) in visible_indices.iter().copied().enumerate() {
        let token = &resolved[index];
        if position > 0 {
            let previous = &resolved[visible_indices[position - 1]];
            spans.push(Span::styled(
                tokens::separator(previous, token),
                Style::default().fg(p.overlay0).add_modifier(Modifier::DIM),
            ));
        }
        match &token.kind {
            ResolvedTokenKind::StateIcon => {
                spans.push(Span::styled(
                    state_icon.0.to_string(),
                    apply_token_style(state_icon.1, token.style),
                ));
            }
            ResolvedTokenKind::StateText(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(state_text_style, token.style),
                ));
            }
            ResolvedTokenKind::Workspace(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(workspace_style, token.style),
                ));
            }
            ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::Branch(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(secondary_style, token.style),
                ));
            }
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        apply_token_style(Style::default().fg(p.green), token.style),
                    ));
                }
                if *ahead > 0 && *behind > 0 {
                    spans.push(Span::styled(
                        " ",
                        apply_token_style(Style::default(), token.style),
                    ));
                }
                if *behind > 0 {
                    spans.push(Span::styled(
                        format!("↓{behind}"),
                        apply_token_style(Style::default().fg(p.red), token.style),
                    ));
                }
            }
            ResolvedTokenKind::TerminalTitle(text) | ResolvedTokenKind::Custom(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(custom_style, token.style),
                ));
            }
        }
    }
    spans
}

fn apply_token_style(mut style: Style, patch: crate::config::SidebarTokenStyle) -> Style {
    if let Some(fg) = patch.fg {
        style = style.fg(fg.ratatui());
    }
    if let Some(bold) = patch.bold {
        style = if bold {
            style.add_modifier(Modifier::BOLD)
        } else {
            style.remove_modifier(Modifier::BOLD)
        };
    }
    if let Some(dim) = patch.dim {
        style = if dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style.remove_modifier(Modifier::DIM)
        };
    }
    style
}

fn render_header_button(frame: &mut Frame, rect: Rect, label: &str, style: Style, p: &Palette) {
    if rect.width < 3 || rect.height < 3 {
        return;
    }
    let border = Style::default().fg(p.text);
    let horizontal = "─".repeat(rect.width.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(format!("┌{horizontal}┐"), border)),
        Rect::new(rect.x, rect.y, rect.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(label.to_string(), style)).alignment(Alignment::Center),
        Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 1),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(format!("└{horizontal}┘"), border)),
        Rect::new(rect.x, rect.y + 2, rect.width, 1),
    );
    let buf = frame.buffer_mut();
    for x in [rect.x, rect.x + rect.width - 1] {
        buf[(x, rect.y + 1)].set_symbol("│");
        buf[(x, rect.y + 1)].set_style(border);
    }
}

fn render_workspace_list(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
    is_navigating: bool,
) {
    let p = &app.palette;
    let dragged_ws_idx = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder { source_ws_idx, .. }) => {
            Some(*source_ws_idx)
        }
        _ => None,
    };
    let insertion_row = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder {
            insert_idx: Some(insert_idx),
            ..
        }) => workspace_drop_indicator_row(&app.view.workspace_card_areas, area, *insert_idx),
        Some(crate::app::state::DragTarget::SidebarTabMove {
            target_ws_idx: Some(target),
            insert_idx,
            ..
        }) => sidebar_tab_drop_indicator_row(&app.view.sidebar_tab_rows, *target, *insert_idx),
        _ => None,
    };

    let list_bottom = area.y + area.height;
    if area.height >= 4 {
        let plain = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
        render_header_button(frame, app.sidebar_minimize_button_rect(), "◧", plain, p);
        render_header_button(frame, app.sidebar_new_tab_button_rect(), "new tab", plain, p);
        render_header_button(frame, app.sidebar_new_button_rect(), "new space", plain, p);
        let menu_style = if app.global_menu_attention_badge_visible() {
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
        } else {
            plain
        };
        render_header_button(frame, app.global_launcher_rect(), "menu", menu_style, p);
        render_header_button(frame, app.sidebar_panel_toggle_rect(), "◨", plain, p);
    }
    if area.height >= WORKSPACE_SECTION_HEADER_ROWS {
        let sep_y = area.y + WORKSPACE_SECTION_HEADER_ROWS - 1;
        let buf = frame.buffer_mut();
        for x in area.x..area.x + area.width {
            buf[(x, sep_y)].set_symbol("─");
            buf[(x, sep_y)].set_style(Style::default().fg(p.text));
        }
    }

    let cards = &app.view.workspace_card_areas;

    for card in cards {
        let i = card.ws_idx;
        let ws = &app.workspaces[i];
        let row_y = card.rect.y;
        let row_height = card.rect.height;
        let content_height = row_height;
        let selected = i == app.selected && is_navigating;
        let is_active = Some(i) == app.active;
        let is_dragged = dragged_ws_idx == Some(i);
        let is_drop_target = matches!(
            app.drag.as_ref().map(|drag| &drag.target),
            Some(crate::app::state::DragTarget::SidebarTabMove {
                source_ws_idx,
                target_ws_idx: Some(target),
                ..
            }) if *target == i && *source_ws_idx != i
        );
        let name_style = if selected || is_active || is_dragged {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0)
        };

        let label = ws.display_name_from(&app.terminals, terminal_runtimes);
        let display_label = if card.indented {
            grouped_child_display_label(&label, ws.branch().as_deref(), ws.custom_name.is_some())
        } else {
            label
        };
        let parent_group = (!card.indented)
            .then(|| workspace_parent_group_state(app, i))
            .flatten();

        let content_bottom = (row_y + content_height).min(list_bottom);
        let mut y = row_y;
        if workspace_name_row_visible(app, i, ws, card.indented) && y < content_bottom {
            let mut spans = Vec::new();
            let prefix_width = if let Some((_, collapsed)) = parent_group.as_ref() {
                spans.push(Span::styled(
                    if *collapsed { "▸" } else { "▾" },
                    Style::default().fg(p.accent),
                ));
                spans.push(Span::raw(" "));
                2
            } else {
                spans.push(Span::raw(" "));
                1
            };
            let style = name_style;
            spans.push(Span::styled(
                truncate_end(
                    &display_label,
                    card.rect.width.saturating_sub(prefix_width + 1) as usize,
                ),
                style,
            ));
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(card.rect.x, y, card.rect.width, 1),
            );
            y = y.saturating_add(1);
        }

        let collapsed_group = parent_group
            .as_ref()
            .is_some_and(|(_, collapsed)| *collapsed);
        let dashes = if collapsed_group {
            Vec::new()
        } else {
            tab_dashboards(app, ws, card.rect.width)
        };
        let rail = Style::default().fg(space_rail_color(ws, p));
        let buf = frame.buffer_mut();
        for dash in dashes.iter() {
            if y >= content_bottom {
                break;
            }
            let box_h = (dash.rows.len() as u16 + 2).min(content_bottom - y);
            let bx = card.rect.x + 1;
            let bw = card.rect.width.saturating_sub(1);
            if bw < 5 || box_h < 2 {
                break;
            }
            let tab_is_active = is_active && dash.tab_idx == ws.active_tab;
            // fill the whole box, border cells included: the line glyphs are
            // thin, so stopping at the interior leaves a dark margin inside
            // the box (Josh 2026-08-27)
            let box_bg = if selected {
                Some(p.surface0)
            } else if is_dragged || is_drop_target {
                Some(p.surface1)
            } else if tab_is_active {
                Some(p.surface_dim)
            } else {
                None
            };
            if let Some(bg) = box_bg {
                for by in y..y + box_h {
                    for x in bx..bx + bw {
                        buf[(x, by)].set_style(Style::default().bg(bg));
                    }
                }
            }
            let border = Style::default().fg(p.overlay0);
            let top = y;
            let bottom = y + box_h - 1;
            draw_tab_box_border(buf, bx, bw, top, bottom, border, box_bg.is_some());
            for by in y..y + box_h {
                buf[(card.rect.x, by)].set_symbol("▌");
                buf[(card.rect.x, by)].set_style(rail);
            }

            let dot = state_dot(dash.state, dash.seen, p);
            let phase_style = phase_tint(dash.state, dash.seen, p);
            let base = Style::default().fg(p.text);
            let inner_x = bx + 2;
            let inner_w = bw.saturating_sub(4);
            let mut ry = top + 1;
            for row in &dash.rows {
                if ry >= bottom {
                    break;
                }
                match row {
                    TabDashRow::Title {
                        text,
                        tint_at,
                        since_ms,
                    } => {
                        let elapsed = matches!(dash.state, AgentState::Working)
                            .then_some(*since_ms)
                            .flatten()
                            .map(fmt_elapsed);
                        let ringed =
                            dash.subs_live && matches!(dash.state, AgentState::Working);
                        let status_width = 1
                            + usize::from(ringed) * 2
                            + elapsed.as_ref().map(|e| display_width(e) + 1).unwrap_or(0);
                        let text_w =
                            inner_w.saturating_sub(status_width as u16 + 1);
                        draw_tab_line(
                            buf, inner_x, ry, text_w, text, *tint_at, true, base,
                            phase_style,
                        );
                        let status_x = inner_x
                            + inner_w.saturating_sub(status_width as u16);
                        let mut sx = status_x;
                        if let Some(e) = &elapsed {
                            buf.set_string(sx, ry, e, Style::default().fg(p.subtext0));
                            sx += display_width(e) as u16 + 1;
                        }
                        if ringed {
                            let ring = Style::default().fg(p.blue);
                            buf.set_string(sx, ry, "(", ring);
                            buf.set_string(sx + 1, ry, dot.0, dot.1);
                            buf.set_string(sx + 2, ry, ")", ring);
                        } else {
                            buf.set_string(sx, ry, dot.0, dot.1);
                        }
                    }
                    TabDashRow::TitleCont { text, tint_at } => {
                        draw_tab_line(
                            buf, inner_x, ry, inner_w, text, *tint_at, false, base,
                            phase_style,
                        );
                    }
                    TabDashRow::Counts(text) => {
                        buf.set_string(
                            inner_x,
                            ry,
                            truncate_end(text, inner_w as usize),
                            Style::default().fg(p.overlay1).add_modifier(Modifier::DIM),
                        );
                    }
                    TabDashRow::Lane(text) => {
                        buf.set_string(
                            inner_x,
                            ry,
                            truncate_end(text, inner_w as usize),
                            Style::default().fg(p.overlay0),
                        );
                    }
                }
                ry += 1;
            }
            y = y.saturating_add(box_h);
        }
    }

    if let Some(y) = insertion_row.filter(|y| *y < list_bottom) {
        let buf = frame.buffer_mut();
        for x in area.x..area.x + area.width {
            buf[(x, y)].set_symbol("─");
            buf[(x, y)].set_style(Style::default().fg(p.accent));
        }
    }
}

pub(crate) fn collapsed_sidebar_toggle_rect(area: Rect) -> Rect {
    let bottom_y = area.y + area.height.saturating_sub(1);
    let content_w = area.width.saturating_sub(1);
    if content_w == 0 || area.height == 0 {
        return Rect::default();
    }
    let x = area.x + content_w / 2;
    Rect::new(x, bottom_y, 1, 1)
}

pub(crate) fn expanded_sidebar_toggle_rect(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(
        area.x + area.width.saturating_sub(2),
        area.y + area.height.saturating_sub(1),
        1,
        1,
    )
}

fn render_sidebar_toggle(
    app: &AppState,
    frame: &mut Frame,
    area: Rect,
    collapsed: bool,
    p: &Palette,
) {
    let toggle_area = if collapsed {
        collapsed_sidebar_toggle_rect(area)
    } else {
        expanded_sidebar_toggle_rect(area)
    };
    if toggle_area == Rect::default() {
        return;
    }
    let icon = if collapsed { "»" } else { "«" };
    let icon_style = if collapsed && app.global_menu_attention_badge_visible() {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0)
    };
    frame.render_widget(Paragraph::new(Span::styled(icon, icon_style)), toggle_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{detect::Agent, workspace::Workspace};
    use ratatui::{backend::TestBackend, Terminal};

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
        (0..width)
            .map(|x| buffer[(x, row)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn find_symbol_x(buffer: &ratatui::buffer::Buffer, row: u16, width: u16, symbol: &str) -> u16 {
        (0..width)
            .find(|x| buffer[(*x, row)].symbol() == symbol)
            .unwrap_or_else(|| {
                panic!(
                    "missing symbol {symbol:?} in row {}",
                    row_text(buffer, row, width)
                )
            })
    }

    #[test]
    fn default_space_workspace_style_tracks_active_state() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let first_row = app.view.workspace_card_areas[0].rect.y;
        let second_row = app.view.workspace_card_areas[1].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // spaces carry no titles; each tab is a bordered box behind a rail,
        // and the active tab's whole box, borders included, gets the
        // backdrop (Josh 2026-08-27)
        assert_eq!(buffer[(0, first_row)].symbol(), "▌");
        assert_eq!(buffer[(1, first_row)].symbol(), "▛");

        let active_tab_row = first_row + 1;
        let tab = buffer[(find_symbol_x(buffer, active_tab_row, 25, "s"), active_tab_row)].style();
        assert_eq!(tab.fg, Some(app.palette.text));
        assert_eq!(tab.bg, Some(app.palette.surface_dim));
        let active_border = buffer[(1, first_row)].style();
        assert_eq!(active_border.bg, Some(app.palette.surface_dim));

        let idle_tab_row = second_row + 1;
        let idle = buffer[(find_symbol_x(buffer, idle_tab_row, 25, "s"), idle_tab_row)].style();
        assert_eq!(idle.fg, Some(app.palette.text));
        assert_eq!(idle.bg, Some(ratatui::style::Color::Reset));
    }

    #[test]
    fn occurrence_foreground_flattens_composite_git_status_colors() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#123456" }]]
"##,
        )
        .unwrap();
        let spans = resolved_token_spans(
            &[ResolvedToken {
                kind: ResolvedTokenKind::GitStatus {
                    ahead: 2,
                    behind: 1,
                },
                style: config.ui.sidebar.spaces.rows[0][0].parts().1,
            }],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            20,
        );

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "↑2 ↓1"
        );
        assert!(spans
            .iter()
            .all(|span| { span.style.fg == Some(ratatui::style::Color::Rgb(0x12, 0x34, 0x56)) }));
    }

    #[test]
    fn stripped_terminal_title_renders_with_unicode_width_truncation() {
        let app = crate::app::state::AppState::test_new();

        let spans = resolved_token_spans(
            &[ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle(
                "修复🙂标题很长".into(),
            ))],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &app.palette,
            8,
        );
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(display_width(&text) <= 8, "resolved title: {text:?}");
    }

    #[test]
    fn variable_agent_heights_pack_the_bottom_and_reveal_targets() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
        ];
        app.ensure_test_terminals();
        for workspace in &app.workspaces {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);
        }
        let first_pane = app.workspaces[0].tabs[0].root_pane;
        let first_terminal = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal)
            .unwrap()
            .metadata_tokens
            .patch(
                std::collections::HashMap::from([
                    ("a".into(), Some("a".into())),
                    ("b".into(), Some("b".into())),
                ]),
                None,
                std::time::Instant::now(),
            );
        app.sidebar_agents.rows = vec![
            vec![crate::config::AgentSidebarToken::Agent],
            vec![crate::config::AgentSidebarToken::Custom("a".into())],
            vec![crate::config::AgentSidebarToken::Custom("b".into())],
        ];
        let area = Rect::new(0, 0, 20, 6);

        let metrics = agent_panel_scroll_metrics(&app, area);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(agent_panel_scroll_for_target(&app, area, 0, 2), 1);
    }

    #[test]
    fn oversized_space_layout_is_clipped_to_the_section_body() {
        let mut app = crate::app::state::AppState::test_new();
        let mut big = Workspace::test_new("one");
        for i in 0..12 {
            big.test_add_tab(Some(&format!("tab-{i}")));
        }
        app.workspaces = vec![big, Workspace::test_new("two")];
        let area = Rect::new(0, 0, 20, 10);
        let workspace_area = workspace_list_rect(area, app.sidebar_section_split);
        let body = workspace_list_body_rect(workspace_area, false);

        let metrics = workspace_list_scroll_metrics(&app, workspace_area);
        let (cards, _) = compute_workspace_list_areas(&app, area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 0);
        assert_eq!(cards[0].rect.height, body.height);
    }

    #[test]
    fn oversized_agent_override_is_clipped_to_the_panel_body() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("one");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        app.sidebar_agents.rows_by_agent.insert(
            "claude".into(),
            vec![vec![crate::config::AgentSidebarToken::Agent]; 6],
        );
        let panel = Rect::new(0, 0, 20, 5);

        let metrics = agent_panel_scroll_metrics(&app, panel);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 0);
        let entry = agent_panel_entries(&app).pop().unwrap();
        assert_eq!(
            agent_entry_height_in_body(&app, &entry, agent_panel_body_rect(panel, false).height),
            agent_panel_body_rect(panel, false).height
        );
    }

    #[test]
    fn render_sidebar_toggle_draws_expanded_collapse_icon() {
        let app = crate::app::state::AppState::test_new();
        let area = Rect::new(0, 0, 26, 20);
        let mut terminal =
            Terminal::new(TestBackend::new(26, 20)).expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_toggle(&app, frame, area, false, &app.palette))
            .expect("sidebar toggle should render");

        let toggle = expanded_sidebar_toggle_rect(area);
        assert_eq!(
            terminal.backend().buffer()[(toggle.x, toggle.y)].symbol(),
            "«"
        );
    }

    #[test]
    fn expanded_sidebar_toggle_sits_inside_sidebar_content() {
        let area = Rect::new(0, 0, 26, 20);
        let toggle = expanded_sidebar_toggle_rect(area);

        assert_eq!(toggle.x, area.x + area.width - 2);
        assert_eq!(toggle.y, area.y + area.height - 1);
    }

    #[test]
    fn agent_panel_tab_label_visibility_tracks_tab_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let single_auto = Workspace::test_new("auto");
        let mut single_custom = Workspace::test_new("custom");
        single_custom.tabs[0].set_custom_name("focus".into());
        let mut multi = Workspace::test_new("multi");
        multi.test_add_tab(Some("logs"));

        app.workspaces = vec![single_auto, single_custom, multi];
        app.ensure_test_terminals();
        for (ws_idx, tab_idx, agent) in [
            (0, 0, Agent::Pi),
            (1, 0, Agent::Claude),
            (2, 0, Agent::Codex),
            (2, 1, Agent::Pi),
        ] {
            let pane_id = app.workspaces[ws_idx].tabs[tab_idx].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(agent);
        }

        let entries = agent_panel_entries(&app);
        let labels: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    entry.primary_label.as_str(),
                    entry.primary_tab_label.as_deref(),
                )
            })
            .collect();

        assert_eq!(
            labels,
            [
                ("auto", None),
                ("custom", Some("focus")),
                ("multi", Some("1")),
                ("multi", Some("logs")),
            ]
        );
    }

    #[test]
    fn priority_agent_panel_sort_uses_attention_then_space_order() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
            Workspace::test_new("four"),
        ];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;

        let set_state = |app: &mut crate::app::state::AppState, ws_idx: usize, state| {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = state;
        };
        set_state(&mut app, 0, AgentState::Working);
        set_state(&mut app, 1, AgentState::Idle);
        set_state(&mut app, 2, AgentState::Working);
        set_state(&mut app, 3, AgentState::Blocked);

        let done_pane = app.workspaces[1].tabs[0].root_pane;
        app.workspaces[1].tabs[0]
            .panes
            .get_mut(&done_pane)
            .unwrap()
            .seen = false;

        let labels: Vec<String> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.primary_label)
            .collect();

        assert_eq!(labels, ["four", "two", "one", "three"]);
    }

    #[test]
    fn collapsed_sidebar_draws_a_dot_box_per_tab() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.sidebar_spaces.row_gap = 1;

        let pane = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let term = app.terminals.get_mut(&terminal_id).unwrap();
        term.detected_agent = Some(Agent::Claude);
        term.state = AgentState::Working;

        let area = Rect::new(0, 0, 7, 20);
        let boxes = collapsed_tab_boxes(&app, area);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].rect, Rect::new(0, 5, 6, 3));
        assert_eq!(boxes[1].rect.y, 9);

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");
        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(3, 2)].symbol(), "◧");
        assert_eq!(buffer[(0, 5)].symbol(), "▌");
        // the active tab's filled box wears the half-block frame
        assert_eq!(buffer[(1, 5)].symbol(), "▛");
        assert_eq!(buffer[(5, 7)].symbol(), "▟");
        assert_eq!(buffer[(3, 6)].symbol(), "●");
    }

    #[test]
    fn collapsed_sidebar_active_tab_box_gets_backdrop() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.active = Some(1);
        app.selected = 0;
        app.mode = Mode::Terminal;

        let area = Rect::new(0, 0, 7, 20);
        let boxes = collapsed_tab_boxes(&app, area);
        let active_box = boxes.iter().find(|b| b.ws_idx == 1).unwrap().rect;
        let idle_box = boxes.iter().find(|b| b.ws_idx == 0).unwrap().rect;

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");
        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        let active = buffer[(active_box.x + 2, active_box.y + 1)].style();
        assert_eq!(active.bg, Some(app.palette.surface_dim));
        let idle = buffer[(idle_box.x + 2, idle_box.y + 1)].style();
        assert_ne!(idle.bg, Some(app.palette.surface_dim));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_workspaces_agent_panel_entries_use_live_root_runtime_cwd_for_workspace_label() {
        let unique = format!(
            "herdr-agent-panel-runtime-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let stale_cwd = root.join("issue-264-nix-support");
        let live_cwd = root.join("herdr");
        std::fs::create_dir_all(stale_cwd.join(".git")).unwrap();
        std::fs::create_dir_all(live_cwd.join(".git")).unwrap();

        let mut app = crate::app::state::AppState::test_new();
        let mut workspace = Workspace::test_new("stale-name");
        workspace.custom_name = None;
        workspace.identity_cwd = stale_cwd.clone();
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.cwd = stale_cwd;
        terminal.detected_agent = Some(Agent::Pi);
        app.active = Some(0);
        app.selected = 0;

        let (events, _) = tokio::sync::mpsc::channel(4);
        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane,
            24,
            80,
            live_cwd.clone(),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            crate::pane::PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::NonLogin),
            &crate::pane::PaneLaunchEnv::default(),
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd() != Some(live_cwd.clone()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut runtime_registry = TerminalRuntimeRegistry::new();
        runtime_registry.insert(terminal_id, runtime);
        let entries = agent_panel_entries_from(&app, &runtime_registry);
        let primary_label = entries[0].primary_label.clone();

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(primary_label, "herdr");
    }

    #[test]
    fn all_workspaces_agent_panel_entries_prefer_agent_names_for_agent_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let first_pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .set_agent_name("planner".into());
        app.active = Some(0);
        app.selected = 0;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "bridge");
        assert_eq!(entries[0].agent_label.as_deref(), Some("planner"));
    }

    #[test]
    fn expanded_sidebar_sections_handle_tiny_heights() {
        let (ws_area, detail_area) = expanded_sidebar_sections(Rect::new(0, 0, 20, 5), 0.9);

        assert_eq!(ws_area, Rect::new(0, 0, 19, 3));
        assert_eq!(detail_area, Rect::new(0, 3, 19, 2));
    }

    #[test]
    fn sidebar_section_divider_is_hidden_for_tiny_heights() {
        let divider = sidebar_section_divider_rect(Rect::new(0, 0, 20, 5), 0.5);

        assert_eq!(divider, Rect::default());
    }

    #[test]
    fn grouped_child_label_keeps_custom_workspace_name() {
        assert_eq!(
            grouped_child_display_label("renamed issue", Some("worktree/issue-137"), true),
            "renamed issue"
        );
    }

    #[test]
    fn grouped_child_label_uses_short_branch_for_auto_named_workspace() {
        assert_eq!(
            grouped_child_display_label("herdr-issue", Some("worktree/issue-137"), false),
            "issue-137"
        );
    }

    #[test]
    fn workspace_list_truncates_cjk_branch_without_panic() {
        let mut app = crate::app::state::AppState::test_new();
        let mut ws = Workspace::test_new("repo");
        ws.cached_git_branch = Some("feature/中文-分支-644".into());
        app.workspaces = vec![ws];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.view.workspace_card_areas = vec![crate::app::state::WorkspaceCardArea {
            ws_idx: 0,
            rect: Rect::new(0, 1, 15, 2),
            indented: false,
        }];

        let mut terminal = Terminal::new(TestBackend::new(15, 6)).expect("test terminal");
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        terminal
            .draw(|frame| {
                render_workspace_list(&app, &runtimes, frame, Rect::new(0, 0, 15, 6), false)
            })
            .expect("workspace list should render");
    }

    fn workspace_with_worktree_space(
        name: &str,
        key: Option<&str>,
        checkout_key: &str,
    ) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        if let Some(key) = key {
            ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
                key: key.into(),
                label: "herdr".into(),
                repo_root: std::path::PathBuf::from("/repo/herdr"),
                checkout_path: std::path::PathBuf::from(checkout_key),
                is_linked_worktree: name != "main",
            });
        }
        ws
    }

    fn workspace_with_git_space(name: &str, key: &str) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        ws.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
            key: key.into(),
            checkout_key: format!("/repo/{name}"),
            label: "herdr".into(),
            repo_root: std::path::PathBuf::from(format!("/repo/{name}")),
            is_linked_worktree: false,
        });
        ws
    }

    #[test]
    fn parent_workspace_row_stays_clickable_when_grouped() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.sidebar_spaces.row_gap = 1;

        let (cards, _) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 20));

        assert_eq!(cards[0].ws_idx, 0);
        assert!(!cards[0].indented);
        assert_eq!(cards[1].ws_idx, 1);
        assert!(cards[1].indented);
        assert_eq!(cards[1].rect.y, cards[0].rect.y + cards[0].rect.height + 1);
    }

    #[test]
    fn space_row_gap_preserves_compact_worktree_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
            Workspace::test_new("notes"),
        ];
        app.sidebar_spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
        app.sidebar_spaces.row_gap = 2;

        let (spacious, _) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 30));
        assert_eq!(
            spacious[1].rect.y,
            spacious[0].rect.y + spacious[0].rect.height + 2
        );
        assert_eq!(
            spacious[2].rect.y,
            spacious[1].rect.y + spacious[1].rect.height
        );
        assert_eq!(
            spacious[3].rect.y,
            spacious[2].rect.y + spacious[2].rect.height + 2
        );
        let spacious_metrics = workspace_list_scroll_metrics(&app, Rect::new(0, 0, 30, 16));
        assert_eq!(spacious_metrics.viewport_rows, 2);
        assert_eq!(spacious_metrics.max_offset_from_bottom, 2);

        app.sidebar_spaces.row_gap = 0;
        let (packed, _) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 30));
        assert!(packed
            .windows(2)
            .all(|pair| pair[1].rect.y == pair[0].rect.y + pair[0].rect.height));
        let packed_metrics = workspace_list_scroll_metrics(&app, Rect::new(0, 0, 30, 16));
        assert_eq!(packed_metrics.viewport_rows, 2);
        assert_eq!(packed_metrics.max_offset_from_bottom, 1);
    }

    #[test]
    fn bottom_space_that_part_fits_still_fills_remaining_rows() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.sidebar_spaces.row_gap = 1;

        let (cards, tabs) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 11));

        assert_eq!(cards.len(), 2);
        assert_eq!(cards[1].rect.height, 2);
        assert_eq!(tabs.last().unwrap().rect.height, 2);
    }

    #[test]
    fn packed_workspace_drag_indicator_overlays_an_internal_boundary() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        app.sidebar_spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
        app.sidebar_spaces.row_gap = 0;
        let area = Rect::new(0, 0, 30, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);
        let indicator_row =
            workspace_drop_indicator_row(&app.view.workspace_card_areas, list_area, 2).unwrap();
        assert_eq!(indicator_row, app.view.workspace_card_areas[2].rect.y - 1);
        app.drag = Some(crate::app::state::DragState {
            target: crate::app::state::DragTarget::WorkspaceReorder {
                source_ws_idx: 0,
                insert_idx: Some(2),
            },
        });

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();

        assert_eq!(
            terminal.backend().buffer()[(list_area.x, indicator_row)].symbol(),
            "─"
        );
    }

    #[test]
    fn linked_only_worktree_members_do_not_form_parentless_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
        ];

        let entries = workspace_list_entries(&app);

        assert_eq!(
            entries,
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false
                },
            ]
        );
    }

    #[test]
    fn compact_space_group_scroll_clamps_when_all_entries_fit() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("one", Some("repo-key"), "/repo/herdr-one"),
            workspace_with_worktree_space("two", Some("repo-key"), "/repo/herdr-two"),
        ];
        let area = Rect::new(0, 0, 30, 20);
        app.workspace_scroll = normalized_workspace_scroll(&app, area, 2);

        let (cards, _) = compute_workspace_list_areas(&app, area);

        assert_eq!(app.workspace_scroll, 0);
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[2].ws_idx, 2);
    }

    #[test]
    fn workspace_scroll_metrics_count_display_entries_not_raw_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        for workspace in &mut app.workspaces {
            workspace.cached_git_branch = Some("main".into());
        }
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;

        let ws_area = Rect::new(0, 0, 30, 6);
        let metrics = workspace_list_scroll_metrics(&app, ws_area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(metrics.offset_from_bottom, 1);
    }

    #[test]
    fn workspace_scroll_offset_applies_to_group_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;
        app.workspace_scroll = 1;

        let (cards, tab_rows) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 12));

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
        assert!(tab_rows.iter().all(|row| row.ws_idx == 2));
    }

    #[test]
    fn workspace_list_entries_group_multiple_workspaces_in_same_git_space() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_group_non_contiguous_explicit_members() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("normal", "other-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_group_normal_git_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_git_space("two", "repo-key"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_auto_attach_normal_git_workspace_to_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("scratch", "repo-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_leave_single_git_and_non_git_workspaces_flat() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_worktree_space("notes", None, "/notes"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn collapsed_group_hides_inactive_children_but_keeps_active_visible() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.active = Some(1);
        app.mode = Mode::Terminal;
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );

        app.active = None;
        app.mode = Mode::Terminal;
        assert_eq!(
            workspace_list_entries(&app),
            vec![WorkspaceListEntry::Workspace {
                ws_idx: 0,
                indented: false,
            }]
        );
    }

    #[test]
    fn collapsed_group_keeps_selected_child_visible_in_navigate_mode() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.mode = Mode::Navigate;
        app.selected = 1;
        app.active = Some(1);
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }
}

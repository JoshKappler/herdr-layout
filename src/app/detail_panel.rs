use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::state::AppState;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) const CYCLE_DWELL: Duration = Duration::from_secs(6);

/// Focused tab's task timeline for the right-side panel, read from the
/// summarizer's per-session state file.
pub struct DetailPanelCache {
    pub session_key: String,
    pub agent: String,
    pub timeline: Option<Timeline>,
    checked_at: Instant,
    timeline_sig: (u64, u64),
}

#[derive(serde::Deserialize, Clone, Default)]
pub struct Timeline {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub total_secs: f64,
    #[serde(default)]
    pub out_tokens: u64,
    #[serde(default)]
    pub done: Vec<TimelineItem>,
    #[serde(default)]
    pub current: Vec<TimelineItem>,
    #[serde(default)]
    pub next: Vec<String>,
    #[serde(default)]
    pub subs: Vec<SubLane>,
    #[serde(default, rename = "_exch")]
    pub exch: Vec<Exchange>,
}

/// Per-exchange detail the daemon writes alongside the timeline.
#[derive(serde::Deserialize, Clone)]
pub struct Exchange {
    pub u: String,
    #[serde(default)]
    pub ts: f64,
    #[serde(default)]
    pub last_ts: f64,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub rhead: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub ylabel: String,
}

/// One exchange as the prompt viewer shows it.
pub struct ExchangeView {
    pub u: String,
    pub label: String,
    pub head: String,
    pub rhead: String,
    pub ts: f64,
    pub secs: Option<f64>,
    pub done: bool,
}

impl Timeline {
    /// Every exchange in wall-clock order, done items then current.
    pub(crate) fn exchange_views(&self) -> Vec<ExchangeView> {
        let view = |item: &TimelineItem, done: bool| {
            let exch = self.exch.iter().find(|e| e.u == item.u);
            let first = |a: &str, b: &str| {
                if a.is_empty() { b } else { a }.to_string()
            };
            // the label mirrors the task row's; the head prefers the
            // longer `_exch` text over the item's clipped copy
            ExchangeView {
                u: item.u.clone(),
                label: first(
                    &item.label,
                    exch.map(|e| first(&e.ylabel, &e.label))
                        .unwrap_or_default()
                        .as_str(),
                ),
                head: first(exch.map(|e| e.head.as_str()).unwrap_or_default(), &item.head),
                rhead: exch.map(|e| e.rhead.clone()).unwrap_or_default(),
                ts: exch.map(|e| e.ts).filter(|ts| *ts > 0.0).unwrap_or(item.ts),
                secs: done
                    .then(|| {
                        item.secs.or_else(|| {
                            exch.and_then(|e| {
                                (e.ts > 0.0 && e.last_ts > e.ts).then(|| e.last_ts - e.ts)
                            })
                        })
                    })
                    .flatten(),
                done,
            }
        };
        self.done
            .iter()
            .map(|item| view(item, true))
            .chain(self.current.iter().map(|item| view(item, false)))
            .collect()
    }
}

#[derive(serde::Deserialize, Clone)]
pub struct TimelineItem {
    pub u: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub ts: f64,
    #[serde(default)]
    pub secs: Option<f64>,
    #[serde(default)]
    pub off: u64,
}

#[derive(serde::Deserialize, Clone)]
pub struct SubLane {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub started: f64,
    #[serde(default)]
    pub ended: f64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub events: Vec<String>,
}

#[cfg(test)]
impl DetailPanelCache {
    pub(crate) fn test_with_timeline(timeline: Timeline) -> Self {
        Self {
            session_key: "test".into(),
            agent: "claude".into(),
            timeline: Some(timeline),
            checked_at: Instant::now(),
            timeline_sig: (0, 0),
        }
    }
}

fn timeline_path(session_id: &str) -> Option<PathBuf> {
    if session_id.contains('/') || session_id.contains("..") {
        return None;
    }
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".local/state/herdr-detail")
            .join(format!("{session_id}.json")),
    )
}

impl AppState {
    /// Advances the prompt cycler once per dwell; `now` is injected so
    /// tests can drive the clock.
    pub(crate) fn tick_detail_cycler(&mut self, now: Instant) {
        if self.detail_panel_pinned.is_some() {
            self.detail_panel_cycle_at = Some(now);
            return;
        }
        match self.detail_panel_cycle_at {
            Some(at) if now.duration_since(at) >= CYCLE_DWELL => {
                self.detail_panel_cycle = self.detail_panel_cycle.wrapping_add(1);
                self.detail_panel_cycle_at = Some(now);
            }
            Some(_) => {}
            None => self.detail_panel_cycle_at = Some(now),
        }
    }

    fn drop_unresolved_pin(&mut self) {
        if let Some(u) = self.detail_panel_pinned.as_deref() {
            let resolves = self
                .detail_panel
                .as_ref()
                .and_then(|cache| cache.timeline.as_ref())
                .is_some_and(|tl| tl.done.iter().chain(tl.current.iter()).any(|i| i.u == u));
            if !resolves {
                self.detail_panel_pinned = None;
            }
        }
    }

    pub(crate) fn refresh_detail_panel(&mut self) {
        let session = self
            .active
            .and_then(|i| self.workspaces.get(i))
            .and_then(|ws| {
                let pane_id = ws.focused_pane_id()?;
                let terminal_id = ws.terminal_id(pane_id)?;
                self.terminals.get(terminal_id)
            })
            .and_then(|terminal| {
                let session = terminal.persisted_agent_session.as_ref()?;
                Some((session.agent.clone(), session.session_ref.value.clone()))
            });
        let Some((agent, value)) = session else {
            self.detail_panel = None;
            self.detail_panel_pinned = None;
            return;
        };

        if let Some(cache) = &self.detail_panel {
            if cache.session_key == value && cache.checked_at.elapsed() < REFRESH_INTERVAL {
                return;
            }
        }

        let tl_path = timeline_path(&value);
        let timeline_sig = tl_path.as_deref().map(file_signature).unwrap_or((0, 0));
        if let Some(cache) = &mut self.detail_panel {
            if cache.session_key == value && cache.timeline_sig == timeline_sig {
                cache.checked_at = Instant::now();
                return;
            }
        }

        let timeline = tl_path.as_deref().and_then(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            serde_json::from_str::<Timeline>(&text).ok()
        });
        self.detail_panel = Some(DetailPanelCache {
            session_key: value,
            agent,
            timeline,
            checked_at: Instant::now(),
            timeline_sig,
        });
        // a pin the new timeline cannot resolve would freeze the viewer
        self.drop_unresolved_pin();
    }
}

fn file_signature(path: &Path) -> (u64, u64) {
    std::fs::metadata(path)
        .map(|meta| {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (meta.len(), mtime)
        })
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycler_advances_per_dwell_and_holds_while_pinned() {
        let mut app = crate::app::state::AppState::test_new();
        let t0 = Instant::now();
        app.tick_detail_cycler(t0);
        app.tick_detail_cycler(t0 + CYCLE_DWELL - Duration::from_secs(1));
        assert_eq!(app.detail_panel_cycle, 0);
        app.tick_detail_cycler(t0 + CYCLE_DWELL);
        assert_eq!(app.detail_panel_cycle, 1);

        app.detail_panel_pinned = Some("u9".into());
        app.tick_detail_cycler(t0 + CYCLE_DWELL * 10);
        assert_eq!(app.detail_panel_cycle, 1);

        // unpinning restarts the dwell from the pinned stretch's last tick
        app.detail_panel_pinned = None;
        app.tick_detail_cycler(t0 + CYCLE_DWELL * 10 + Duration::from_secs(1));
        assert_eq!(app.detail_panel_cycle, 1);
        app.tick_detail_cycler(t0 + CYCLE_DWELL * 11);
        assert_eq!(app.detail_panel_cycle, 2);
    }
}

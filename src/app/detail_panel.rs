use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::state::AppState;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

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
    pub started: f64,
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

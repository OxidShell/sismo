use serde::Serialize;
use sismo_format::Level;

use crate::groups::Group;
use crate::query::Query;
use crate::session::Session;
use crate::stats::{FrameSummary, FunctionStats, GroupSplit, SortBy};

#[derive(Debug, Clone)]
pub struct SummaryOptions {
    pub query: Query,
    pub window: Option<(u64, u64)>,
    pub top_functions: usize,
    pub slow_frames: usize,
    pub problems: usize,
}

impl Default for SummaryOptions {
    fn default() -> Self {
        Self {
            query: Query::default(),
            window: None,
            top_functions: 15,
            slow_frames: 10,
            problems: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SlowFrame {
    pub index: u32,
    pub start_ns: u64,
    pub duration_ns: u64,
    pub split: GroupSplit,
    /// Heaviest spans by self time inside the frame: `(name, self_ns)`.
    pub top: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct LogCounts {
    pub error: usize,
    pub warn: usize,
    pub info: usize,
    pub debug: usize,
    pub trace: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub t_ns: u64,
    pub level: &'static str,
    pub target: String,
    pub source: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadSummary {
    pub name: String,
    pub spans: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkerLine {
    pub t_ns: u64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PanicLine {
    pub t_ns: u64,
    pub thread: String,
    pub message: String,
    pub location: String,
}

/// Compact, stable overview of a recording for agents and reports.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub app: String,
    pub app_version: String,
    pub build: String,
    pub platform: String,
    pub device: String,
    pub session_id: String,
    pub started_unix_ns: u64,
    pub window_ns: (u64, u64),
    pub duration_ns: u64,
    pub events: u64,
    pub dropped_events: u64,
    pub threads: Vec<ThreadSummary>,
    pub frames: FrameSummary,
    pub split: GroupSplit,
    pub split_percent: [(Group, f64); 3],
    pub top_functions: Vec<FunctionStats>,
    pub slow_frames: Vec<SlowFrame>,
    pub logs: LogCounts,
    /// Warnings and errors, oldest first.
    pub problems: Vec<LogLine>,
    pub markers: Vec<MarkerLine>,
    pub panics: Vec<PanicLine>,
}

impl Session {
    #[must_use]
    pub fn summary(&self, options: &SummaryOptions) -> Summary {
        let window = options.window.unwrap_or_else(|| options.query.window(self));
        let (from, to) = window;
        let query = &options.query;
        let mut top = self.functions(query, window, 1, SortBy::SelfTime);
        top.truncate(options.top_functions);
        for row in &mut top {
            row.history_p95_ns.clear();
        }
        let split = self.group_split(query, None, window);
        let slow_frames = self
            .slowest_frames(window, options.slow_frames)
            .into_iter()
            .map(|frame| {
                let mut spans: Vec<(String, u64)> = self
                    .thread(frame.thread)
                    .map(|t| {
                        t.spans_in(frame.start, frame.end)
                            .map(|s| (self.name(s.callsite).to_owned(), s.self_ns))
                            .collect()
                    })
                    .unwrap_or_default();
                spans.sort_unstable_by_key(|s| std::cmp::Reverse(s.1));
                spans.truncate(5);
                SlowFrame {
                    index: frame.index,
                    start_ns: frame.start,
                    duration_ns: frame.duration(),
                    split: self.frame_split(&frame),
                    top: spans,
                }
            })
            .collect();
        let logs_in = self
            .logs
            .iter()
            .filter(|l| l.t >= from && l.t < to && query.matches_log(self, l));
        let mut counts = LogCounts::default();
        let mut problems = Vec::new();
        for log in logs_in {
            match log.level {
                Level::Error => counts.error += 1,
                Level::Warn => counts.warn += 1,
                Level::Info => counts.info += 1,
                Level::Debug => counts.debug += 1,
                Level::Trace => counts.trace += 1,
            }
            if log.level <= Level::Warn && problems.len() < options.problems {
                problems.push(LogLine {
                    t_ns: log.t,
                    level: log.level.as_str(),
                    target: self
                        .callsite(log.callsite)
                        .map(|c| c.target.clone())
                        .unwrap_or_default(),
                    source: log.source.as_str(),
                    message: log.message.clone(),
                });
            }
        }
        Summary {
            app: self.info.app.clone(),
            app_version: self.info.app_version.clone(),
            build: self.info.build.clone(),
            platform: self.info.platform.clone(),
            device: self.info.device.clone(),
            session_id: format!("{:016x}", self.info.session_id),
            started_unix_ns: self.info.started_unix_ns,
            window_ns: window,
            duration_ns: to.saturating_sub(from),
            events: self.events,
            dropped_events: self.dropped,
            threads: self
                .threads
                .iter()
                .map(|t| ThreadSummary {
                    name: t.name.clone(),
                    spans: t.spans.len(),
                })
                .collect(),
            frames: self.frame_summary(window),
            split,
            split_percent: Group::ALL.map(|g| (g, (split.percent(g) * 10.0).round() / 10.0)),
            top_functions: top,
            slow_frames,
            logs: counts,
            problems,
            markers: self
                .markers
                .iter()
                .filter(|m| m.t >= from && m.t < to)
                .map(|m| MarkerLine {
                    t_ns: m.t,
                    note: m.note.clone(),
                })
                .collect(),
            panics: self
                .panics
                .iter()
                .map(|p| PanicLine {
                    t_ns: p.t,
                    thread: self.thread_name(p.thread).to_owned(),
                    message: p.message.clone(),
                    location: p.location.clone(),
                })
                .collect(),
        }
    }
}

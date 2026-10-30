use std::collections::HashMap;

use serde::Serialize;

use crate::groups::Group;
use crate::query::{Query, SLOW_FRAME_NS};
use crate::session::{Frame, Session};

/// Aggregate timing of one callsite over a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionStats {
    pub callsite: u32,
    pub name: String,
    #[serde(rename = "crate")]
    pub krate: String,
    pub group: Group,
    pub file: String,
    pub line: u32,
    pub calls: u64,
    pub total_ns: u64,
    pub self_ns: u64,
    pub mean_ns: u64,
    pub min_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    /// p95 per equal slice of the window, oldest first (0 = no calls).
    pub history_p95_ns: Vec<u64>,
}

/// Value at quantile `q` of an already sorted slice.
#[must_use]
pub fn percentile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// How to order function rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    SelfTime,
    Total,
    Calls,
    P95,
    Max,
    Name,
}

impl SortBy {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "self" => Some(SortBy::SelfTime),
            "total" => Some(SortBy::Total),
            "calls" => Some(SortBy::Calls),
            "p95" => Some(SortBy::P95),
            "max" => Some(SortBy::Max),
            "name" => Some(SortBy::Name),
            _ => None,
        }
    }
}

/// Node of an aggregated call tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FlameNode {
    pub callsite: u32,
    pub total_ns: u64,
    pub self_ns: u64,
    pub calls: u64,
    pub children: Vec<FlameNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct GroupSplit {
    pub app_ns: u64,
    pub framework_ns: u64,
    pub other_ns: u64,
}

impl GroupSplit {
    fn add(&mut self, group: Group, ns: u64) {
        match group {
            Group::App => self.app_ns += ns,
            Group::Framework => self.framework_ns += ns,
            Group::Other => self.other_ns += ns,
        }
    }

    #[must_use]
    pub fn total(&self) -> u64 {
        self.app_ns + self.framework_ns + self.other_ns
    }

    #[must_use]
    pub fn get(&self, group: Group) -> u64 {
        match group {
            Group::App => self.app_ns,
            Group::Framework => self.framework_ns,
            Group::Other => self.other_ns,
        }
    }

    /// Share of `group` in percent.
    #[must_use]
    pub fn percent(&self, group: Group) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let share = self.get(group) as f64 / total as f64 * 100.0;
        share
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FrameSummary {
    pub count: usize,
    pub mean_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    /// Frames above the 60 Hz budget.
    pub slow: usize,
}

fn overlap(start: u64, end: u64, from: u64, to: u64) -> u64 {
    end.min(to).saturating_sub(start.max(from))
}

/// Self time scaled to the part of a span inside the window.
fn clipped_self(self_ns: u64, start: u64, end: u64, from: u64, to: u64) -> u64 {
    let duration = end.saturating_sub(start);
    if duration == 0 {
        return 0;
    }
    let inside = overlap(start, end, from, to);
    if inside >= duration {
        return self_ns;
    }
    u64::try_from(u128::from(self_ns) * u128::from(inside) / u128::from(duration))
        .unwrap_or(self_ns)
}

impl Session {
    /// Per-callsite statistics of spans in `[from, to)` that pass `query`.
    #[must_use]
    pub fn functions(
        &self,
        query: &Query,
        (from, to): (u64, u64),
        buckets: usize,
        sort: SortBy,
    ) -> Vec<FunctionStats> {
        struct Acc {
            durations: Vec<u64>,
            self_ns: u64,
            per_bucket: Vec<Vec<u64>>,
        }
        let buckets = buckets.max(1);
        let width = (to.saturating_sub(from) / buckets as u64).max(1);
        let mut acc: HashMap<u32, Acc> = HashMap::new();
        for thread in &self.threads {
            for span in thread.spans_in(from, to) {
                if span.is_open() || !query.matches_span(self, &thread.name, span) {
                    continue;
                }
                let entry = acc.entry(span.callsite).or_insert_with(|| Acc {
                    durations: Vec::new(),
                    self_ns: 0,
                    per_bucket: vec![Vec::new(); buckets],
                });
                let duration = span.end - span.start;
                entry.durations.push(duration);
                entry.self_ns += span.self_ns;
                let bucket = usize::try_from(span.start.saturating_sub(from) / width)
                    .unwrap_or(usize::MAX)
                    .min(buckets - 1);
                entry.per_bucket[bucket].push(duration);
            }
        }
        let mut rows: Vec<FunctionStats> = acc
            .into_iter()
            .map(|(callsite, mut a)| {
                a.durations.sort_unstable();
                let total: u64 = a.durations.iter().sum();
                let calls = a.durations.len() as u64;
                let def = self.callsite(callsite);
                FunctionStats {
                    callsite,
                    name: self.name(callsite).to_owned(),
                    krate: self.crate_of(callsite).to_owned(),
                    group: self.group_of(callsite),
                    file: def.map(|d| d.file.clone()).unwrap_or_default(),
                    line: def.map_or(0, |d| d.line),
                    calls,
                    total_ns: total,
                    self_ns: a.self_ns,
                    mean_ns: total / calls.max(1),
                    min_ns: a.durations.first().copied().unwrap_or(0),
                    p50_ns: percentile(&a.durations, 0.5),
                    p95_ns: percentile(&a.durations, 0.95),
                    p99_ns: percentile(&a.durations, 0.99),
                    max_ns: a.durations.last().copied().unwrap_or(0),
                    history_p95_ns: a
                        .per_bucket
                        .into_iter()
                        .map(|mut b| {
                            b.sort_unstable();
                            percentile(&b, 0.95)
                        })
                        .collect(),
                }
            })
            .filter(|row| query.matches_stats(row.calls, row.self_ns, row.total_ns))
            .collect();
        rows.sort_by(|a, b| match sort {
            SortBy::SelfTime => b.self_ns.cmp(&a.self_ns),
            SortBy::Total => b.total_ns.cmp(&a.total_ns),
            SortBy::Calls => b.calls.cmp(&a.calls),
            SortBy::P95 => b.p95_ns.cmp(&a.p95_ns),
            SortBy::Max => b.max_ns.cmp(&a.max_ns),
            SortBy::Name => a.name.cmp(&b.name),
        });
        rows
    }

    /// Durations of every completed call of `callsite` in the window, with
    /// start time and thread, slowest first.
    #[must_use]
    pub fn calls_of(&self, callsite: u32, (from, to): (u64, u64)) -> Vec<(u64, u64, u32)> {
        let mut calls: Vec<(u64, u64, u32)> = self
            .threads
            .iter()
            .flat_map(|t| {
                t.spans_in(from, to)
                    .filter(move |s| s.callsite == callsite && !s.is_open())
                    .map(move |s| (s.end - s.start, s.start, t.id))
            })
            .collect();
        calls.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));
        calls
    }

    /// Aggregated call tree of `thread` (or every thread) in the window.
    #[must_use]
    pub fn flame(&self, query: &Query, thread: Option<u32>, (from, to): (u64, u64)) -> FlameNode {
        let mut root = FlameNode {
            callsite: 0,
            total_ns: 0,
            self_ns: 0,
            calls: 0,
            children: Vec::new(),
        };
        for t in self
            .threads
            .iter()
            .filter(|t| thread.is_none_or(|id| t.id == id))
        {
            // Path of child indices from the root for every included span.
            let mut paths: HashMap<u32, Vec<usize>> = HashMap::new();
            for index in t.indices_in(from, to) {
                let span = &t.spans[index as usize];
                let end = span.end.min(self.end);
                let mut path = span
                    .parent
                    .and_then(|p| paths.get(&p))
                    .cloned()
                    .unwrap_or_default();
                if !query.matches_span(self, &t.name, span) {
                    // Filtered spans vanish; their children attach to the
                    // nearest kept ancestor.
                    paths.insert(index, path);
                    continue;
                }
                let mut node = &mut root;
                for &step in &path {
                    node = &mut node.children[step];
                }
                let position = node
                    .children
                    .iter()
                    .position(|c| c.callsite == span.callsite)
                    .unwrap_or_else(|| {
                        node.children.push(FlameNode {
                            callsite: span.callsite,
                            total_ns: 0,
                            self_ns: 0,
                            calls: 0,
                            children: Vec::new(),
                        });
                        node.children.len() - 1
                    });
                let child = &mut node.children[position];
                child.total_ns += overlap(span.start, end, from, to);
                child.self_ns += clipped_self(span.self_ns, span.start, end, from, to);
                child.calls += 1;
                path.push(position);
                paths.insert(index, path);
            }
        }
        root.total_ns = root.children.iter().map(|c| c.total_ns).sum();
        sort_flame(&mut root);
        root
    }

    /// Self time per group in the window, optionally on one thread.
    #[must_use]
    pub fn group_split(
        &self,
        query: &Query,
        thread: Option<u32>,
        (from, to): (u64, u64),
    ) -> GroupSplit {
        let mut split = GroupSplit::default();
        for t in self
            .threads
            .iter()
            .filter(|t| thread.is_none_or(|id| t.id == id))
        {
            for span in t.spans_in(from, to) {
                if !query.matches_span(self, &t.name, span) {
                    continue;
                }
                let end = span.end.min(self.end);
                split.add(
                    self.group_of(span.callsite),
                    clipped_self(span.self_ns, span.start, end, from, to),
                );
            }
        }
        split
    }

    /// Group split of one frame on the frame thread.
    #[must_use]
    pub fn frame_split(&self, frame: &Frame) -> GroupSplit {
        self.group_split(
            &Query::default(),
            Some(frame.thread),
            (frame.start, frame.end),
        )
    }

    /// Frames overlapping the window.
    #[must_use]
    pub fn frames_in(&self, (from, to): (u64, u64)) -> &[Frame] {
        let first = self.frames.partition_point(|f| f.end <= from);
        let last = self.frames.partition_point(|f| f.start < to);
        &self.frames[first..last.max(first)]
    }

    #[must_use]
    pub fn frame_summary(&self, window: (u64, u64)) -> FrameSummary {
        let frames = self.frames_in(window);
        let mut durations: Vec<u64> = frames.iter().map(Frame::duration).collect();
        durations.sort_unstable();
        let total: u64 = durations.iter().sum();
        FrameSummary {
            count: durations.len(),
            mean_ns: total / (durations.len() as u64).max(1),
            p50_ns: percentile(&durations, 0.5),
            p95_ns: percentile(&durations, 0.95),
            p99_ns: percentile(&durations, 0.99),
            max_ns: durations.last().copied().unwrap_or(0),
            slow: durations.iter().filter(|d| **d > SLOW_FRAME_NS).count(),
        }
    }

    /// Slowest frames in the window, slowest first.
    #[must_use]
    pub fn slowest_frames(&self, window: (u64, u64), limit: usize) -> Vec<Frame> {
        let mut frames = self.frames_in(window).to_vec();
        frames.sort_unstable_by_key(|f| std::cmp::Reverse(f.duration()));
        frames.truncate(limit);
        frames
    }
}

fn sort_flame(node: &mut FlameNode) {
    node.children
        .sort_unstable_by_key(|c| std::cmp::Reverse(c.total_ns));
    node.children.iter_mut().for_each(sort_flame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sismo_format::{CallsiteDef, CallsiteKind, Event, Level, Record};

    fn session() -> Session {
        let def = |id, name: &str| {
            Record::Callsite(CallsiteDef {
                id,
                kind: CallsiteKind::Span,
                name: name.into(),
                target: name.into(),
                module_path: name.into(),
                file: "x.rs".into(),
                line: 1,
                level: Level::Trace,
            })
        };
        let mut events = vec![Event::Frame { t: 0 }];
        for i in 0..10u64 {
            let base = i * 20_000_000;
            events.extend([
                Event::Begin {
                    t: base,
                    callsite: 1,
                },
                Event::Begin {
                    t: base + 1_000_000,
                    callsite: 2,
                },
                Event::End {
                    t: base + 1_000_000 + (i + 1) * 1_000_000,
                },
                Event::End {
                    t: base + 15_000_000,
                },
                Event::Frame {
                    t: base + 20_000_000,
                },
            ]);
        }
        let mut s = Session::default();
        s.ingest(vec![
            def(1, "demo::frame"),
            def(2, "freya_core::render"),
            Record::Batch { thread: 1, events },
        ]);
        s
    }

    #[test]
    fn function_stats_flame_and_split() {
        let s = session();
        let window = (0, s.end + 1);
        let q = Query::default();
        let rows = s.functions(&q, window, 4, SortBy::Total);
        assert_eq!(rows[0].name, "demo::frame");
        assert_eq!(rows[1].calls, 10);
        assert_eq!(rows[1].max_ns, 10_000_000);
        let flame = s.flame(&q, None, window);
        assert_eq!(flame.children.len(), 1);
        assert_eq!(flame.children[0].children[0].calls, 10);
        let split = s.group_split(&q, None, window);
        assert_eq!(split.framework_ns, (1..=10).sum::<u64>() * 1_000_000);
        assert_eq!(s.frame_summary(window).count, 10);
        let fw = s.functions(
            &Query::parse("crate:framework").unwrap(),
            window,
            1,
            SortBy::SelfTime,
        );
        assert_eq!(fw.len(), 1);
        assert_eq!(s.calls_of(2, window)[0].0, 10_000_000);
    }
}

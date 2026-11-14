use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use sismo_format::{
    CallsiteDef, CallsiteKind, Chunk, DecodeError, Event, FileReader, Level, ReadOutcome, Record,
    SessionInfo, Source,
};

use crate::groups::{Group, Groups};

/// Span end used while the span is still open.
pub const OPEN: u64 = u64::MAX;

/// One completed (or still open) span on a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
    pub callsite: u32,
    pub depth: u16,
    /// Index of the parent span in the same thread, if any.
    pub parent: Option<u32>,
    /// Time not covered by child spans.
    pub self_ns: u64,
    /// Bytes allocated while the span ran, children included.
    pub alloc_bytes: u64,
    /// Allocations made while the span ran, children included.
    pub alloc_count: u64,
    /// Bytes allocated by the span itself, children excluded.
    pub alloc_self_bytes: u64,
}

impl Span {
    #[must_use]
    pub fn duration(&self, now: u64) -> u64 {
        self.end.min(now).saturating_sub(self.start)
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.end == OPEN
    }
}

/// A thread's stack at one instant: depth and innermost open span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StackMark {
    depth: usize,
    top: Option<u32>,
}

/// An async span still running, with the stack of the thread it began on.
#[derive(Debug, Clone, Copy)]
struct PendingAsync {
    span: AsyncSpan,
    mark: StackMark,
}

#[derive(Debug, Clone, Copy)]
struct Open {
    index: u32,
    children_ns: u64,
    children_alloc: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Thread {
    pub id: u32,
    pub name: String,
    /// Spans in begin order.
    pub spans: Vec<Span>,
    pub max_depth: u16,
    stack: Vec<Open>,
    pending_alloc: Option<(u64, u64)>,
}

impl Thread {
    fn begin(&mut self, t: u64, callsite: u32) {
        let depth = u16::try_from(self.stack.len()).unwrap_or(u16::MAX);
        self.max_depth = self.max_depth.max(depth);
        let index = u32::try_from(self.spans.len()).unwrap_or(u32::MAX);
        self.spans.push(Span {
            start: t,
            end: OPEN,
            callsite,
            depth,
            parent: self.stack.last().map(|o| o.index),
            self_ns: 0,
            alloc_bytes: 0,
            alloc_count: 0,
            alloc_self_bytes: 0,
        });
        self.stack.push(Open {
            index,
            children_ns: 0,
            children_alloc: 0,
        });
    }

    /// Closes the innermost span; ends without a begin (lost to a ring
    /// buffer cut) are ignored.
    fn end(&mut self, t: u64) {
        let Some(open) = self.stack.pop() else {
            return;
        };
        let Some(span) = self.spans.get_mut(open.index as usize) else {
            return;
        };
        span.end = t.max(span.start);
        let duration = span.end - span.start;
        span.self_ns = duration.saturating_sub(open.children_ns);
        let (bytes, count) = self.pending_alloc.take().unwrap_or((0, 0));
        span.alloc_bytes = bytes;
        span.alloc_count = count;
        span.alloc_self_bytes = bytes.saturating_sub(open.children_alloc);
        if let Some(parent) = self.stack.last_mut() {
            parent.children_ns += duration;
            parent.children_alloc += bytes;
        }
    }

    /// Depth and innermost open span, to tell later whether a span that
    /// begins now nested properly.
    fn stack_mark(&self) -> StackMark {
        StackMark {
            depth: self.stack.len(),
            top: self.stack.last().map(|o| o.index),
        }
    }

    /// Inserts a span only known once it ends, an async scope that turned out
    /// to run nested on this thread: the stack must be the one `mark` saw at
    /// `start`. The spans that began since become its children. Returns false
    /// when it did not nest, so the caller keeps it as an async span.
    fn insert_nested(&mut self, start: u64, end: u64, callsite: u32, mark: StackMark) -> bool {
        if self.stack_mark() != mark {
            return false;
        }
        let depth = u16::try_from(mark.depth).unwrap_or(u16::MAX);
        // After every open span, even one that began in the same nanosecond.
        let first = self
            .spans
            .partition_point(|s| s.start < start)
            .max(mark.top.map_or(0, |top| top as usize + 1));
        let children: Vec<usize> = (first..self.spans.len())
            .filter(|&i| self.spans[i].depth == depth)
            .collect();
        // A sismo `async fn` done in its first poll: that poll span already
        // shows it.
        if let [only] = children[..]
            && self.spans[only].callsite == callsite
        {
            return true;
        }
        let children_ns: u64 = children.iter().map(|&i| self.spans[i].duration(end)).sum();
        let children_alloc: u64 = children.iter().map(|&i| self.spans[i].alloc_bytes).sum();
        let index = u32::try_from(first).unwrap_or(u32::MAX);
        for span in &mut self.spans[first..] {
            if span.depth == depth {
                span.parent = Some(index);
            } else if let Some(parent) = span.parent.as_mut()
                && *parent >= index
            {
                *parent += 1;
            }
            span.depth = span.depth.saturating_add(1);
            self.max_depth = self.max_depth.max(span.depth);
        }
        let duration = end.saturating_sub(start);
        self.spans.insert(
            first,
            Span {
                start,
                end,
                callsite,
                depth,
                parent: mark.top,
                self_ns: duration.saturating_sub(children_ns),
                alloc_bytes: children_alloc,
                alloc_count: 0,
                alloc_self_bytes: 0,
            },
        );
        self.max_depth = self.max_depth.max(depth);
        if let Some(parent) = self.stack.last_mut() {
            parent.children_ns = parent.children_ns.saturating_sub(children_ns) + duration;
        }
        true
    }

    /// Index of the first span starting at or after `t`.
    #[must_use]
    pub fn first_at(&self, t: u64) -> usize {
        self.spans.partition_point(|s| s.start < t)
    }

    /// Spans overlapping `[from, to)`, in begin order.
    pub fn spans_in(&self, from: u64, to: u64) -> impl Iterator<Item = &Span> {
        let first = self.first_at(from);
        self.open_at(first, from)
            .into_iter()
            .map(|i| &self.spans[i as usize])
            .chain(self.spans[first..].iter().take_while(move |s| s.start < to))
    }

    /// Indices of the spans overlapping `[from, to)`, in begin order.
    #[must_use]
    pub fn indices_in(&self, from: u64, to: u64) -> Vec<u32> {
        let first = self.first_at(from);
        let mut indices = self.open_at(first, from);
        let tail = self.spans[first..]
            .iter()
            .take_while(|s| s.start < to)
            .count();
        indices.extend((first..first + tail).filter_map(|i| u32::try_from(i).ok()));
        indices
    }

    /// Spans that began before `from` (index `first`) and are still open
    /// at it, outermost first. On one thread spans nest, so these are the
    /// last span that began before `from` and its ancestors.
    fn open_at(&self, first: usize, from: u64) -> Vec<u32> {
        let mut indices = Vec::new();
        let mut cursor = first.checked_sub(1).and_then(|i| u32::try_from(i).ok());
        while let Some(index) = cursor {
            let span = &self.spans[index as usize];
            if span.end > from {
                indices.push(index);
            }
            cursor = span.parent;
        }
        indices.reverse();
        indices
    }
}

/// A span recorded across `.await` points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncSpan {
    pub id: u64,
    pub callsite: u32,
    pub thread: u32,
    pub start: u64,
    pub end: u64,
}

impl AsyncSpan {
    #[must_use]
    pub fn duration(&self, now: u64) -> u64 {
        self.end.min(now).saturating_sub(self.start)
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.end == OPEN
    }

    /// The same interval as a top-level thread span, for the filters and
    /// exports that take one. Wall time is not time spent on a thread, so it
    /// carries no self time.
    #[must_use]
    pub fn as_span(&self) -> Span {
        Span {
            start: self.start,
            end: self.end,
            callsite: self.callsite,
            depth: 0,
            parent: None,
            self_ns: 0,
            alloc_bytes: 0,
            alloc_count: 0,
            alloc_self_bytes: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub t: u64,
    pub thread: u32,
    pub callsite: u32,
    pub level: Level,
    pub source: Source,
    pub message: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub index: u32,
    pub thread: u32,
    pub start: u64,
    pub end: u64,
}

impl Frame {
    #[must_use]
    pub fn duration(&self) -> u64 {
        self.end - self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub t: u64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panic {
    pub t: u64,
    pub thread: u32,
    pub message: String,
    pub location: String,
    pub backtrace: String,
}

/// Summary of loading a recording.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub chunks: usize,
    pub bytes: u64,
    /// Offset and reason of the first unreadable chunk; the rest is ignored.
    pub damaged: Option<(u64, DecodeError)>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not open {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("not a sismo recording: {0}")]
    Format(#[from] DecodeError),
}

/// Everything known about one recorded process, built incrementally from
/// records (a file or a live stream).
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub info: SessionInfo,
    callsites: Vec<Option<CallsiteDef>>,
    pub threads: Vec<Thread>,
    thread_index: HashMap<u32, usize>,
    pub async_spans: Vec<AsyncSpan>,
    /// Async spans still running; they join `async_spans` when they end,
    /// unless they turn out to have run nested on one thread.
    open_async: HashMap<u64, PendingAsync>,
    pub logs: Vec<LogEntry>,
    pub counters: HashMap<u32, Vec<(u64, f64)>>,
    pub frames: Vec<Frame>,
    frame_marks: HashMap<u32, Vec<u64>>,
    frame_thread: Option<u32>,
    pub markers: Vec<Marker>,
    pub panics: Vec<Panic>,
    pub dropped: u64,
    /// Latest timestamp seen.
    pub end: u64,
    /// Earliest timestamp seen.
    pub start: Option<u64>,
    /// Read freely; change it with [`Session::set_groups`] so the per-callsite
    /// cache follows.
    pub groups: Groups,
    /// Group of every callsite, by id: classifying a crate allocates and
    /// walks every rule, far too slow to repeat per span.
    group_cache: Vec<Group>,
    pub events: u64,
    /// Whether the recording carries allocation counts.
    pub allocations: bool,
    /// Records after a session record of another run are skipped: that run's
    /// ids and timestamps mean other things.
    foreign: bool,
}

impl Session {
    /// Loads a `.sismo` file, keeping whatever precedes a damaged tail.
    ///
    /// # Errors
    /// [`LoadError`] when the file cannot be opened or is not a recording.
    pub fn load(path: &Path) -> Result<(Self, LoadReport), LoadError> {
        let file = File::open(path).map_err(|source| LoadError::Open {
            path: path.display().to_string(),
            source,
        })?;
        let mut session = Session::default();
        let mut report = LoadReport::default();
        let mut reader = FileReader::new(BufReader::new(file))?;
        for outcome in reader.by_ref() {
            match outcome {
                ReadOutcome::Chunk(records) => {
                    report.chunks += 1;
                    session.ingest(records);
                }
                ReadOutcome::Damaged { offset, error } => report.damaged = Some((offset, error)),
            }
        }
        report.bytes = reader.offset();
        Ok((session, report))
    }

    /// Decodes and ingests one chunk received from the network.
    ///
    /// # Errors
    /// [`DecodeError`] for malformed bytes.
    pub fn ingest_chunk(&mut self, bytes: Vec<u8>) -> Result<(), DecodeError> {
        let chunk = Chunk::from_bytes(bytes)?;
        self.ingest(chunk.records()?);
        Ok(())
    }

    pub fn ingest(&mut self, records: Vec<Record>) {
        for record in records {
            match record {
                Record::Session(info) => {
                    self.foreign =
                        self.info.session_id != 0 && self.info.session_id != info.session_id;
                    if !self.foreign {
                        self.groups.set_app(&info.app);
                        self.groups.add_app_crates(&info.app_crates);
                        self.refresh_groups();
                        self.info = info;
                    }
                }
                _ if self.foreign => {}
                Record::Thread(def) => {
                    let index = self.thread_slot(def.id);
                    self.threads[index].name = def.name;
                }
                Record::Callsite(def) => {
                    let id = def.id as usize;
                    if self.callsites.len() <= id {
                        self.callsites.resize(id + 1, None);
                    }
                    self.callsites[id] = Some(def);
                    self.refresh_group(id);
                }
                Record::Batch { thread, events } => {
                    for event in events {
                        self.apply(thread, event);
                    }
                }
            }
        }
    }

    fn thread_slot(&mut self, id: u32) -> usize {
        if let Some(index) = self.thread_index.get(&id) {
            return *index;
        }
        let index = self.threads.len();
        self.threads.push(Thread {
            id,
            name: format!("thread-{id}"),
            ..Thread::default()
        });
        self.thread_index.insert(id, index);
        index
    }

    fn apply(&mut self, thread: u32, event: Event) {
        let t = event.time();
        self.end = self.end.max(t);
        self.start = Some(self.start.map_or(t, |s| s.min(t)));
        self.events += 1;
        let slot = self.thread_slot(thread);
        match event {
            Event::Begin { t, callsite } => self.threads[slot].begin(t, callsite),
            Event::End { t } => self.threads[slot].end(t),
            Event::AsyncBegin { t, callsite, id } => {
                let mark = self.threads[slot].stack_mark();
                let span = AsyncSpan {
                    id,
                    callsite,
                    thread,
                    start: t,
                    end: OPEN,
                };
                self.open_async.insert(id, PendingAsync { span, mark });
            }
            Event::AsyncEnd { t, id } => {
                if let Some(PendingAsync { mut span, mark }) = self.open_async.remove(&id) {
                    // A scope that may suspend (hotpath's `measure_block!`) but ended
                    // on its thread with the stack untouched ran nested: it belongs
                    // in that thread's tree, not in the async lane.
                    let nested = span.thread == thread
                        && self.threads[slot].insert_nested(span.start, t, span.callsite, mark);
                    if !nested {
                        span.end = t;
                        self.async_spans.push(span);
                    }
                }
            }
            Event::Instant { t, callsite } => {
                let message = self.name(callsite).to_owned();
                self.logs.push(LogEntry {
                    t,
                    thread,
                    callsite,
                    level: Level::Trace,
                    source: Source::Sismo,
                    message,
                    fields: Vec::new(),
                });
            }
            Event::Counter { t, callsite, value } => {
                self.counters.entry(callsite).or_default().push((t, value));
            }
            Event::Frame { t } => self.frame_mark(thread, t),
            Event::Log {
                t,
                callsite,
                level,
                source,
                message,
                fields,
            } => self.logs.push(LogEntry {
                t,
                thread,
                callsite,
                level,
                source,
                message,
                fields,
            }),
            Event::Marker { t, note } => self.markers.push(Marker { t, note }),
            Event::Dropped { count, .. } => self.dropped += count,
            Event::Panic {
                t,
                message,
                location,
                backtrace,
            } => {
                // Also a log line, so it reads in context with what led to it.
                self.logs.push(LogEntry {
                    t,
                    thread,
                    callsite: 0,
                    level: Level::Panic,
                    source: Source::Sismo,
                    message: message.clone(),
                    fields: vec![("location".to_owned(), location.clone())],
                });
                self.panics.push(Panic {
                    t,
                    thread,
                    message,
                    location,
                    backtrace,
                });
            }
            Event::Alloc { bytes, count, .. } => {
                self.threads[slot].pending_alloc = Some((bytes, count));
                self.allocations = true;
            }
        }
    }

    fn frame_mark(&mut self, thread: u32, t: u64) {
        let marks = self.frame_marks.entry(thread).or_default();
        marks.push(t);
        let count = marks.len();
        let current = self
            .frame_thread
            .and_then(|ft| self.frame_marks.get(&ft))
            .map_or(0, Vec::len);
        let spans = self
            .thread_index
            .get(&thread)
            .map_or(&[][..], |i| self.threads[*i].spans.as_slice());
        if self.frame_thread != Some(thread) && count > current {
            // The thread with most marks drives frames; rebuild when it changes.
            self.frame_thread = Some(thread);
            self.frames = self.frame_marks[&thread]
                .windows(2)
                .enumerate()
                .map(|(i, w)| Frame {
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                    thread,
                    start: frame_start(spans, w[0], w[1]),
                    end: w[1],
                })
                .collect();
        } else if self.frame_thread == Some(thread) && count >= 2 {
            let previous = self.frame_marks[&thread][count - 2];
            self.frames.push(Frame {
                index: u32::try_from(self.frames.len()).unwrap_or(u32::MAX),
                thread,
                start: frame_start(spans, previous, t),
                end: t,
            });
        }
    }

    #[must_use]
    pub fn callsite(&self, id: u32) -> Option<&CallsiteDef> {
        self.callsites.get(id as usize).and_then(Option::as_ref)
    }

    pub fn callsites(&self) -> impl Iterator<Item = &CallsiteDef> {
        self.callsites.iter().flatten()
    }

    /// Display name of a callsite, or `?` when its definition is missing.
    #[must_use]
    pub fn name(&self, id: u32) -> &str {
        self.callsite(id).map_or("?", |c| c.name.as_str())
    }

    #[must_use]
    pub fn crate_of(&self, id: u32) -> &str {
        self.callsite(id).map_or("?", CallsiteDef::crate_name)
    }

    fn refresh_group(&mut self, id: usize) {
        if self.group_cache.len() <= id {
            self.group_cache.resize(id + 1, Group::Other);
        }
        let krate = self.callsites[id]
            .as_ref()
            .map_or("?", CallsiteDef::crate_name);
        self.group_cache[id] = self.groups.classify(krate);
    }

    fn refresh_groups(&mut self) {
        for id in 0..self.callsites.len() {
            self.refresh_group(id);
        }
    }

    #[must_use]
    pub fn group_of(&self, id: u32) -> Group {
        self.group_cache
            .get(id as usize)
            .copied()
            .unwrap_or_else(|| self.groups.classify(self.crate_of(id)))
    }

    #[must_use]
    pub fn thread(&self, id: u32) -> Option<&Thread> {
        self.thread_index.get(&id).map(|i| &self.threads[*i])
    }

    /// Async spans overlapping `[from, to)`, in begin order.
    pub fn async_spans_in(&self, from: u64, to: u64) -> impl Iterator<Item = &AsyncSpan> {
        self.async_spans
            .iter()
            .filter(move |s| s.start < to && s.end > from)
    }

    #[must_use]
    pub fn thread_name(&self, id: u32) -> &str {
        self.thread(id).map_or("?", |t| t.name.as_str())
    }

    /// The panic a [`Level::Panic`] log line stands for.
    #[must_use]
    pub fn panic_of(&self, log: &LogEntry) -> Option<&Panic> {
        if log.level != Level::Panic {
            return None;
        }
        self.panics
            .iter()
            .find(|p| p.t == log.t && p.thread == log.thread)
    }

    /// The thread whose frame marks define [`Session::frames`].
    #[must_use]
    pub fn frame_thread(&self) -> Option<u32> {
        self.frame_thread
    }

    /// Distinct crate names that own spans or events.
    #[must_use]
    pub fn crates(&self) -> Vec<String> {
        let mut crates: Vec<String> = self
            .callsites()
            .filter(|c| c.kind != CallsiteKind::Counter)
            .map(|c| c.crate_name().to_owned())
            .collect();
        crates.sort();
        crates.dedup();
        crates
    }

    /// Converts a session timestamp to Unix nanoseconds.
    #[must_use]
    pub fn unix_ns(&self, t: u64) -> u64 {
        self.info.started_unix_ns.saturating_add(t)
    }

    /// Replaces the crate groups, keeping the crates the app declared as
    /// its own.
    pub fn set_groups(&mut self, mut groups: Groups) {
        groups.add_app_crates(&self.info.app_crates);
        self.groups = groups;
        self.refresh_groups();
    }

    /// The app, thread and callsite records known so far, enough for a
    /// reader to decode events that follow.
    #[must_use]
    pub fn definitions(&self) -> Vec<Record> {
        std::iter::once(Record::Session(self.info.clone()))
            .chain(self.threads.iter().map(|t| {
                Record::Thread(sismo_format::ThreadDef {
                    id: t.id,
                    name: t.name.clone(),
                })
            }))
            .chain(self.callsites().cloned().map(Record::Callsite))
            .collect()
    }

    /// Forgets every recorded event but keeps the app, thread and callsite
    /// definitions, so a live stream keeps decoding from here on. Spans
    /// still open are dropped too; their ends are ignored when they come.
    pub fn clear_events(&mut self) {
        for thread in &mut self.threads {
            thread.spans.clear();
            thread.stack.clear();
            thread.max_depth = 0;
            thread.pending_alloc = None;
        }
        self.async_spans.clear();
        self.open_async.clear();
        self.logs.clear();
        self.counters.clear();
        self.frames.clear();
        self.frame_marks.clear();
        self.frame_thread = None;
        self.markers.clear();
        self.panics.clear();
        self.dropped = 0;
        self.end = 0;
        self.start = None;
        self.events = 0;
    }

    /// First and last timestamp of the recording.
    #[must_use]
    pub fn span(&self) -> (u64, u64) {
        (self.start.unwrap_or(0), self.end)
    }
}

/// A pause longer than this between top-level spans means the thread was
/// idle, not still producing the frame.
const IDLE_GAP_NS: u64 = 4_000_000;

/// Where the frame ending at `end` began: walking back from the mark over
/// top-level spans while they follow each other closely, so an app idling
/// between redraws does not show the wait as one long frame. Threads
/// without spans keep mark-to-mark.
fn frame_start(spans: &[Span], previous: u64, end: u64) -> u64 {
    if spans.is_empty() {
        return previous;
    }
    let first = spans.partition_point(|s| s.start < previous);
    let last = spans.partition_point(|s| s.start < end);
    let mut start = end;
    let mut busy = 0;
    // Spans are in begin order on one thread; a recording that repeats data
    // can break that, so a slice that does not make sense means no frame.
    let Some(window) = spans.get(first..last) else {
        return end;
    };
    for span in window.iter().rev().filter(|s| s.depth == 0) {
        let span_end = if span.is_open() { end } else { span.end };
        if span_end.saturating_add(IDLE_GAP_NS) < start {
            break;
        }
        // A trickle of short spans (pointer events handled between redraws)
        // has small gaps but is mostly idle: stop once taking the next span
        // would leave the frame busy less than half its time.
        let with = busy + span_end.saturating_sub(span.start);
        let extent = end.saturating_sub(start.min(span.start));
        if busy > 0 && with * 2 < extent {
            break;
        }
        busy = with;
        start = start.min(span.start);
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    use sismo_format::{CallsiteKind, SessionInfo, ThreadDef};

    fn def(id: u32, name: &str) -> Record {
        Record::Callsite(CallsiteDef {
            id,
            kind: CallsiteKind::Span,
            name: name.into(),
            target: name.into(),
            module_path: name.into(),
            file: String::new(),
            line: 0,
            level: Level::Trace,
        })
    }

    #[test]
    fn builds_nested_spans_and_frames() {
        let mut session = Session::default();
        session.ingest(vec![
            Record::Thread(ThreadDef {
                id: 1,
                name: "main".into(),
            }),
            def(1, "app::frame"),
            def(2, "freya_core::render"),
            Record::Batch {
                thread: 1,
                events: vec![
                    Event::Frame { t: 0 },
                    Event::Begin { t: 10, callsite: 1 },
                    Event::Begin { t: 20, callsite: 2 },
                    Event::End { t: 50 },
                    Event::End { t: 100 },
                    Event::Frame { t: 110 },
                    Event::End { t: 120 },
                ],
            },
        ]);
        let main = session.thread(1).unwrap();
        assert_eq!(main.spans.len(), 2);
        assert_eq!(main.spans[0].self_ns, 60);
        assert_eq!(main.spans[1].depth, 1);
        assert_eq!(main.spans[1].parent, Some(0));
        assert_eq!(session.frames.len(), 1);
        // The frame starts at its first span, not at the previous mark.
        assert_eq!(session.frames[0].duration(), 100);
        assert_eq!(main.spans_in(30, 40).count(), 2);
        assert_eq!(main.spans_in(60, 70).count(), 1);
    }

    #[test]
    fn idle_time_between_redraws_is_not_frame_time() {
        let mut session = Session::default();
        session.ingest(vec![
            def(1, "app::frame"),
            Record::Batch {
                thread: 1,
                events: vec![
                    Event::Frame { t: 0 },
                    Event::Begin {
                        t: 60_000,
                        callsite: 1,
                    },
                    Event::End { t: 64_000 },
                    Event::Frame { t: 65_000 },
                ],
            },
        ]);
        assert_eq!(session.frames[0].start, 60_000);
        assert_eq!(session.frames[0].duration(), 5_000);
    }

    #[test]
    fn nested_async_scope_joins_the_thread_tree() {
        let mut session = Session::default();
        session.ingest(vec![
            def(1, "core::render"),
            def(2, "Element Render"),
            def(3, "core::from_element"),
            Record::Batch {
                thread: 1,
                events: vec![
                    Event::Begin { t: 0, callsite: 1 },
                    Event::AsyncBegin {
                        t: 10,
                        callsite: 2,
                        id: 1,
                    },
                    Event::Begin { t: 20, callsite: 3 },
                    Event::End { t: 30 },
                    Event::AsyncEnd { t: 40, id: 1 },
                    Event::End { t: 100 },
                ],
            },
        ]);
        assert_eq!(session.async_spans, [] as [AsyncSpan; 0]);
        let spans = &session.threads[0].spans;
        let shape: Vec<_> = spans
            .iter()
            .map(|s| (s.callsite, s.depth, s.parent, s.self_ns))
            .collect();
        assert_eq!(
            shape,
            vec![(1, 0, None, 70), (2, 1, Some(0), 20), (3, 2, Some(1), 10)]
        );
        assert_eq!(session.threads[0].max_depth, 2);
    }

    #[test]
    fn suspended_async_scope_stays_async() {
        let mut session = Session::default();
        session.ingest(vec![
            def(1, "freya::tasks"),
            def(2, "app::fetch"),
            Record::Batch {
                thread: 1,
                events: vec![
                    Event::Begin { t: 0, callsite: 1 },
                    Event::AsyncBegin {
                        t: 10,
                        callsite: 2,
                        id: 1,
                    },
                    Event::End { t: 20 },
                    Event::Begin { t: 50, callsite: 1 },
                    Event::AsyncEnd { t: 60, id: 1 },
                    Event::End { t: 70 },
                ],
            },
        ]);
        assert_eq!(session.async_spans.len(), 1);
        assert_eq!(session.async_spans[0].duration(u64::MAX), 50);
        assert!(session.threads[0].spans.iter().all(|s| s.callsite == 1));
    }

    #[test]
    fn frame_skips_a_trickle_of_events_before_the_redraw() {
        let mut session = Session::default();
        let mut events = vec![Event::Frame { t: 0 }];
        // Pointer events every 2 ms for 100 ms, 10 us each, no redraw.
        for i in 0..50u64 {
            let t = 1_000_000 + i * 2_000_000;
            events.push(Event::Begin { t, callsite: 1 });
            events.push(Event::End { t: t + 10_000 });
        }
        // Then the redraw: 3 ms of work right before the mark.
        events.extend([
            Event::Begin {
                t: 101_000_000,
                callsite: 2,
            },
            Event::End { t: 104_000_000 },
            Event::Frame { t: 104_100_000 },
        ]);
        session.ingest(vec![
            def(1, "freya::events"),
            def(2, "freya::frame"),
            Record::Batch { thread: 1, events },
        ]);
        // The event right before the redraw stays, it likely triggered it; the
        // 98 ms of idle trickle before it does not.
        assert_eq!(session.frames[0].start, 99_000_000);
    }

    #[test]
    fn records_of_another_run_are_skipped() {
        let info = |id| {
            Record::Session(SessionInfo {
                session_id: id,
                app: "app".into(),
                ..SessionInfo::default()
            })
        };
        let mut session = Session::default();
        let work = |t| Record::Batch {
            thread: 1,
            events: vec![Event::Begin { t, callsite: 1 }, Event::End { t: t + 1 }],
        };
        session.ingest(vec![info(7), def(1, "app::work"), work(10)]);
        session.ingest(vec![info(8), def(1, "other::thing"), work(2)]);
        session.ingest(vec![info(7), work(20)]);
        assert_eq!(session.info.session_id, 7);
        assert_eq!(session.name(1), "app::work");
        assert_eq!(session.thread(1).unwrap().spans.len(), 2);
    }

    #[test]
    fn clearing_keeps_definitions_for_later_events() {
        let mut session = Session::default();
        session.ingest(vec![
            def(1, "app::work"),
            Record::Batch {
                thread: 1,
                events: vec![Event::Begin { t: 1, callsite: 1 }, Event::End { t: 5 }],
            },
        ]);
        session.clear_events();
        assert_eq!(session.thread(1).unwrap().spans, [] as [Span; 0]);
        assert_eq!(session.span(), (0, 0));
        session.ingest(vec![Record::Batch {
            thread: 1,
            events: vec![Event::Begin { t: 9, callsite: 1 }, Event::End { t: 12 }],
        }]);
        assert_eq!(session.thread(1).unwrap().spans.len(), 1);
        assert_eq!(session.name(1), "app::work");
        assert_eq!(session.span(), (9, 12));
    }
}

use std::io::{self, Write};

use serde_json::json;

use super::ExportOptions;
use crate::session::Session;

#[allow(clippy::cast_precision_loss)]
fn micros(ns: u64) -> f64 {
    ns as f64 / 1000.0
}

/// Chrome trace event JSON: complete events for spans, instants for logs,
/// counters as `C` events.
pub(super) fn write(
    session: &Session,
    options: &ExportOptions,
    (from, to): (u64, u64),
    out: &mut dyn Write,
) -> io::Result<()> {
    let pid = session.info.pid;
    let mut events = Vec::new();
    events.push(
        json!({"ph": "M", "name": "process_name", "pid": pid, "args": {"name": session.info.app}}),
    );
    for thread in &session.threads {
        events.push(json!({"ph": "M", "name": "thread_name", "pid": pid, "tid": thread.id, "args": {"name": thread.name}}));
        for span in thread.spans_in(from, to) {
            if !options.query.matches_span(session, &thread.name, span) {
                continue;
            }
            events.push(json!({
                "ph": "X",
                "name": session.name(span.callsite),
                "cat": session.crate_of(span.callsite),
                "ts": micros(span.start),
                "dur": micros(span.duration(session.end)),
                "pid": pid,
                "tid": thread.id,
            }));
        }
    }
    if options.include_logs {
        for log in session
            .logs
            .iter()
            .filter(|l| l.t >= from && l.t < to && options.query.matches_log(session, l))
        {
            events.push(json!({
                "ph": "i", "s": "t", "name": log.message, "cat": log.level.as_str(),
                "ts": micros(log.t), "pid": pid, "tid": log.thread,
            }));
        }
    }
    for (callsite, samples) in &session.counters {
        for (t, value) in samples.iter().filter(|(t, _)| *t >= from && *t < to) {
            events.push(json!({
                "ph": "C", "name": session.name(*callsite), "ts": micros(*t), "pid": pid,
                "args": {"value": value},
            }));
        }
    }
    serde_json::to_writer(
        &mut *out,
        &json!({"traceEvents": events, "displayTimeUnit": "ms"}),
    )
    .map_err(io::Error::other)
}

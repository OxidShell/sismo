//! Minimal protobuf writer for the Perfetto `Trace` format: one track per
//! thread, slices for spans, instants for logs and markers, counter tracks.

use std::io::{self, Write};

use super::ExportOptions;
use crate::session::Session;

const SEQUENCE: u64 = 1;
const PROCESS_TRACK: u64 = 1;
const THREAD_TRACK_BASE: u64 = 1_000;
const COUNTER_TRACK_BASE: u64 = 1_000_000;

const SLICE_BEGIN: u64 = 1;
const SLICE_END: u64 = 2;
const INSTANT: u64 = 3;
const COUNTER: u64 = 4;

#[derive(Default)]
struct Proto(Vec<u8>);

impl Proto {
    fn varint(&mut self, mut value: u64) {
        while value >= 0x80 {
            #[allow(clippy::cast_possible_truncation)]
            self.0.push((value as u8) | 0x80);
            value >>= 7;
        }
        #[allow(clippy::cast_possible_truncation)]
        self.0.push(value as u8);
    }

    fn tag(&mut self, field: u64, wire: u64) {
        self.varint((field << 3) | wire);
    }

    fn uint(&mut self, field: u64, value: u64) -> &mut Self {
        self.tag(field, 0);
        self.varint(value);
        self
    }

    fn double(&mut self, field: u64, value: f64) -> &mut Self {
        self.tag(field, 1);
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn bytes(&mut self, field: u64, value: &[u8]) -> &mut Self {
        self.tag(field, 2);
        self.varint(value.len() as u64);
        self.0.extend_from_slice(value);
        self
    }

    fn string(&mut self, field: u64, value: &str) -> &mut Self {
        self.bytes(field, value.as_bytes())
    }

    fn message(&mut self, field: u64, inner: &Proto) -> &mut Self {
        self.bytes(field, &inner.0)
    }
}

struct Writer<'a> {
    out: &'a mut dyn Write,
}

impl Writer<'_> {
    /// Wraps a `TracePacket` as field 1 of `Trace` and writes it.
    fn packet(&mut self, packet: &mut Proto) -> io::Result<()> {
        packet.uint(10, SEQUENCE);
        let mut trace = Proto::default();
        trace.message(1, packet);
        self.out.write_all(&trace.0)
    }

    fn descriptor(&mut self, descriptor: &Proto) -> io::Result<()> {
        let mut packet = Proto::default();
        packet.message(60, descriptor);
        self.packet(&mut packet)
    }

    fn event(&mut self, ts: u64, event: &Proto) -> io::Result<()> {
        let mut packet = Proto::default();
        packet.uint(8, ts).message(11, event);
        self.packet(&mut packet)
    }
}

fn track_event(kind: u64, track: u64, name: Option<&str>) -> Proto {
    let mut event = Proto::default();
    event.uint(9, kind).uint(11, track);
    if let Some(name) = name {
        event.string(23, name);
    }
    event
}

pub(super) fn write(
    session: &Session,
    options: &ExportOptions,
    (from, to): (u64, u64),
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut w = Writer { out };
    let pid = u64::from(session.info.pid);
    let mut process = Proto::default();
    process.uint(1, pid).string(
        6,
        &format!("{} {}", session.info.app, session.info.app_version),
    );
    let mut descriptor = Proto::default();
    descriptor.uint(1, PROCESS_TRACK).message(3, &process);
    w.descriptor(&descriptor)?;

    for thread in &session.threads {
        let track = THREAD_TRACK_BASE + u64::from(thread.id);
        let mut td = Proto::default();
        td.uint(1, pid)
            .uint(2, u64::from(thread.id))
            .string(5, &thread.name);
        let mut descriptor = Proto::default();
        descriptor
            .uint(1, track)
            .uint(5, PROCESS_TRACK)
            .message(4, &td);
        w.descriptor(&descriptor)?;

        // Emit begin/end in time order so nesting is preserved.
        let mut edges: Vec<(u64, u8, u16, u32)> = Vec::new();
        for span in thread.spans_in(from, to) {
            if !options.query.matches_span(session, &thread.name, span) {
                continue;
            }
            let end = span.end.min(session.end);
            edges.push((span.start, 1, span.depth, span.callsite));
            edges.push((end, 0, u16::MAX - span.depth, span.callsite));
        }
        edges.sort_unstable();
        for (t, kind, _, callsite) in edges {
            let ts = session.unix_ns(t);
            if kind == 1 {
                let mut event = track_event(SLICE_BEGIN, track, Some(session.name(callsite)));
                event.string(22, session.crate_of(callsite));
                w.event(ts, &event)?;
            } else {
                w.event(ts, &track_event(SLICE_END, track, None))?;
            }
        }
    }

    if options.include_logs {
        for log in session
            .logs
            .iter()
            .filter(|l| l.t >= from && l.t < to && options.query.matches_log(session, l))
        {
            let track = THREAD_TRACK_BASE + u64::from(log.thread);
            let mut event = track_event(INSTANT, track, Some(&log.message));
            event.string(22, log.level.as_str());
            w.event(session.unix_ns(log.t), &event)?;
        }
    }
    for marker in session.markers.iter().filter(|m| m.t >= from && m.t < to) {
        let event = track_event(INSTANT, PROCESS_TRACK, Some(&marker.note));
        w.event(session.unix_ns(marker.t), &event)?;
    }
    for (callsite, samples) in &session.counters {
        let track = COUNTER_TRACK_BASE + u64::from(*callsite);
        let mut descriptor = Proto::default();
        descriptor
            .uint(1, track)
            .uint(5, PROCESS_TRACK)
            .string(2, session.name(*callsite))
            .message(8, &Proto::default());
        w.descriptor(&descriptor)?;
        for (t, value) in samples.iter().filter(|(t, _)| *t >= from && *t < to) {
            let mut event = track_event(COUNTER, track, None);
            event.double(44, *value);
            w.event(session.unix_ns(*t), &event)?;
        }
    }
    Ok(())
}

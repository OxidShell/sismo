use std::io::{self, Write};

use sismo_format::{BatchWriter, Chunk, Encoder, FileWriter, Record, ThreadDef};

use super::ExportOptions;
use crate::session::Session;

/// Re-encodes the window as a standalone `.sismo` recording.
pub(super) fn write(
    session: &Session,
    options: &ExportOptions,
    (from, to): (u64, u64),
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut defs: Vec<Record> = vec![Record::Session(session.info.clone())];
    defs.extend(session.threads.iter().map(|t| {
        Record::Thread(ThreadDef {
            id: t.id,
            name: t.name.clone(),
        })
    }));
    defs.extend(session.callsites().cloned().map(Record::Callsite));
    let mut file = FileWriter::new(out)?;
    file.write_chunk(&Chunk::from_records(&defs).map_err(io::Error::other)?)?;

    for thread in &session.threads {
        // (time, order, payload) where order keeps nesting stable at equal times.
        let mut edges: Vec<(u64, u32, Option<u32>)> = Vec::new();
        for span in thread.spans_in(from, to) {
            edges.push((
                span.start,
                1_000_000 + u32::from(span.depth),
                Some(span.callsite),
            ));
            edges.push((
                span.end.min(session.end),
                1_000_000 - u32::from(span.depth) - 1,
                None,
            ));
        }
        edges.sort_unstable();
        let mut writer = BatchWriter::default();
        for (t, _, callsite) in edges {
            match callsite {
                Some(callsite) => writer.begin(t, callsite),
                None => writer.end(t),
            }
        }
        for frame in session
            .frames_in((from, to))
            .iter()
            .filter(|f| f.thread == thread.id)
        {
            writer.frame(frame.start);
        }
        if options.include_logs {
            for log in session
                .logs
                .iter()
                .filter(|l| l.thread == thread.id && l.t >= from && l.t < to)
            {
                let fields = if options.strip_fields {
                    &[][..]
                } else {
                    &log.fields[..]
                };
                writer.log(
                    log.t,
                    log.callsite,
                    log.level,
                    log.source,
                    &log.message,
                    fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                );
            }
        }
        if writer.is_empty() {
            continue;
        }
        let mut enc = Encoder::default();
        writer.encode_record(thread.id, &mut enc);
        file.write_chunk(
            &Chunk::seal(enc.as_slice(), sismo_format::Codec::Lz4).map_err(io::Error::other)?,
        )?;
    }
    file.flush()
}

use crate::{DecodeError, Decoder, Encoder};

/// Severity of a log record, ordered from most to least severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// A panic: never written as a log record, viewers derive it from the
    /// panic event so it lists among the logs.
    Panic = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Panic,
        Level::Error,
        Level::Warn,
        Level::Info,
        Level::Debug,
        Level::Trace,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Panic => "panic",
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Level::ALL
            .into_iter()
            .find(|level| level.as_str().eq_ignore_ascii_case(value))
    }
}

impl TryFrom<u8> for Level {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(Level::Panic),
            1 => Ok(Level::Error),
            2 => Ok(Level::Warn),
            3 => Ok(Level::Info),
            4 => Ok(Level::Debug),
            5 => Ok(Level::Trace),
            other => Err(DecodeError::InvalidTag {
                what: "level",
                tag: other,
            }),
        }
    }
}

/// Where a log record entered the recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Sismo = 0,
    Tracing = 1,
    Log = 2,
    Stdout = 3,
    Stderr = 4,
    Puffin = 5,
}

impl Source {
    pub const ALL: [Source; 6] = [
        Source::Sismo,
        Source::Tracing,
        Source::Log,
        Source::Stdout,
        Source::Stderr,
        Source::Puffin,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Source::Sismo => "sismo",
            Source::Tracing => "tracing",
            Source::Log => "log",
            Source::Stdout => "stdout",
            Source::Stderr => "stderr",
            Source::Puffin => "puffin",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Source::ALL
            .into_iter()
            .find(|source| source.as_str().eq_ignore_ascii_case(value))
    }
}

impl TryFrom<u8> for Source {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        Source::ALL
            .into_iter()
            .find(|source| *source as u8 == value)
            .ok_or(DecodeError::InvalidTag {
                what: "source",
                tag: value,
            })
    }
}

/// What a callsite records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallsiteKind {
    Span = 0,
    Event = 1,
    Counter = 2,
}

impl TryFrom<u8> for CallsiteKind {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(CallsiteKind::Span),
            1 => Ok(CallsiteKind::Event),
            2 => Ok(CallsiteKind::Counter),
            other => Err(DecodeError::InvalidTag {
                what: "callsite kind",
                tag: other,
            }),
        }
    }
}

/// Identity of the recorded process, sent first on every stream and file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionInfo {
    pub session_id: u64,
    pub app: String,
    pub app_version: String,
    pub build: String,
    pub platform: String,
    pub device: String,
    pub pid: u32,
    /// Wall-clock time of timestamp zero, in nanoseconds since the Unix epoch.
    pub started_unix_ns: u64,
    pub sismo_version: String,
    /// Crate patterns (`name` or `prefix*`) the app counts as its own.
    pub app_crates: Vec<String>,
}

impl SessionInfo {
    /// Reads the fields after a session tag; `crates` says whether the
    /// app's crate patterns follow (older recordings do not carry them).
    fn decode(dec: &mut Decoder<'_>, crates: bool) -> Result<Self, DecodeError> {
        let mut info = SessionInfo {
            session_id: dec.uvar()?,
            app: dec.string()?,
            app_version: dec.string()?,
            build: dec.string()?,
            platform: dec.string()?,
            device: dec.string()?,
            pid: dec.u32var()?,
            started_unix_ns: dec.uvar()?,
            sismo_version: dec.string()?,
            app_crates: Vec::new(),
        };
        if crates {
            let count = dec.uvar()?;
            for _ in 0..count {
                info.app_crates.push(dec.string()?);
            }
        }
        Ok(info)
    }

    /// File name stem `<app>-<version>-<platform>-<device>-<stamp>`, so
    /// recordings of several apps and devices sort and never collide.
    #[must_use]
    pub fn file_stem(&self, stamp: crate::FileStamp) -> String {
        let clean = |part: &str| -> String {
            part.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        [
            self.app.as_str(),
            self.app_version.as_str(),
            self.platform.as_str(),
            self.device.as_str(),
        ]
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(clean)
        .chain(std::iter::once(stamp.to_string()))
        .collect::<Vec<_>>()
        .join("-")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadDef {
    pub id: u32,
    pub name: String,
}

/// Static description of one instrumentation point, interned by `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallsiteDef {
    pub id: u32,
    pub kind: CallsiteKind,
    pub name: String,
    pub target: String,
    pub module_path: String,
    pub file: String,
    pub line: u32,
    pub level: Level,
}

impl CallsiteDef {
    /// Crate name derived from the module path (or the target when the
    /// module path is unknown), with `-` normalized to `_`.
    #[must_use]
    pub fn crate_name(&self) -> &str {
        let path = if self.module_path.is_empty() {
            &self.target
        } else {
            &self.module_path
        };
        path.split("::").next().unwrap_or(path)
    }
}

/// One timestamped occurrence on a thread. Timestamps are nanoseconds since
/// the session start.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Begin {
        t: u64,
        callsite: u32,
    },
    End {
        t: u64,
    },
    AsyncBegin {
        t: u64,
        callsite: u32,
        id: u64,
    },
    AsyncEnd {
        t: u64,
        id: u64,
    },
    Instant {
        t: u64,
        callsite: u32,
    },
    Counter {
        t: u64,
        callsite: u32,
        value: f64,
    },
    Frame {
        t: u64,
    },
    Log {
        t: u64,
        callsite: u32,
        level: Level,
        source: Source,
        message: String,
        fields: Vec<(String, String)>,
    },
    Marker {
        t: u64,
        note: String,
    },
    Dropped {
        t: u64,
        count: u64,
    },
    Panic {
        t: u64,
        message: String,
        location: String,
        /// Captured backtrace, as text; empty when none could be taken.
        backtrace: String,
    },
    /// Heap allocations made while the innermost open span ran (children
    /// included); emitted right before that span's [`Event::End`].
    Alloc {
        t: u64,
        bytes: u64,
        count: u64,
    },
}

impl Event {
    #[must_use]
    pub fn time(&self) -> u64 {
        match self {
            Event::Begin { t, .. }
            | Event::End { t }
            | Event::AsyncBegin { t, .. }
            | Event::AsyncEnd { t, .. }
            | Event::Instant { t, .. }
            | Event::Counter { t, .. }
            | Event::Frame { t }
            | Event::Log { t, .. }
            | Event::Marker { t, .. }
            | Event::Dropped { t, .. }
            | Event::Panic { t, .. }
            | Event::Alloc { t, .. } => *t,
        }
    }
}

mod tag {
    pub const SESSION: u8 = 1;
    pub const THREAD: u8 = 2;
    pub const CALLSITE: u8 = 3;
    pub const BATCH: u8 = 4;
    /// [`SESSION`] followed by the app's own crate patterns.
    pub const SESSION_CRATES: u8 = 5;

    pub const BEGIN: u8 = 1;
    pub const END: u8 = 2;
    pub const ASYNC_BEGIN: u8 = 3;
    pub const ASYNC_END: u8 = 4;
    pub const INSTANT: u8 = 5;
    pub const COUNTER: u8 = 6;
    pub const FRAME: u8 = 7;
    pub const LOG: u8 = 8;
    pub const MARKER: u8 = 9;
    pub const DROPPED: u8 = 10;
    pub const PANIC: u8 = 11;
    pub const ALLOC: u8 = 12;
}

/// Top-level unit inside a chunk payload.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Session(SessionInfo),
    Thread(ThreadDef),
    Callsite(CallsiteDef),
    Batch { thread: u32, events: Vec<Event> },
}

impl Record {
    pub fn encode(&self, enc: &mut Encoder) {
        match self {
            Record::Session(s) => {
                enc.u8(tag::SESSION_CRATES)
                    .uvar(s.session_id)
                    .str(&s.app)
                    .str(&s.app_version)
                    .str(&s.build)
                    .str(&s.platform)
                    .str(&s.device)
                    .uvar(u64::from(s.pid))
                    .uvar(s.started_unix_ns)
                    .str(&s.sismo_version)
                    .uvar(s.app_crates.len() as u64);
                for pattern in &s.app_crates {
                    enc.str(pattern);
                }
            }
            Record::Thread(t) => {
                enc.u8(tag::THREAD).uvar(u64::from(t.id)).str(&t.name);
            }
            Record::Callsite(c) => {
                enc.u8(tag::CALLSITE)
                    .uvar(u64::from(c.id))
                    .u8(c.kind as u8)
                    .str(&c.name)
                    .str(&c.target)
                    .str(&c.module_path)
                    .str(&c.file)
                    .uvar(u64::from(c.line))
                    .u8(c.level as u8);
            }
            Record::Batch { thread, events } => {
                let mut writer = BatchWriter::default();
                for event in events {
                    writer.push(event);
                }
                writer.encode_record(*thread, enc);
            }
        }
    }

    /// # Errors
    /// Any [`DecodeError`] for malformed input.
    pub fn decode(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        match dec.u8()? {
            tag::SESSION => Ok(Record::Session(SessionInfo::decode(dec, false)?)),
            tag::SESSION_CRATES => Ok(Record::Session(SessionInfo::decode(dec, true)?)),
            tag::THREAD => Ok(Record::Thread(ThreadDef {
                id: dec.u32var()?,
                name: dec.string()?,
            })),
            tag::CALLSITE => Ok(Record::Callsite(CallsiteDef {
                id: dec.u32var()?,
                kind: CallsiteKind::try_from(dec.u8()?)?,
                name: dec.string()?,
                target: dec.string()?,
                module_path: dec.string()?,
                file: dec.string()?,
                line: dec.u32var()?,
                level: Level::try_from(dec.u8()?)?,
            })),
            tag::BATCH => {
                let thread = dec.u32var()?;
                let base = dec.uvar()?;
                let body = dec.bytes()?;
                Ok(Record::Batch {
                    thread,
                    events: decode_events(base, body)?,
                })
            }
            other => Err(DecodeError::InvalidTag {
                what: "record",
                tag: other,
            }),
        }
    }
}

fn decode_events(base: u64, body: &[u8]) -> Result<Vec<Event>, DecodeError> {
    let mut dec = Decoder::new(body);
    let mut events = Vec::new();
    let mut t = base;
    while !dec.is_empty() {
        let kind = dec.u8()?;
        t = t.checked_add(dec.uvar()?).ok_or(DecodeError::OutOfRange)?;
        let event = match kind {
            tag::BEGIN => Event::Begin {
                t,
                callsite: dec.u32var()?,
            },
            tag::END => Event::End { t },
            tag::ASYNC_BEGIN => Event::AsyncBegin {
                t,
                callsite: dec.u32var()?,
                id: dec.uvar()?,
            },
            tag::ASYNC_END => Event::AsyncEnd { t, id: dec.uvar()? },
            tag::INSTANT => Event::Instant {
                t,
                callsite: dec.u32var()?,
            },
            tag::COUNTER => Event::Counter {
                t,
                callsite: dec.u32var()?,
                value: dec.f64()?,
            },
            tag::FRAME => Event::Frame { t },
            tag::LOG => {
                let callsite = dec.u32var()?;
                let level = Level::try_from(dec.u8()?)?;
                let source = Source::try_from(dec.u8()?)?;
                let message = dec.string()?;
                let count = dec.uvar()?;
                let fields = (0..count)
                    .map(|_| Ok((dec.string()?, dec.string()?)))
                    .collect::<Result<Vec<_>, DecodeError>>()?;
                Event::Log {
                    t,
                    callsite,
                    level,
                    source,
                    message,
                    fields,
                }
            }
            tag::MARKER => Event::Marker {
                t,
                note: dec.string()?,
            },
            tag::DROPPED => Event::Dropped {
                t,
                count: dec.uvar()?,
            },
            tag::PANIC => Event::Panic {
                t,
                message: dec.string()?,
                location: dec.string()?,
                backtrace: dec.string()?,
            },
            tag::ALLOC => Event::Alloc {
                t,
                bytes: dec.uvar()?,
                count: dec.uvar()?,
            },
            other => {
                return Err(DecodeError::InvalidTag {
                    what: "event",
                    tag: other,
                });
            }
        };
        events.push(event);
    }
    Ok(events)
}

/// Hot-path event encoder: appends delta-timed events of one thread into a
/// reusable buffer, without building [`Event`] values.
#[derive(Debug, Default, Clone)]
pub struct BatchWriter {
    body: Encoder,
    base: Option<u64>,
    last: u64,
    events: usize,
}

impl BatchWriter {
    fn stamp(&mut self, kind: u8, t: u64) -> &mut Encoder {
        let base = *self.base.get_or_insert(t);
        let prev = self.last.max(base);
        // Clocks may step backwards across threads sharing a writer; clamp to keep deltas unsigned.
        let t = t.max(prev);
        self.last = t;
        self.events += 1;
        self.body.u8(kind).uvar(t - prev);
        &mut self.body
    }

    pub fn begin(&mut self, t: u64, callsite: u32) {
        self.stamp(tag::BEGIN, t).uvar(u64::from(callsite));
    }

    pub fn end(&mut self, t: u64) {
        self.stamp(tag::END, t);
    }

    pub fn async_begin(&mut self, t: u64, callsite: u32, id: u64) {
        self.stamp(tag::ASYNC_BEGIN, t)
            .uvar(u64::from(callsite))
            .uvar(id);
    }

    pub fn async_end(&mut self, t: u64, id: u64) {
        self.stamp(tag::ASYNC_END, t).uvar(id);
    }

    pub fn instant(&mut self, t: u64, callsite: u32) {
        self.stamp(tag::INSTANT, t).uvar(u64::from(callsite));
    }

    pub fn counter(&mut self, t: u64, callsite: u32, value: f64) {
        self.stamp(tag::COUNTER, t)
            .uvar(u64::from(callsite))
            .f64(value);
    }

    pub fn frame(&mut self, t: u64) {
        self.stamp(tag::FRAME, t);
    }

    pub fn log<'a>(
        &mut self,
        t: u64,
        callsite: u32,
        level: Level,
        source: Source,
        message: &str,
        fields: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
    ) {
        let enc = self.stamp(tag::LOG, t);
        enc.uvar(u64::from(callsite))
            .u8(level as u8)
            .u8(source as u8)
            .str(message)
            .uvar(fields.len() as u64);
        for (key, value) in fields {
            enc.str(key).str(value);
        }
    }

    pub fn marker(&mut self, t: u64, note: &str) {
        self.stamp(tag::MARKER, t).str(note);
    }

    pub fn dropped(&mut self, t: u64, count: u64) {
        self.stamp(tag::DROPPED, t).uvar(count);
    }

    pub fn panic(&mut self, t: u64, message: &str, location: &str, backtrace: &str) {
        self.stamp(tag::PANIC, t)
            .str(message)
            .str(location)
            .str(backtrace);
    }

    pub fn alloc(&mut self, t: u64, bytes: u64, count: u64) {
        self.stamp(tag::ALLOC, t).uvar(bytes).uvar(count);
    }

    /// Appends an already decoded event.
    pub fn push(&mut self, event: &Event) {
        match event {
            Event::Begin { t, callsite } => self.begin(*t, *callsite),
            Event::End { t } => self.end(*t),
            Event::AsyncBegin { t, callsite, id } => self.async_begin(*t, *callsite, *id),
            Event::AsyncEnd { t, id } => self.async_end(*t, *id),
            Event::Instant { t, callsite } => self.instant(*t, *callsite),
            Event::Counter { t, callsite, value } => self.counter(*t, *callsite, *value),
            Event::Frame { t } => self.frame(*t),
            Event::Log {
                t,
                callsite,
                level,
                source,
                message,
                fields,
            } => self.log(
                *t,
                *callsite,
                *level,
                *source,
                message,
                fields.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            ),
            Event::Marker { t, note } => self.marker(*t, note),
            Event::Dropped { t, count } => self.dropped(*t, *count),
            Event::Panic {
                t,
                message,
                location,
                backtrace,
            } => self.panic(*t, message, location, backtrace),
            Event::Alloc { t, bytes, count } => self.alloc(*t, *bytes, *count),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events == 0
    }

    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events
    }

    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.body.len()
    }

    /// Writes the buffered events as a [`Record::Batch`] for `thread`.
    pub fn encode_record(&self, thread: u32, enc: &mut Encoder) {
        enc.u8(tag::BATCH)
            .uvar(u64::from(thread))
            .uvar(self.base.unwrap_or(0))
            .bytes(self.body.as_slice());
    }

    pub fn clear(&mut self) {
        self.body.clear();
        self.base = None;
        self.last = 0;
        self.events = 0;
    }
}

/// Decodes every record in a decompressed chunk payload.
///
/// # Errors
/// Any [`DecodeError`] for malformed input.
pub fn decode_records(payload: &[u8]) -> Result<Vec<Record>, DecodeError> {
    let mut dec = Decoder::new(payload);
    let mut records = Vec::new();
    while !dec.is_empty() {
        records.push(Record::decode(&mut dec)?);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let records = vec![
            Record::Session(SessionInfo {
                session_id: 42,
                app: "demo".into(),
                app_version: "0.1.0".into(),
                build: "abc".into(),
                platform: "linux".into(),
                device: "host".into(),
                pid: 7,
                started_unix_ns: 1_700_000_000_000_000_000,
                sismo_version: "0.1.0".into(),
                app_crates: vec!["demo_*".into()],
            }),
            Record::Thread(ThreadDef {
                id: 1,
                name: "main".into(),
            }),
            Record::Callsite(CallsiteDef {
                id: 3,
                kind: CallsiteKind::Span,
                name: "demo::work".into(),
                target: "demo".into(),
                module_path: "demo::work".into(),
                file: "src/main.rs".into(),
                line: 12,
                level: Level::Info,
            }),
            Record::Batch {
                thread: 1,
                events: vec![
                    Event::Begin {
                        t: 100,
                        callsite: 3,
                    },
                    Event::Counter {
                        t: 150,
                        callsite: 4,
                        value: 2.0,
                    },
                    Event::Log {
                        t: 160,
                        callsite: 5,
                        level: Level::Warn,
                        source: Source::Log,
                        message: "careful".into(),
                        fields: vec![("k".into(), "v".into())],
                    },
                    Event::End { t: 400 },
                    Event::Frame { t: 401 },
                    Event::AsyncBegin {
                        t: 500,
                        callsite: 3,
                        id: 9,
                    },
                    Event::AsyncEnd { t: 900, id: 9 },
                    Event::Marker {
                        t: 901,
                        note: "here".into(),
                    },
                    Event::Dropped { t: 902, count: 3 },
                    Event::Alloc {
                        t: 902,
                        bytes: 4096,
                        count: 2,
                    },
                    Event::Panic {
                        t: 903,
                        message: "boom".into(),
                        location: "src/main.rs:1".into(),
                        backtrace: "0: main".into(),
                    },
                ],
            },
        ];
        let mut enc = Encoder::default();
        for record in &records {
            record.encode(&mut enc);
        }
        assert_eq!(decode_records(enc.as_slice()).unwrap(), records);
    }

    #[test]
    fn crate_name_uses_module_path() {
        let def = CallsiteDef {
            id: 1,
            kind: CallsiteKind::Span,
            name: "x".into(),
            target: "custom".into(),
            module_path: "freya_core::tree".into(),
            file: String::new(),
            line: 0,
            level: Level::Trace,
        };
        assert_eq!(def.crate_name(), "freya_core");
    }
}

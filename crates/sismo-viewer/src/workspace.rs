//! UI-independent state of the viewer: open sessions, their queries,
//! selections and live connections.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, BufWriter};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sismo_analysis::query::{Matcher, Term, TermKind};
use sismo_analysis::{LoadReport, Query, Session};
use sismo_format::{Chunk, DecodeError, FileWriter};

use crate::client::{ClientEvent, Command, Connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Timeline,
    Flamegraph,
    Functions,
    Logs,
    Metrics,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Timeline,
        Tab::Flamegraph,
        Tab::Functions,
        Tab::Logs,
        Tab::Metrics,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Tab::Timeline => "tab-timeline",
            Tab::Flamegraph => "tab-flamegraph",
            Tab::Functions => "tab-functions",
            Tab::Logs => "tab-logs",
            Tab::Metrics => "tab-metrics",
        }
    }
}

/// Where a live connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveStatus {
    Connecting,
    NeedsPairing { rejected: Option<String> },
    Streaming,
    Lost { reason: String, since: Instant },
}

/// What to do when recording starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFrom {
    Buffer,
    Now,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRule {
    Manual,
    After(Duration),
    Size(u64),
    FirstError,
}

/// A recording of a live session to a file on this machine.
pub struct Recording {
    pub path: PathBuf,
    writer: FileWriter<BufWriter<File>>,
    pub started: Instant,
    pub markers: u32,
    pub stop: StopRule,
    error_seen_at: Option<Instant>,
}

impl Recording {
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.writer.bytes_written()
    }

    fn should_stop(&self) -> bool {
        match self.stop {
            StopRule::Manual => false,
            StopRule::After(limit) => self.started.elapsed() >= limit,
            StopRule::Size(limit) => self.bytes() >= limit,
            StopRule::FirstError => self
                .error_seen_at
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(10)),
        }
    }
}

/// Finished recording, shown in a toast.
#[derive(Debug, Clone)]
pub struct Saved {
    pub path: PathBuf,
    pub bytes: u64,
    pub duration: Duration,
    pub markers: u32,
}

pub struct Live {
    pub addr: SocketAddr,
    pub connection: Option<Connection>,
    pub status: LiveStatus,
    pub app_filter: String,
    pub history_bytes: u64,
    pub rtt: Option<Duration>,
    /// Every chunk received, so a recording can include what came before.
    pub received: Vec<Vec<u8>>,
    pub received_bytes: u64,
    pub recording: Option<Recording>,
    pub device_upload: Option<(PathBuf, Vec<u8>, u64)>,
    pub last_chunk: Option<Instant>,
    pub events_per_sec: f64,
    rate_window: (Instant, u64),
}

pub enum Origin {
    File { path: PathBuf, report: LoadReport },
    Live(Box<Live>),
}

/// One open session tab.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view toggles, not a state machine"
)]
pub struct SessionView {
    pub id: u64,
    pub origin: Origin,
    pub session: Session,
    pub tab: Tab,
    pub query_text: String,
    pub query: Query,
    pub query_error: Option<String>,
    pub hidden_crates: BTreeSet<String>,
    pub hidden_threads: BTreeSet<u32>,
    /// Visible time range of the timeline.
    pub view: (u64, u64),
    /// Range picked on the frames strip, if any.
    pub selection: Option<(u64, u64)>,
    pub selected_frame: Option<u32>,
    pub selected_span: Option<(u32, u32)>,
    pub selected_function: Option<u32>,
    pub selected_log: Option<usize>,
    /// Flamegraph focus path (callsites from the root).
    pub flame_focus: Vec<u32>,
    pub paused: bool,
    pub follow: bool,
    /// Bumped whenever data or view changes; the UI keys repaints on it.
    pub revision: u64,
    /// Last measured size of named canvases, for hit testing.
    pub canvas: std::collections::HashMap<&'static str, (f32, f32)>,
    pub sort: sismo_analysis::SortBy,
    pub hidden_levels: BTreeSet<sismo_format::Level>,
    pub hidden_sources: Vec<sismo_format::Source>,
    /// Recording to diff against (A), this session being B.
    pub compare: Option<(PathBuf, Box<Session>)>,
    pub compare_metric: sismo_analysis::Metric,
    /// Whether the per-app crate groups from the preferences were applied.
    pub groups_applied: bool,
    /// Set when pairing just succeeded, so the viewer can remember the app.
    pub paired_now: bool,
    /// Time span the timeline keeps while following a live app.
    pub live_window_ns: u64,
}

impl SessionView {
    fn new(id: u64, origin: Origin, session: Session) -> Self {
        let (start, end) = session.span();
        Self {
            id,
            origin,
            session,
            tab: Tab::Timeline,
            query_text: String::new(),
            query: Query::default(),
            query_error: None,
            hidden_crates: BTreeSet::new(),
            hidden_threads: BTreeSet::new(),
            view: (start, end.max(start + 1)),
            selection: None,
            selected_frame: None,
            selected_span: None,
            selected_function: None,
            selected_log: None,
            flame_focus: Vec::new(),
            paused: false,
            follow: true,
            revision: 0,
            canvas: std::collections::HashMap::new(),
            sort: sismo_analysis::SortBy::SelfTime,
            hidden_levels: BTreeSet::new(),
            hidden_sources: Vec::new(),
            compare: None,
            compare_metric: sismo_analysis::Metric::P95,
            groups_applied: false,
            paired_now: false,
            live_window_ns: 3_000_000_000,
        }
    }

    /// Opens a recording.
    ///
    /// # Errors
    /// [`sismo_analysis::LoadError`] when the file cannot be read.
    pub fn open_file(id: u64, path: &Path) -> Result<Self, sismo_analysis::LoadError> {
        let (session, report) = Session::load(path)?;
        let mut view = Self::new(
            id,
            Origin::File {
                path: path.to_path_buf(),
                report,
            },
            session,
        );
        view.follow = false;
        Ok(view)
    }

    /// Starts connecting to a live app.
    ///
    /// # Errors
    /// I/O errors starting the connection thread.
    pub fn connect(id: u64, addr: SocketAddr) -> io::Result<Self> {
        let connection = Connection::open(addr, format!("sismo {}", env!("CARGO_PKG_VERSION")))?;
        let live = Live {
            addr,
            connection: Some(connection),
            status: LiveStatus::Connecting,
            app_filter: String::new(),
            history_bytes: 0,
            rtt: None,
            received: Vec::new(),
            received_bytes: 0,
            recording: None,
            device_upload: None,
            last_chunk: None,
            events_per_sec: 0.0,
            rate_window: (Instant::now(), 0),
        };
        Ok(Self::new(
            id,
            Origin::Live(Box::new(live)),
            Session::default(),
        ))
    }

    #[must_use]
    pub fn live(&self) -> Option<&Live> {
        match &self.origin {
            Origin::Live(live) => Some(live),
            Origin::File { .. } => None,
        }
    }

    pub fn live_mut(&mut self) -> Option<&mut Live> {
        match &mut self.origin {
            Origin::Live(live) => Some(live),
            Origin::File { .. } => None,
        }
    }

    #[must_use]
    pub fn title(&self) -> String {
        match &self.origin {
            Origin::File { path, .. } => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Origin::Live(_) => format!(
                "{} {} · {} · {}",
                self.session.info.app,
                self.session.info.app_version,
                self.session.info.device,
                self.session.info.platform
            ),
        }
    }

    pub fn set_query(&mut self, text: String) {
        match Query::parse(&text) {
            Ok(query) => {
                self.query = query;
                self.query_error = None;
            }
            Err(error) => self.query_error = Some(error.to_string()),
        }
        self.query_text = text;
        self.revision += 1;
    }

    /// The text query plus the crates and threads hidden in the side panel.
    #[must_use]
    pub fn effective_query(&self) -> Query {
        let mut query = self.query.clone();
        if self.hidden_crates.is_empty() && self.hidden_threads.is_empty() {
            return query;
        }
        let mut extra: Vec<Term> = self
            .hidden_crates
            .iter()
            .map(|c| Term {
                negate: true,
                kind: TermKind::Crate(vec![Matcher::Exact(c.to_lowercase())], Vec::new()),
            })
            .collect();
        extra.extend(self.hidden_threads.iter().map(|id| Term {
            negate: true,
            kind: TermKind::Thread(vec![Matcher::Exact(
                self.session.thread_name(*id).to_lowercase(),
            )]),
        }));
        if query.groups.is_empty() {
            query.groups.push(extra);
        } else {
            for group in &mut query.groups {
                group.extend(extra.iter().cloned());
            }
        }
        query
    }

    /// Window used by aggregations: the selection, else the query range.
    #[must_use]
    pub fn window(&self) -> (u64, u64) {
        self.selection
            .unwrap_or_else(|| self.query.window(&self.session))
    }

    pub fn toggle_crate(&mut self, krate: &str) {
        if !self.hidden_crates.remove(krate) {
            self.hidden_crates.insert(krate.to_owned());
        }
        self.revision += 1;
    }

    pub fn toggle_thread(&mut self, id: u32) {
        if !self.hidden_threads.remove(&id) {
            self.hidden_threads.insert(id);
        }
        self.revision += 1;
    }

    pub fn send(&self, command: Command) {
        if let Some(conn) = self.live().and_then(|l| l.connection.as_ref()) {
            conn.send(command);
        }
    }

    /// Drains pending network events; returns a finished recording, if any.
    pub fn poll_live(&mut self) -> Option<Saved> {
        let Origin::Live(live) = &mut self.origin else {
            return None;
        };
        let conn = live.connection.as_ref()?;
        let events: Vec<ClientEvent> = conn.events.try_iter().collect();
        if events.is_empty() {
            return None;
        }
        let mut changed = false;
        for event in events {
            match event {
                ClientEvent::Welcome {
                    session,
                    pairing_required,
                    filter,
                    history_bytes,
                } => {
                    self.session.info = session;
                    live.app_filter = filter;
                    live.history_bytes = history_bytes;
                    if pairing_required {
                        live.status = LiveStatus::NeedsPairing { rejected: None };
                    } else {
                        live.status = LiveStatus::Streaming;
                        conn.send(Command::Subscribe { history: true });
                    }
                }
                ClientEvent::Paired => {
                    self.paired_now = true;
                    live.status = LiveStatus::Streaming;
                    conn.send(Command::Subscribe { history: true });
                }
                ClientEvent::PairRejected(reason) => {
                    live.status = LiveStatus::NeedsPairing {
                        rejected: Some(reason),
                    };
                }
                ClientEvent::Chunk(bytes) => {
                    live.received_bytes += bytes.len() as u64;
                    live.last_chunk = Some(Instant::now());
                    if let Some(recording) = &mut live.recording
                        && let Ok(chunk) = Chunk::from_bytes(bytes.clone())
                    {
                        let _ = recording.writer.write_chunk(&chunk);
                    }
                    let before = self.session.events;
                    if self.session.ingest_chunk(bytes.clone()).is_ok() {
                        changed = true;
                    }
                    live.rate_window.1 += self.session.events - before;
                    live.received.push(bytes);
                }
                ClientEvent::HistoryEnd => changed = true,
                ClientEvent::Filter(filter) => live.app_filter = filter,
                ClientEvent::DeviceRecording {
                    offset,
                    total,
                    bytes,
                } => {
                    if let Some((_, buffer, expected)) = &mut live.device_upload {
                        *expected = total;
                        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                        if buffer.len() < offset + bytes.len() {
                            buffer.resize(offset + bytes.len(), 0);
                        }
                        buffer[offset..offset + bytes.len()].copy_from_slice(&bytes);
                    }
                }
                ClientEvent::Rtt(rtt) => live.rtt = Some(rtt),
                ClientEvent::Lost(reason) => {
                    live.status = LiveStatus::Lost {
                        reason,
                        since: Instant::now(),
                    };
                }
            }
        }
        let elapsed = live.rate_window.0.elapsed();
        if elapsed >= Duration::from_secs(1) {
            #[allow(clippy::cast_precision_loss)]
            let rate = live.rate_window.1 as f64 / elapsed.as_secs_f64();
            live.events_per_sec = rate;
            live.rate_window = (Instant::now(), 0);
        }
        if let Some((path, buffer, total)) = &live.device_upload
            && *total > 0
            && buffer.len() as u64 >= *total
        {
            let _ = std::fs::write(path, buffer);
            live.device_upload = None;
        }
        if changed {
            if self.follow && !self.paused {
                let (start, end) = self.session.span();
                let current = self.view.1.saturating_sub(self.view.0);
                let width = if current < 1_000_000 {
                    self.live_window_ns
                } else {
                    current
                };
                self.view = (end.saturating_sub(width).max(start), end.max(start + 1));
            }
            self.revision += 1;
        }
        let error_seen = self
            .session
            .logs
            .last()
            .is_some_and(|l| l.level == sismo_format::Level::Error);
        let Origin::Live(live) = &mut self.origin else {
            return None;
        };
        if let Some(recording) = &mut live.recording {
            if error_seen && recording.error_seen_at.is_none() {
                recording.error_seen_at = Some(Instant::now());
            }
            if recording.should_stop() {
                return self.stop_recording();
            }
        }
        None
    }

    /// Starts writing the live stream to `path`.
    ///
    /// # Errors
    /// I/O errors creating the file.
    pub fn start_recording(
        &mut self,
        path: PathBuf,
        from: RecordFrom,
        stop: StopRule,
        on_device: bool,
    ) -> io::Result<()> {
        let Origin::Live(live) = &mut self.origin else {
            return Err(io::Error::other("only live sessions can record"));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = FileWriter::new(BufWriter::new(File::create(&path)?))?;
        let backlog: &[Vec<u8>] = match from {
            RecordFrom::Buffer => &live.received,
            // The first chunk carries every definition; later chunks need it.
            RecordFrom::Now => live.received.get(..1).unwrap_or(&[]),
        };
        for bytes in backlog {
            if let Ok(chunk) = Chunk::from_bytes(bytes.clone()) {
                writer.write_chunk(&chunk)?;
            }
        }
        live.recording = Some(Recording {
            path,
            writer,
            started: Instant::now(),
            markers: 0,
            stop,
            error_seen_at: None,
        });
        if on_device && let Some(conn) = &live.connection {
            conn.send(Command::StartDeviceRecording);
        }
        Ok(())
    }

    /// Finishes the recording in progress.
    pub fn stop_recording(&mut self) -> Option<Saved> {
        let Origin::Live(live) = &mut self.origin else {
            return None;
        };
        let mut recording = live.recording.take()?;
        let _ = recording.writer.flush();
        if let Some(conn) = &live.connection {
            conn.send(Command::StopDeviceRecording);
            let device_path = recording.path.with_extension("device.sismo");
            live.device_upload = Some((device_path, Vec::new(), 0));
        }
        Some(Saved {
            bytes: recording.bytes(),
            path: recording.path,
            duration: recording.started.elapsed(),
            markers: recording.markers,
        })
    }

    pub fn add_marker(&mut self, note: String) {
        self.send(Command::Marker(note));
        if let Some(recording) = self.live_mut().and_then(|l| l.recording.as_mut()) {
            recording.markers += 1;
        }
    }

    /// Saves every chunk received so far (used after a connection drops).
    ///
    /// # Errors
    /// I/O errors writing the file.
    pub fn save_received(&self, path: &Path) -> io::Result<u64> {
        let live = self
            .live()
            .ok_or_else(|| io::Error::other("not a live session"))?;
        let mut writer = FileWriter::new(BufWriter::new(File::create(path)?))?;
        for bytes in &live.received {
            let chunk = Chunk::from_bytes(bytes.clone())
                .map_err(|e: DecodeError| io::Error::other(e.to_string()))?;
            writer.write_chunk(&chunk)?;
        }
        writer.flush()?;
        Ok(writer.bytes_written())
    }

    /// Reconnects after a lost connection.
    ///
    /// # Errors
    /// I/O errors starting the connection thread.
    pub fn reconnect(&mut self) -> io::Result<()> {
        let Origin::Live(live) = &mut self.origin else {
            return Ok(());
        };
        live.connection = Some(Connection::open(
            live.addr,
            format!("sismo {}", env!("CARGO_PKG_VERSION")),
        )?);
        live.status = LiveStatus::Connecting;
        Ok(())
    }
}

/// Default folder for recordings made from the viewer.
#[must_use]
pub fn recordings_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("sismo")
}

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
use sismo_format::{Chunk, DecodeError, FileWriter, SessionInfo};

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

/// Backoff for reconnecting a lost live session: 1 s, doubling, up to 30 s.
#[derive(Debug, Clone, Copy, Default)]
pub struct Retry {
    attempts: u32,
    next: Option<Instant>,
}

impl Retry {
    const FIRST: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(30);

    /// Plans the next attempt after one more failure.
    pub fn schedule(&mut self) {
        let delay = Self::FIRST
            .saturating_mul(1 << self.attempts.min(5))
            .min(Self::MAX);
        self.attempts += 1;
        self.next = Some(Instant::now() + delay);
    }

    /// Whether an attempt is planned and its time has come.
    #[must_use]
    pub fn due(&self) -> bool {
        self.next.is_some_and(|t| Instant::now() >= t)
    }

    /// Time left before the next attempt, for the lost banner.
    #[must_use]
    pub fn wait(&self) -> Option<Duration> {
        self.next
            .map(|t| t.saturating_duration_since(Instant::now()))
    }
}

/// A recording made on the device, arriving in pieces. Complete only when
/// every byte arrived: a lost piece must not end up as zeros in the file.
#[derive(Debug)]
pub struct DeviceUpload {
    pub path: PathBuf,
    pub buffer: Vec<u8>,
    total: u64,
    pieces: std::collections::BTreeMap<u64, u64>,
}

impl DeviceUpload {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            buffer: Vec::new(),
            total: 0,
            pieces: std::collections::BTreeMap::new(),
        }
    }

    pub fn add(&mut self, offset: u64, total: u64, bytes: &[u8]) {
        self.total = total;
        let Ok(start) = usize::try_from(offset) else {
            return;
        };
        let Some(end) = start.checked_add(bytes.len()) else {
            return;
        };
        if self.buffer.len() < end {
            self.buffer.resize(end, 0);
        }
        self.buffer[start..end].copy_from_slice(bytes);
        self.pieces.insert(offset, bytes.len() as u64);
    }

    #[must_use]
    pub fn complete(&self) -> bool {
        self.total > 0 && self.pieces.values().sum::<u64>() >= self.total
    }
}

/// A run of the app that started while this tab showed an older one.
#[derive(Debug)]
pub struct NextRun {
    pub info: SessionInfo,
    chunks: Vec<Vec<u8>>,
    last_seq: u64,
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
    /// How many of `received` the session has applied; the rest arrived
    /// while paused.
    pub applied: usize,
    pub received_bytes: u64,
    pub recording: Option<Recording>,
    pub device_upload: Option<DeviceUpload>,
    /// Highest chunk seq received from the current app session.
    pub last_seq: u64,
    /// A new run of the app (it restarted), held until the user decides.
    pub next_run: Option<NextRun>,
    /// When to try again after the connection is lost.
    pub retry: Retry,
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
    /// Whether the logs list stays at its newest line.
    pub logs_follow: bool,
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
            logs_follow: true,
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
            applied: 0,
            received_bytes: 0,
            recording: None,
            device_upload: None,
            last_seq: 0,
            retry: Retry::default(),
            next_run: None,
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

    /// Pauses or resumes a live session. While paused nothing new is shown
    /// (it is still received, recorded and saved); resuming applies it all.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.follow = !paused;
        if !paused && let Origin::Live(live) = &mut self.origin {
            ingest_pending(&mut self.session, &live.received, &mut live.applied);
            if self.selection.is_none() {
                let (start, end) = self.session.span();
                let width = self.view.1.saturating_sub(self.view.0).max(1_000_000);
                self.view = (end.saturating_sub(width).max(start), end.max(start + 1));
            }
        }
        self.revision += 1;
    }

    /// Drops the picked range: a live session follows its latest seconds
    /// again, a recording shows everything.
    pub fn reset_view(&mut self) {
        self.selection = None;
        self.selected_frame = None;
        if self.live().is_some() {
            self.set_paused(false);
        }
        let (start, end) = self.session.span();
        if self.live().is_some() {
            self.follow = true;
            self.logs_follow = true;
            self.view = (
                end.saturating_sub(self.live_window_ns).max(start),
                end.max(start + 1),
            );
        } else {
            self.view = (start, end.max(start + 1));
        }
        self.revision += 1;
    }

    /// Whether the view shows something other than its default range.
    #[must_use]
    pub fn view_moved(&self) -> bool {
        if self.selection.is_some() {
            return true;
        }
        if self.live().is_some() {
            return !self.follow;
        }
        let (start, end) = self.session.span();
        self.view != (start, end.max(start + 1))
    }

    /// Default name for files saved from this session.
    #[must_use]
    pub fn file_stem(&self) -> String {
        self.session.info.file_stem(sismo_format::FileStamp::now())
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
        let mut subscribe = false;
        for event in events {
            match event {
                ClientEvent::Welcome {
                    session,
                    pairing_required,
                    filter,
                    history_bytes,
                } => {
                    let known = self.session.info.session_id;
                    if known == 0 || known == session.session_id {
                        self.session.info = session;
                    } else if live
                        .next_run
                        .as_ref()
                        .is_none_or(|next| next.info.session_id != session.session_id)
                    {
                        // The app restarted: its timeline starts over and its
                        // ids mean other things, so it cannot be merged. Hold
                        // the new run until the user decides about this one.
                        live.next_run = Some(NextRun {
                            info: session,
                            chunks: Vec::new(),
                            last_seq: 0,
                        });
                    }
                    live.app_filter = filter;
                    live.history_bytes = history_bytes;
                    live.retry = Retry::default();
                    if pairing_required {
                        live.status = LiveStatus::NeedsPairing { rejected: None };
                    } else {
                        live.status = LiveStatus::Streaming;
                        subscribe = true;
                    }
                }
                ClientEvent::Paired => {
                    self.paired_now = true;
                    live.status = LiveStatus::Streaming;
                    subscribe = true;
                }
                ClientEvent::PairRejected(reason) => {
                    live.status = LiveStatus::NeedsPairing {
                        rejected: Some(reason),
                    };
                }
                ClientEvent::Chunk { seq, bytes } => {
                    // Seq 0 carries definitions, safe to apply again; anything
                    // else at or below what we have is a replay.
                    if let Some(next) = &mut live.next_run {
                        if seq == 0 || seq > next.last_seq {
                            next.last_seq = next.last_seq.max(seq);
                            next.chunks.push(bytes);
                        }
                        continue;
                    }
                    if seq != 0 && seq <= live.last_seq {
                        continue;
                    }
                    live.last_seq = live.last_seq.max(seq);
                    live.received_bytes += bytes.len() as u64;
                    live.last_chunk = Some(Instant::now());
                    if let Some(recording) = &mut live.recording
                        && let Ok(chunk) = Chunk::from_bytes(bytes.clone())
                    {
                        let _ = recording.writer.write_chunk(&chunk);
                    }
                    live.received.push(bytes);
                    // Paused: keep what arrives without showing it, so the
                    // screen stays exactly as it was until resumed.
                    if !self.paused {
                        let before = self.session.events;
                        changed |=
                            ingest_pending(&mut self.session, &live.received, &mut live.applied);
                        live.rate_window.1 += self.session.events - before;
                    }
                }
                ClientEvent::HistoryEnd => changed = true,
                ClientEvent::Filter(filter) => live.app_filter = filter,
                ClientEvent::DeviceRecording {
                    offset,
                    total,
                    bytes,
                } => {
                    if let Some(upload) = &mut live.device_upload {
                        upload.add(offset, total, &bytes);
                    }
                }
                ClientEvent::Rtt(rtt) => live.rtt = Some(rtt),
                ClientEvent::Lost(reason) => {
                    live.status = LiveStatus::Lost {
                        reason,
                        since: Instant::now(),
                    };
                    live.retry.schedule();
                }
            }
        }
        let Origin::Live(live) = &mut self.origin else {
            return None;
        };
        if subscribe && let Some(conn) = &live.connection {
            conn.send(Command::Subscribe {
                history: true,
                since: live
                    .next_run
                    .as_ref()
                    .map_or(live.last_seq, |next| next.last_seq),
            });
        }
        let elapsed = live.rate_window.0.elapsed();
        // While paused nothing is decoded; keep showing the last rate.
        if elapsed >= Duration::from_secs(1) && !self.paused {
            #[allow(clippy::cast_precision_loss)]
            let rate = live.rate_window.1 as f64 / elapsed.as_secs_f64();
            live.events_per_sec = rate;
            live.rate_window = (Instant::now(), 0);
        }
        if live
            .device_upload
            .as_ref()
            .is_some_and(DeviceUpload::complete)
            && let Some(upload) = live.device_upload.take()
        {
            let _ = std::fs::write(&upload.path, &upload.buffer);
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
            live.device_upload = Some(DeviceUpload::new(device_path));
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

    /// Forgets what a live app sent so far, keeping the connection. Saving
    /// afterwards writes the app's definitions first, so the file still
    /// names every function.
    ///
    /// # Errors
    /// Encoding the definitions chunk failed; nothing was cleared.
    pub fn clear_received(&mut self) -> Result<(), DecodeError> {
        let mut enc = sismo_format::Encoder::with_capacity(16 * 1024);
        for record in self.session.definitions() {
            record.encode(&mut enc);
        }
        let definitions = Chunk::seal(enc.as_slice(), sismo_format::Codec::Lz4)?;
        let Origin::Live(live) = &mut self.origin else {
            return Ok(());
        };
        live.received = vec![definitions.into_bytes()];
        live.applied = live.received.len();
        live.received_bytes = 0;
        self.session.clear_events();
        self.selected_span = None;
        self.selected_function = None;
        self.selected_log = None;
        self.flame_focus.clear();
        self.reset_view();
        Ok(())
    }

    /// Reconnects after a lost connection.
    ///
    /// # Errors
    /// I/O errors starting the connection thread.
    pub fn reconnect(&mut self) -> io::Result<()> {
        let addr = self.live().map(|l| l.addr);
        match addr {
            Some(addr) => self.reconnect_to(addr),
            None => Ok(()),
        }
    }

    /// Reconnects to `addr`, which may differ from the old address when the
    /// app came back on another port or network. What was received stays;
    /// the app only resends what came after it, or everything if it is a
    /// new run of the app.
    ///
    /// # Errors
    /// I/O errors starting the connection thread.
    pub fn reconnect_to(&mut self, addr: SocketAddr) -> io::Result<()> {
        let Origin::Live(live) = &mut self.origin else {
            return Ok(());
        };
        // Drop the old connection first so its thread says goodbye.
        live.connection = None;
        live.connection = Some(Connection::open(
            addr,
            format!("sismo {}", env!("CARGO_PKG_VERSION")),
        )?);
        live.addr = addr;
        live.status = LiveStatus::Connecting;
        live.rtt = None;
        live.retry.next = None;
        Ok(())
    }

    /// The new run of the app, waiting while the user decides what to do
    /// with the current one.
    #[must_use]
    pub fn next_run(&self) -> Option<&NextRun> {
        self.live().and_then(|l| l.next_run.as_ref())
    }

    /// Leaves the current run for the one waiting: the recording of the
    /// old run is closed, the view starts empty and shows the new run from
    /// its first chunk (it arrived while the user was deciding).
    pub fn switch_to_next_run(&mut self) -> Option<Saved> {
        let next = self.live_mut().and_then(|l| l.next_run.take())?;
        let saved = self.start_over(next.info);
        if let Origin::Live(live) = &mut self.origin {
            live.last_seq = next.last_seq;
            live.received_bytes = next.chunks.iter().map(|c| c.len() as u64).sum();
            live.received = next.chunks;
            if !self.paused {
                ingest_pending(&mut self.session, &live.received, &mut live.applied);
            }
        }
        self.reset_view();
        saved
    }

    /// Starts an empty session for run `info`, closing the recording (it
    /// belongs to the old run).
    fn start_over(&mut self, info: SessionInfo) -> Option<Saved> {
        let saved = self.stop_recording();
        self.session = Session::default();
        self.session.info = info;
        self.groups_applied = false;
        self.selection = None;
        self.selected_frame = None;
        self.selected_span = None;
        self.selected_function = None;
        self.selected_log = None;
        self.flame_focus.clear();
        if let Origin::Live(live) = &mut self.origin {
            live.received.clear();
            live.applied = 0;
            live.received_bytes = 0;
            live.last_seq = 0;
            live.device_upload = None;
        }
        self.reset_view();
        saved
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

/// Applies the chunks of `received` past `applied`; returns whether any
/// decoded.
fn ingest_pending(session: &mut Session, received: &[Vec<u8>], applied: &mut usize) -> bool {
    let mut changed = false;
    for bytes in &received[(*applied).min(received.len())..] {
        changed |= session.ingest_chunk(bytes.clone()).is_ok();
    }
    *applied = received.len();
    changed
}

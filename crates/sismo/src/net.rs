use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sismo_format::net::{AuthMode, Beacon, DISCOVERY_APP_ID, ToRecorder, ToViewer, stream};
use sismo_format::{Chunk, FORMAT_VERSION, FileWriter};
use tokio::sync::mpsc;
use zudp::{Discovery, DiscoveryConfig};
use zudp::{Keypair, PeerEvent, RawBytes, Zudp, ZudpSender, ZudpSocket};

use crate::collector::snapshot_chunk;
use crate::config::LiveConfig;
use crate::handle::Handle;
use crate::registry::registry;
use crate::sink::{Sink, lock};

/// Viewers that stay silent this long are dropped (they ping every 2 s).
const VIEWER_TIMEOUT: Duration = Duration::from_secs(8);
/// Wrong pairing codes per IP before pairing locks for a while.
const MAX_PAIR_ATTEMPTS: u32 = 5;
/// First lockout; each further one doubles, up to [`MAX_PAIR_LOCKOUT`].
const PAIR_LOCKOUT: Duration = Duration::from_secs(30);
const MAX_PAIR_LOCKOUT: Duration = Duration::from_mins(15);
/// Piece size for on-device recording uploads.
const UPLOAD_PIECE: usize = 192 * 1024;

type FlushRequest = std::sync::mpsc::SyncSender<()>;

pub(crate) struct NetHandle {
    pairing_code: String,
    flush: mpsc::UnboundedSender<FlushRequest>,
}

impl NetHandle {
    pub fn pairing_code(&self) -> &str {
        &self.pairing_code
    }

    /// Waits, up to `timeout`, until every chunk handed to the network so
    /// far went out to the viewers.
    pub fn flush(&self, timeout: Duration) {
        let (reply, done) = std::sync::mpsc::sync_channel(1);
        if self.flush.send(reply).is_ok() {
            let _ = done.recv_timeout(timeout);
        }
    }
}

type ChunkFeed = mpsc::Sender<(u64, Arc<Chunk>)>;

/// Collector sink forwarding sealed chunks to the network task.
struct NetSink {
    feed: ChunkFeed,
}

impl Sink for NetSink {
    fn open(&mut self, _snapshot: &Chunk) {}

    fn write(&mut self, seq: u64, chunk: &Arc<Chunk>) -> bool {
        match self.feed.try_send((seq, Arc::clone(chunk))) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn flush(&mut self) {}
}

/// On-device copy written while a viewer asked for it.
#[derive(Default)]
struct DeviceRecording {
    writer: Option<FileWriter<BufWriter<File>>>,
    path: Option<PathBuf>,
}

struct DeviceSink {
    state: Arc<Mutex<DeviceRecording>>,
}

impl Sink for DeviceSink {
    fn open(&mut self, _snapshot: &Chunk) {}

    fn write(&mut self, _seq: u64, chunk: &Arc<Chunk>) -> bool {
        if let Some(writer) = lock(&self.state).writer.as_mut() {
            let _ = writer.write_chunk(chunk);
        }
        true
    }

    fn flush(&mut self) {
        if let Some(writer) = lock(&self.state).writer.as_mut() {
            let _ = writer.flush();
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(sismo_format::unix_ns());
    hasher.finish()
}

/// Persistent identity: keypair plus the viewers already paired.
struct Identity {
    keypair: Keypair,
    trusted: HashSet<[u8; 32]>,
    dir: Option<PathBuf>,
}

impl Identity {
    const KEY_FILE: &'static str = "sismo-key";
    const TRUSTED_FILE: &'static str = "sismo-trusted";

    fn load(dir: Option<&Path>) -> Self {
        let mut identity = Self {
            keypair: Keypair::generate(),
            trusted: HashSet::new(),
            dir: dir.map(Path::to_path_buf),
        };
        let Some(dir) = dir else {
            return identity;
        };
        match std::fs::read(dir.join(Self::KEY_FILE)) {
            Ok(bytes) if bytes.len() == 64 => {
                identity.keypair.public.copy_from_slice(&bytes[..32]);
                identity.keypair.private.copy_from_slice(&bytes[32..]);
            }
            _ => identity.save_key(),
        }
        if let Ok(bytes) = std::fs::read(dir.join(Self::TRUSTED_FILE)) {
            identity.trusted = bytes.as_chunks::<32>().0.iter().copied().collect();
        }
        identity
    }

    fn save_key(&self) {
        if let Some(dir) = &self.dir {
            let mut bytes = self.keypair.public.to_vec();
            bytes.extend_from_slice(&self.keypair.private);
            let _ = std::fs::create_dir_all(dir)
                .and_then(|()| std::fs::write(dir.join(Self::KEY_FILE), bytes));
        }
    }

    fn trust(&mut self, key: [u8; 32]) {
        if self.trusted.insert(key)
            && let Some(dir) = &self.dir
        {
            let bytes: Vec<u8> = self.trusted.iter().flatten().copied().collect();
            let _ = std::fs::write(dir.join(Self::TRUSTED_FILE), bytes);
        }
    }
}

#[derive(Default)]
struct Viewer {
    authorized: bool,
    subscribed: bool,
    last_seq: u64,
    /// Static key the viewer said hello with; authorization is bound to it.
    key: Option<[u8; 32]>,
    last_seen: Option<Instant>,
}

/// Wrong pairing codes seen from one IP and when pairing opens again.
struct Attempts {
    failures: u32,
    locked_until: Option<Instant>,
}

impl Attempts {
    /// Lockout after every [`MAX_PAIR_ATTEMPTS`] failures, doubling each
    /// time: ephemeral viewer ports make a per-address limit worthless.
    fn fail(&mut self) {
        self.failures += 1;
        if self.failures.is_multiple_of(MAX_PAIR_ATTEMPTS) {
            let rounds = self.failures / MAX_PAIR_ATTEMPTS;
            let lock = PAIR_LOCKOUT
                .saturating_mul(1 << rounds.min(5).saturating_sub(1))
                .min(MAX_PAIR_LOCKOUT);
            self.locked_until = Some(Instant::now() + lock);
        }
    }

    fn locked_for(&self) -> Option<Duration> {
        self.locked_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
    }
}

struct Server {
    sender: ZudpSender,
    identity: Identity,
    auth: AuthMode,
    code: String,
    viewers: HashMap<SocketAddr, Viewer>,
    attempts: HashMap<IpAddr, Attempts>,
    handle: Handle,
    device: Arc<Mutex<DeviceRecording>>,
    device_dir: PathBuf,
    /// Viewer that started the on-device recording.
    device_owner: Option<SocketAddr>,
}

impl Server {
    async fn send(&self, to: SocketAddr, stream_id: u16, message: &ToViewer) {
        let bytes = RawBytes(message.encode().into());
        let _ = self.sender.send_stream(bytes, to, stream_id).await;
    }

    async fn on_message(
        &mut self,
        socket: &ZudpSocket<RawBytes>,
        from: SocketAddr,
        message: ToRecorder,
    ) {
        let key = socket.peer_remote_static_key(from);
        if let ToRecorder::Hello { .. } = message {
            self.hello(from, key).await;
            return;
        }
        let Some(viewer) = self.viewers.get_mut(&from) else {
            // Nothing but a hello opens a conversation.
            return;
        };
        viewer.last_seen = Some(Instant::now());
        // A different key at a known address is a different viewer that
        // reused the port: it gets nothing until it says hello. A key that
        // was not known yet at hello (handshake still settling) is adopted.
        match (viewer.key, key) {
            (Some(known), Some(now)) if known != now => {
                self.viewers.remove(&from);
                return;
            }
            (None, Some(now)) => viewer.key = Some(now),
            _ => {}
        }
        match message {
            ToRecorder::Hello { .. } => {}
            ToRecorder::Pair { code } => self.pair(from, key, &code).await,
            ToRecorder::Subscribe { history, since } => {
                if self.authorized(from) {
                    self.subscribe(from, history, since).await;
                }
            }
            ToRecorder::SetFilter { filter } => {
                if !self.authorized(from) {
                    return;
                }
                let _ = self.handle.set_filter(&filter);
                let reply = ToViewer::Filter {
                    filter: self.handle.filter(),
                };
                for addr in self.subscribed() {
                    self.send(addr, stream::CONTROL, &reply).await;
                }
            }
            ToRecorder::Marker { note } => {
                if self.authorized(from) {
                    crate::marker(&note);
                }
            }
            ToRecorder::StartDeviceRecording => {
                if self.authorized(from) {
                    self.start_device_recording(from);
                }
            }
            ToRecorder::StopDeviceRecording => {
                if self.authorized(from) {
                    self.upload_device_recording(from).await;
                }
            }
            ToRecorder::Ping { nonce } => {
                self.send(from, stream::CONTROL, &ToViewer::Pong { nonce })
                    .await;
            }
            ToRecorder::Bye => self.forget(from),
            ToRecorder::FetchCrashReport { name } => {
                if self.authorized(from) {
                    self.upload_crash_report(from, &name).await;
                }
            }
            ToRecorder::DeleteCrashReport { name } => {
                if self.authorized(from) {
                    let _ = self.handle.delete_crash_report(&name);
                    self.send_crash_reports(from).await;
                }
            }
        }
    }

    async fn hello(&mut self, from: SocketAddr, key: Option<[u8; 32]>) {
        // The same viewer from a new address (restart, new port, roaming):
        // drop the old entry so it stops receiving and does not linger.
        if let Some(key) = key {
            let stale: Vec<SocketAddr> = self
                .viewers
                .iter()
                .filter(|(addr, v)| **addr != from && v.key == Some(key))
                .map(|(addr, _)| *addr)
                .collect();
            for addr in stale {
                self.forget(addr);
            }
        }
        let open = self.auth == AuthMode::Open;
        let trusted = key.is_some_and(|k| self.identity.trusted.contains(&k));
        let viewer = Viewer {
            authorized: open || trusted,
            key,
            last_seen: Some(Instant::now()),
            ..Viewer::default()
        };
        let pairing_required = !viewer.authorized;
        self.viewers.insert(from, viewer);
        let welcome = ToViewer::Welcome {
            session: registry().session(),
            pairing_required,
            filter: registry().filter().to_string(),
            history_bytes: self.handle.flight_recorder_bytes() as u64,
        };
        self.send(from, stream::CONTROL, &welcome).await;
    }

    async fn pair(&mut self, from: SocketAddr, key: Option<[u8; 32]>, code: &str) {
        let reject = |reason: String| ToViewer::PairRejected { reason };
        // Trust is remembered by key; a viewer without one cannot pair.
        let Some(key) = key else {
            let reply = reject("this viewer has no key; update sismo".into());
            self.send(from, stream::CONTROL, &reply).await;
            return;
        };
        let attempts = self.attempts.entry(from.ip()).or_insert(Attempts {
            failures: 0,
            locked_until: None,
        });
        if let Some(wait) = attempts.locked_for() {
            let reply = reject(format!(
                "too many wrong codes; try again in {} s",
                wait.as_secs().max(1)
            ));
            self.send(from, stream::CONTROL, &reply).await;
            return;
        }
        if code.trim() == self.code {
            self.attempts.remove(&from.ip());
            if let Some(viewer) = self.viewers.get_mut(&from) {
                viewer.authorized = true;
            }
            self.identity.trust(key);
            self.send(from, stream::CONTROL, &ToViewer::Paired).await;
        } else {
            attempts.fail();
            self.send(from, stream::CONTROL, &reject("wrong code".into()))
                .await;
        }
    }

    fn authorized(&self, from: SocketAddr) -> bool {
        self.viewers.get(&from).is_some_and(|v| v.authorized)
    }

    fn subscribed(&self) -> Vec<SocketAddr> {
        self.viewers
            .iter()
            .filter(|(_, v)| v.subscribed)
            .map(|(a, _)| *a)
            .collect()
    }

    /// Starts streaming to `from`. The definitions go first (seq 0, always
    /// safe to apply again), then the ring buffer past `since`, so a viewer
    /// reconnecting to the same session only gets what it missed.
    async fn subscribe(&mut self, from: SocketAddr, history: bool, since: u64) {
        if let Some(snapshot) = snapshot_chunk() {
            let message = ToViewer::Chunk {
                seq: 0,
                bytes: snapshot.into_bytes(),
            };
            self.send(from, stream::DATA, &message).await;
        }
        let ring = self
            .handle
            .ring()
            .map(|r| lock(r).history())
            .unwrap_or_default();
        let mut last_seq = ring.last().map_or(0, |(s, _)| *s).max(since);
        if history {
            for (seq, chunk) in ring.into_iter().filter(|(seq, _)| *seq > since) {
                last_seq = last_seq.max(seq);
                let message = ToViewer::Chunk {
                    seq,
                    bytes: chunk.as_bytes().to_vec(),
                };
                self.send(from, stream::DATA, &message).await;
            }
        }
        self.send(from, stream::DATA, &ToViewer::HistoryEnd).await;
        if let Some(viewer) = self.viewers.get_mut(&from) {
            viewer.subscribed = true;
            viewer.last_seq = last_seq;
        }
        self.send_crash_reports(from).await;
    }

    async fn send_crash_reports(&self, to: SocketAddr) {
        let reports = self.handle.crash_reports().iter().map(Into::into).collect();
        self.send(to, stream::CONTROL, &ToViewer::CrashReports { reports })
            .await;
    }

    async fn upload_crash_report(&self, to: SocketAddr, name: &str) {
        let Some(dir) = self.handle.dump_dir() else {
            return;
        };
        let Ok(bytes) = crate::crash::report_path(dir, name).and_then(std::fs::read) else {
            return;
        };
        let total = bytes.len() as u64;
        for (index, piece) in bytes.chunks(UPLOAD_PIECE).enumerate() {
            let message = ToViewer::CrashReportPiece {
                name: name.to_owned(),
                offset: (index * UPLOAD_PIECE) as u64,
                total,
                bytes: piece.to_vec(),
            };
            self.send(to, stream::BULK, &message).await;
        }
    }

    async fn forward(&mut self, seq: u64, chunk: &Chunk) {
        let targets: Vec<SocketAddr> = self
            .viewers
            .iter_mut()
            .filter(|(_, v)| v.subscribed && seq > v.last_seq)
            .map(|(a, v)| {
                v.last_seq = seq;
                *a
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let message = ToViewer::Chunk {
            seq,
            bytes: chunk.as_bytes().to_vec(),
        };
        for addr in targets {
            self.send(addr, stream::DATA, &message).await;
        }
    }

    fn start_device_recording(&mut self, owner: SocketAddr) {
        let mut state = lock(&self.device);
        if state.writer.is_some() {
            return;
        }
        let stem = registry()
            .session()
            .file_stem(sismo_format::FileStamp::now());
        let path = self.device_dir.join(format!("{stem}-device.sismo"));
        let opened = std::fs::create_dir_all(&self.device_dir)
            .and_then(|()| File::create(&path))
            .and_then(|f| FileWriter::new(BufWriter::new(f)));
        if let Ok(mut writer) = opened {
            if let Some(snapshot) = snapshot_chunk() {
                let _ = writer.write_chunk(&snapshot);
            }
            state.writer = Some(writer);
            state.path = Some(path);
            drop(state);
            self.device_owner = Some(owner);
        }
    }

    async fn upload_device_recording(&mut self, to: SocketAddr) {
        self.handle.flush();
        self.device_owner = None;
        let path = {
            let mut state = lock(&self.device);
            if let Some(mut writer) = state.writer.take() {
                let _ = writer.flush();
            }
            state.path.take()
        };
        let Some(path) = path else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let total = bytes.len() as u64;
        for (index, piece) in bytes.chunks(UPLOAD_PIECE).enumerate() {
            let message = ToViewer::DeviceRecording {
                offset: (index * UPLOAD_PIECE) as u64,
                total,
                bytes: piece.to_vec(),
            };
            self.send(to, stream::BULK, &message).await;
        }
        let _ = std::fs::remove_file(path);
    }

    /// Keeps viewers in step with their transport sessions: a viewer whose
    /// session timed out or was replaced (it restarted) is gone, and one that
    /// moved to another address keeps its state there.
    fn on_peer(&mut self, event: &PeerEvent) {
        match event {
            &PeerEvent::Disconnected { addr, .. } | &PeerEvent::Reset { addr, .. } => {
                self.forget(addr);
            }
            &PeerEvent::Migrated { from, to } => {
                if let Some(viewer) = self.viewers.remove(&from) {
                    self.viewers.insert(to, viewer);
                }
                if self.device_owner == Some(from) {
                    self.device_owner = Some(to);
                }
            }
            // Connected needs nothing: a viewer starts with its hello.
            _ => {}
        }
    }

    /// Drops a viewer; an on-device recording it started is discarded so a
    /// lost viewer never fills the device's storage.
    fn forget(&mut self, addr: SocketAddr) {
        self.viewers.remove(&addr);
        if self.device_owner == Some(addr) {
            self.device_owner = None;
            let path = {
                let mut state = lock(&self.device);
                state.writer = None;
                state.path.take()
            };
            if let Some(path) = path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn expire(&mut self) {
        let silent: Vec<SocketAddr> = self
            .viewers
            .iter()
            .filter(|(_, v)| v.last_seen.is_none_or(|t| t.elapsed() >= VIEWER_TIMEOUT))
            .map(|(a, _)| *a)
            .collect();
        for addr in silent {
            self.forget(addr);
        }
        self.attempts
            .retain(|_, a| a.locked_for().is_some() || a.failures % MAX_PAIR_ATTEMPTS != 0);
    }
}

fn pairing_code() -> String {
    format!("{:06}", random_u64() % 1_000_000)
}

/// Starts the network thread: advertises on the LAN and serves viewers.
pub(crate) fn start(config: &LiveConfig, handle: &Handle) -> io::Result<NetHandle> {
    let code = pairing_code();
    let data_dir = config
        .data_dir
        .clone()
        .or_else(crate::config::default_data_dir);
    let identity = Identity::load(data_dir.as_deref());
    let fingerprint = hex(&identity.keypair.public[..8]);
    let (feed_tx, mut feed_rx) = mpsc::channel::<(u64, Arc<Chunk>)>(512);
    let (flush_tx, mut flush_rx) = mpsc::unbounded_channel::<FlushRequest>();
    handle.add_sink(Box::new(NetSink { feed: feed_tx }));
    let device = Arc::new(Mutex::new(DeviceRecording::default()));
    handle.add_sink(Box::new(DeviceSink {
        state: Arc::clone(&device),
    }));

    let port = config.port;
    let auth = config.auth;
    let device_dir = data_dir.unwrap_or_else(std::env::temp_dir);
    let handle = handle.clone();
    let code_for_task = code.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<io::Result<()>>(1);
    std::thread::Builder::new()
        .name("sismo-net".into())
        .spawn(move || {
            runtime.block_on(async move {
                let listen = |port: u16| {
                    Zudp::default()
                        .port(port)
                        .sent_buffer(8192)
                        .security(Keypair {
                            public: identity.keypair.public,
                            private: identity.keypair.private,
                        })
                        .listen::<RawBytes>()
                };
                // The port may still be held by a session that just stopped;
                // viewers find any port through discovery.
                let socket = match listen(port).await {
                    Ok(socket) => Ok(socket),
                    Err(_) => listen(0).await,
                };
                let mut socket = match socket {
                    Ok(socket) => socket,
                    Err(error) => {
                        let _ = ready_tx.send(Err(io::Error::other(error.to_string())));
                        return;
                    }
                };
                let local_port = socket.local_addr().map_or(port, |a| a.port());
                let session = registry().session();
                let beacon = Beacon {
                    format_version: FORMAT_VERSION,
                    sismo_version: session.sismo_version.clone(),
                    session_id: session.session_id,
                    app: session.app.clone(),
                    app_version: session.app_version.clone(),
                    platform: session.platform.clone(),
                    device: session.device.clone(),
                    port: local_port,
                    auth,
                    fingerprint,
                };
                let advertise = Discovery::advertise(
                    DiscoveryConfig::new(DISCOVERY_APP_ID, local_port)
                        .meta(RawBytes(beacon.encode().into())),
                );
                let _advertise = match advertise {
                    Ok(handle) => handle,
                    Err(error) => {
                        let _ = ready_tx.send(Err(io::Error::other(error.to_string())));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                if auth == AuthMode::Pairing {
                    announce_code(&code_for_task);
                }
                let sender = socket.sender();
                let mut server = Server {
                    sender,
                    identity,
                    auth,
                    code: code_for_task,
                    viewers: HashMap::new(),
                    attempts: HashMap::new(),
                    handle,
                    device,
                    device_dir,
                    device_owner: None,
                };
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
                let mut peers = socket.events();
                loop {
                    tokio::select! {
                        packet = socket.recv() => {
                            let Ok(packet) = packet else { return };
                            if let Ok(message) = ToRecorder::decode(&packet.msg.0) {
                                server.on_message(&socket, packet.from, message).await;
                            }
                        }
                        feed = feed_rx.recv() => {
                            let Some((seq, chunk)) = feed else { return };
                            server.forward(seq, &chunk).await;
                        }
                        request = flush_rx.recv() => {
                            let Some(reply) = request else { return };
                            while let Ok((seq, chunk)) = feed_rx.try_recv() {
                                server.forward(seq, &chunk).await;
                            }
                            let _ = reply.send(());
                        }
                        event = peers.recv() => {
                            let Ok(event) = event else { return };
                            server.on_peer(&event);
                        }
                        _ = ticker.tick() => server.expire(),
                    }
                }
            });
        })?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| io::Error::other("network thread did not start"))??;
    Ok(NetHandle {
        pairing_code: code,
        flush: flush_tx,
    })
}

#[cfg_attr(not(feature = "log"), allow(unused_variables))]
fn announce_code(code: &str) {
    #[cfg(feature = "log")]
    ::log::info!(target: "sismo", "viewer pairing code: {code}");
}

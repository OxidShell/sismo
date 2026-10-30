use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sismo_format::net::{AuthMode, Beacon, DISCOVERY_APP_ID, ToRecorder, ToViewer, stream};
use sismo_format::{Chunk, FORMAT_VERSION, FileWriter};
use tokio::sync::mpsc;
use zudp::{Discovery, DiscoveryConfig};
use zudp::{Keypair, RawBytes, Zudp, ZudpSender, ZudpSocket};

use crate::collector::snapshot_chunk;
use crate::config::LiveConfig;
use crate::handle::Handle;
use crate::registry::registry;
use crate::sink::{Sink, lock};

/// Viewers that stay silent this long are dropped.
const VIEWER_TIMEOUT: Duration = Duration::from_secs(15);
/// Wrong pairing codes tolerated per viewer address.
const MAX_PAIR_ATTEMPTS: u32 = 5;
/// Piece size for on-device recording uploads.
const UPLOAD_PIECE: usize = 192 * 1024;

pub(crate) struct NetHandle {
    pairing_code: String,
}

impl NetHandle {
    pub fn pairing_code(&self) -> &str {
        &self.pairing_code
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
    hasher.write_u64(crate::time::unix_ns());
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
            identity.trusted = bytes
                .chunks_exact(32)
                .filter_map(|c| <[u8; 32]>::try_from(c).ok())
                .collect();
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
    attempts: u32,
    last_seen: Option<Instant>,
}

struct Server {
    sender: ZudpSender,
    identity: Identity,
    auth: AuthMode,
    code: String,
    viewers: HashMap<SocketAddr, Viewer>,
    handle: Handle,
    device: Arc<Mutex<DeviceRecording>>,
    device_dir: PathBuf,
}

impl Server {
    async fn send(&self, to: SocketAddr, stream_id: u16, message: &ToViewer) {
        let bytes = RawBytes(message.encode().into());
        let _ = self.sender.send_stream(bytes, to, stream_id).await;
    }

    fn known_key(&self, socket: &ZudpSocket<RawBytes>, from: SocketAddr) -> bool {
        socket
            .peer_remote_static_key(from)
            .is_some_and(|key| self.identity.trusted.contains(&key))
    }

    async fn on_message(
        &mut self,
        socket: &ZudpSocket<RawBytes>,
        from: SocketAddr,
        message: ToRecorder,
    ) {
        let viewer = self.viewers.entry(from).or_default();
        viewer.last_seen = Some(Instant::now());
        match message {
            ToRecorder::Hello { .. } => {
                let open = self.auth == AuthMode::Open;
                let trusted = self.known_key(socket, from);
                let viewer = self.viewers.entry(from).or_default();
                viewer.authorized = open || trusted;
                let pairing_required = !viewer.authorized;
                let history_bytes = self.handle.flight_recorder_bytes() as u64;
                let welcome = ToViewer::Welcome {
                    session: registry().session(),
                    pairing_required,
                    filter: registry().filter().to_string(),
                    history_bytes,
                };
                self.send(from, stream::CONTROL, &welcome).await;
            }
            ToRecorder::Pair { code } => {
                let viewer = self.viewers.entry(from).or_default();
                if viewer.attempts >= MAX_PAIR_ATTEMPTS {
                    let reply = ToViewer::PairRejected {
                        reason: "too many attempts; restart the app to pair again".into(),
                    };
                    self.send(from, stream::CONTROL, &reply).await;
                    return;
                }
                if code.trim() == self.code {
                    viewer.authorized = true;
                    if let Some(key) = socket.peer_remote_static_key(from) {
                        self.identity.trust(key);
                    }
                    self.send(from, stream::CONTROL, &ToViewer::Paired).await;
                } else {
                    viewer.attempts += 1;
                    let reply = ToViewer::PairRejected {
                        reason: "wrong code".into(),
                    };
                    self.send(from, stream::CONTROL, &reply).await;
                }
            }
            ToRecorder::Subscribe { history } => {
                if !self.authorized(from) {
                    return;
                }
                self.subscribe(from, history).await;
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
                    self.start_device_recording();
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
            ToRecorder::Bye => {
                self.viewers.remove(&from);
            }
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

    async fn subscribe(&mut self, from: SocketAddr, history: bool) {
        if let Some(snapshot) = snapshot_chunk() {
            self.send(
                from,
                stream::DATA,
                &ToViewer::Chunk {
                    bytes: snapshot.into_bytes(),
                },
            )
            .await;
        }
        let chunks = if history {
            self.handle
                .ring()
                .map(|r| lock(r).history())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut last_seq = self
            .handle
            .ring()
            .and_then(|r| lock(r).history().last().map(|(s, _)| *s))
            .unwrap_or(0);
        for (seq, chunk) in chunks {
            last_seq = last_seq.max(seq);
            let message = ToViewer::Chunk {
                bytes: chunk.as_bytes().to_vec(),
            };
            self.send(from, stream::DATA, &message).await;
        }
        self.send(from, stream::DATA, &ToViewer::HistoryEnd).await;
        if let Some(viewer) = self.viewers.get_mut(&from) {
            viewer.subscribed = true;
            viewer.last_seq = last_seq;
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
            bytes: chunk.as_bytes().to_vec(),
        };
        for addr in targets {
            self.send(addr, stream::DATA, &message).await;
        }
    }

    fn start_device_recording(&self) {
        let mut state = lock(&self.device);
        if state.writer.is_some() {
            return;
        }
        let path = self.device_dir.join(format!(
            "{}-device-{}.sismo",
            registry().session().app,
            crate::time::FileStamp::now()
        ));
        let opened = std::fs::create_dir_all(&self.device_dir)
            .and_then(|()| File::create(&path))
            .and_then(|f| FileWriter::new(BufWriter::new(f)));
        if let Ok(mut writer) = opened {
            if let Some(snapshot) = snapshot_chunk() {
                let _ = writer.write_chunk(&snapshot);
            }
            state.writer = Some(writer);
            state.path = Some(path);
        }
    }

    async fn upload_device_recording(&self, to: SocketAddr) {
        self.handle.flush();
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

    fn expire(&mut self) {
        self.viewers
            .retain(|_, v| v.last_seen.is_some_and(|t| t.elapsed() < VIEWER_TIMEOUT));
    }
}

fn pairing_code() -> String {
    format!("{:06}", random_u64() % 1_000_000)
}

/// Starts the network thread: advertises on the LAN and serves viewers.
pub(crate) fn start(config: &LiveConfig, handle: &Handle) -> io::Result<NetHandle> {
    let code = pairing_code();
    let identity = Identity::load(config.data_dir.as_deref());
    let fingerprint = hex(&identity.keypair.public[..8]);
    let (feed_tx, mut feed_rx) = mpsc::channel::<(u64, Arc<Chunk>)>(512);
    handle.add_sink(Box::new(NetSink { feed: feed_tx }));
    let device = Arc::new(Mutex::new(DeviceRecording::default()));
    handle.add_sink(Box::new(DeviceSink {
        state: Arc::clone(&device),
    }));

    let port = config.port;
    let auth = config.auth;
    let device_dir = config.data_dir.clone().unwrap_or_else(std::env::temp_dir);
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
                let socket = Zudp::default()
                    .port(port)
                    .sent_buffer(8192)
                    .security(Keypair {
                        public: identity.keypair.public,
                        private: identity.keypair.private,
                    })
                    .listen::<RawBytes>()
                    .await;
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
                    handle,
                    device,
                    device_dir,
                };
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
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
                        _ = ticker.tick() => server.expire(),
                    }
                }
            });
        })?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| io::Error::other("network thread did not start"))??;
    Ok(NetHandle { pairing_code: code })
}

#[cfg_attr(not(feature = "log"), allow(unused_variables))]
fn announce_code(code: &str) {
    #[cfg(feature = "log")]
    ::log::info!(target: "sismo", "viewer pairing code: {code}");
}

//! Viewer side of the LAN protocol: discovery and one connection per app.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sismo_format::SessionInfo;
use sismo_format::net::{Beacon, DISCOVERY_APP_ID, ToRecorder, ToViewer, stream};
use tokio::sync::mpsc;
use zudp::{Discovery, DiscoveryConfig, Keypair, RawBytes, Zudp};

/// Silence after which a connection counts as lost.
const LOST_AFTER: Duration = Duration::from_secs(6);
const PING_EVERY: Duration = Duration::from_secs(2);

/// An app announcing itself on the LAN.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub addr: SocketAddr,
    pub beacon: Beacon,
    pub seen: Instant,
}

/// Something that happened on a connection, in arrival order.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Welcome {
        session: SessionInfo,
        pairing_required: bool,
        filter: String,
        history_bytes: u64,
    },
    Paired,
    PairRejected(String),
    Chunk(Vec<u8>),
    HistoryEnd,
    Filter(String),
    DeviceRecording {
        offset: u64,
        total: u64,
        bytes: Vec<u8>,
    },
    Rtt(Duration),
    Lost(String),
}

/// Requests a viewer can send to a connected app.
#[derive(Debug, Clone)]
pub enum Command {
    Pair(String),
    Subscribe { history: bool },
    SetFilter(String),
    Marker(String),
    StartDeviceRecording,
    StopDeviceRecording,
    Close,
}

/// The viewer's persistent key, so apps remember a paired viewer.
fn viewer_keypair() -> Keypair {
    let path = config_dir().map(|d| d.join("viewer-key"));
    if let Some(path) = &path
        && let Ok(bytes) = std::fs::read(path)
        && bytes.len() == 64
    {
        let mut keypair = Keypair::generate();
        keypair.public.copy_from_slice(&bytes[..32]);
        keypair.private.copy_from_slice(&bytes[32..]);
        return keypair;
    }
    let keypair = Keypair::generate();
    if let Some(path) = path {
        let mut bytes = keypair.public.to_vec();
        bytes.extend_from_slice(&keypair.private);
        let _ = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, bytes));
    }
    keypair
}

/// `$XDG_CONFIG_HOME/sismo`, `~/.config/sismo` or `%APPDATA%\sismo`.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("sismo"))
}

/// Background LAN scanner; new apps arrive on [`Scanner::events`].
pub struct Scanner {
    pub events: flume::Receiver<Result<Discovered, String>>,
    _thread: std::thread::JoinHandle<()>,
}

impl Scanner {
    /// # Errors
    /// I/O errors creating the runtime or thread.
    pub fn start(probe_every: Duration) -> std::io::Result<Self> {
        let (tx, rx) = flume::unbounded();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("sismo-scan".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let config =
                        DiscoveryConfig::new(DISCOVERY_APP_ID, 0).probe_interval(probe_every);
                    let mut scan = match Discovery::scan_stream::<RawBytes>(config).await {
                        Ok(scan) => scan,
                        Err(error) => {
                            let _ = tx.send(Err(error.to_string()));
                            return;
                        }
                    };
                    loop {
                        match scan.next().await {
                            Ok(peer) => {
                                let Ok(beacon) = Beacon::decode(&peer.meta.0) else {
                                    continue;
                                };
                                let found = Discovered {
                                    addr: SocketAddr::new(peer.data_addr.ip(), beacon.port),
                                    beacon,
                                    seen: Instant::now(),
                                };
                                if tx.send(Ok(found)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                if tx.send(Err(error.to_string())).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            })?;
        Ok(Self {
            events: rx,
            _thread: thread,
        })
    }
}

/// Live connection to one app.
pub struct Connection {
    pub events: flume::Receiver<ClientEvent>,
    commands: mpsc::UnboundedSender<Command>,
}

impl Connection {
    /// Connects in the background; failures arrive as [`ClientEvent::Lost`].
    ///
    /// # Errors
    /// I/O errors creating the runtime or thread.
    pub fn open(addr: SocketAddr, viewer_name: String) -> std::io::Result<Self> {
        let (event_tx, event_rx) = flume::unbounded();
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name(format!("sismo-conn-{addr}"))
            .spawn(move || {
                runtime.block_on(async move {
                    let lost = |reason: String| {
                        let _ = event_tx.send(ClientEvent::Lost(reason));
                    };
                    let connect = Zudp::default()
                        .port(0)
                        .security(viewer_keypair())
                        .connect::<RawBytes>(addr);
                    let mut conn = match tokio::time::timeout(Duration::from_secs(5), connect).await {
                        Ok(Ok(conn)) => conn,
                        Ok(Err(error)) => return lost(error.to_string()),
                        Err(_) => return lost("no answer from the app".into()),
                    };
                    let wire = |msg: ToRecorder| RawBytes(msg.encode().into());
                    if conn
                        .send_stream(wire(ToRecorder::Hello { viewer: viewer_name }), stream::CONTROL)
                        .await
                        .is_err()
                    {
                        return lost("could not reach the app".into());
                    }
                    let mut ping = tokio::time::interval(PING_EVERY);
                    let mut last_heard = Instant::now();
                    let mut pings: Vec<(u64, Instant)> = Vec::new();
                    let mut nonce = 0u64;
                    loop {
                        tokio::select! {
                            packet = conn.recv() => {
                                let Ok(packet) = packet else { return lost("connection closed".into()) };
                                last_heard = Instant::now();
                                let Ok(message) = ToViewer::decode(&packet.msg.0) else { continue };
                                let event = match message {
                                    ToViewer::Welcome { session, pairing_required, filter, history_bytes } =>
                                        ClientEvent::Welcome { session, pairing_required, filter, history_bytes },
                                    ToViewer::Paired => ClientEvent::Paired,
                                    ToViewer::PairRejected { reason } => ClientEvent::PairRejected(reason),
                                    ToViewer::Chunk { bytes } => ClientEvent::Chunk(bytes),
                                    ToViewer::HistoryEnd => ClientEvent::HistoryEnd,
                                    ToViewer::Filter { filter } => ClientEvent::Filter(filter),
                                    ToViewer::DeviceRecording { offset, total, bytes } =>
                                        ClientEvent::DeviceRecording { offset, total, bytes },
                                    ToViewer::Pong { nonce } => {
                                        let Some(pos) = pings.iter().position(|(n, _)| *n == nonce) else { continue };
                                        let (_, sent) = pings.remove(pos);
                                        ClientEvent::Rtt(sent.elapsed())
                                    }
                                };
                                if event_tx.send(event).is_err() {
                                    return;
                                }
                            }
                            command = command_rx.recv() => {
                                let message = match command {
                                    None | Some(Command::Close) => {
                                        let _ = conn.send_stream(wire(ToRecorder::Bye), stream::CONTROL).await;
                                        return;
                                    }
                                    Some(Command::Pair(code)) => ToRecorder::Pair { code },
                                    Some(Command::Subscribe { history }) => ToRecorder::Subscribe { history },
                                    Some(Command::SetFilter(filter)) => ToRecorder::SetFilter { filter },
                                    Some(Command::Marker(note)) => ToRecorder::Marker { note },
                                    Some(Command::StartDeviceRecording) => ToRecorder::StartDeviceRecording,
                                    Some(Command::StopDeviceRecording) => ToRecorder::StopDeviceRecording,
                                };
                                if conn.send_stream(wire(message), stream::CONTROL).await.is_err() {
                                    return lost("could not reach the app".into());
                                }
                            }
                            _ = ping.tick() => {
                                if last_heard.elapsed() > LOST_AFTER {
                                    return lost("the app stopped answering".into());
                                }
                                nonce += 1;
                                pings.retain(|(_, t)| t.elapsed() < LOST_AFTER);
                                pings.push((nonce, Instant::now()));
                                let _ = conn.send_stream(wire(ToRecorder::Ping { nonce }), stream::CONTROL).await;
                            }
                        }
                    }
                });
            })?;
        Ok(Self {
            events: event_rx,
            commands: command_tx,
        })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

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
/// Probes per scan stream before it is recreated (see [`Scanner::start`]).
const RESCAN_PROBES: u32 = 4;

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
    Chunk {
        seq: u64,
        bytes: Vec<u8>,
    },
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
    /// `since`: last chunk seq already received from this session, `0` for none.
    Subscribe {
        history: bool,
        since: u64,
    },
    SetFilter(String),
    Marker(String),
    StartDeviceRecording,
    StopDeviceRecording,
    Close,
}

/// The viewer's persistent key, so apps remember a paired viewer.
fn viewer_keypair() -> Keypair {
    let path = viewer_key_path();
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() == 64
    {
        let mut keypair = Keypair::generate();
        keypair.public.copy_from_slice(&bytes[..32]);
        keypair.private.copy_from_slice(&bytes[32..]);
        return keypair;
    }
    let keypair = Keypair::generate();
    let mut bytes = keypair.public.to_vec();
    bytes.extend_from_slice(&keypair.private);
    let _ = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, bytes));
    keypair
}

/// Where the viewer keeps its static key (istmo config dir).
#[must_use]
pub fn viewer_key_path() -> PathBuf {
    istmo::path::config_dir().join("viewer-key")
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
                    // A scan stream reports each address once, so a restarted
                    // app (same address, new session) would never show up
                    // again; a fresh stream every few probes reports
                    // everyone still announcing and keeps `seen` current.
                    let rescan = probe_every.saturating_mul(RESCAN_PROBES);
                    loop {
                        let config =
                            DiscoveryConfig::new(DISCOVERY_APP_ID, 0).probe_interval(probe_every);
                        let mut scan = match Discovery::scan_stream::<RawBytes>(config).await {
                            Ok(scan) => scan,
                            Err(error) => {
                                if tx.send(Err(error.to_string())).is_err() {
                                    return;
                                }
                                tokio::time::sleep(rescan).await;
                                continue;
                            }
                        };
                        let until = tokio::time::Instant::now() + rescan;
                        while let Ok(next) = tokio::time::timeout_at(until, scan.next()).await {
                            let event = match next {
                                Ok(peer) => {
                                    let Ok(beacon) = Beacon::decode(&peer.meta.0) else {
                                        continue;
                                    };
                                    Ok(Discovered {
                                        addr: SocketAddr::new(peer.data_addr.ip(), beacon.port),
                                        beacon,
                                        seen: Instant::now(),
                                    })
                                }
                                // One bad packet must not stop discovery.
                                Err(error) => Err(error.to_string()),
                            };
                            if tx.send(event).is_err() {
                                return;
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
                                    ToViewer::Chunk { seq, bytes } => ClientEvent::Chunk { seq, bytes },
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
                                    Some(Command::Subscribe { history, since }) => ToRecorder::Subscribe { history, since },
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
                                    // Tell the app, in case only its replies are lost.
                                    let _ = conn.send_stream(wire(ToRecorder::Bye), stream::CONTROL).await;
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

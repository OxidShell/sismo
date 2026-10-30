use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use sismo_format::{Filter, FilterError, SessionInfo};

use crate::collector::{Collector, Message};
use crate::config::Config;
use crate::registry::{ACTIVE, registry};
use crate::sink::{RingBuffer, RotatingFile, lock};
use crate::thread::{self, set_sender};
use crate::time::{FileStamp, unix_ns};

/// Reasons [`install`] can fail.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("sismo is already installed in this process")]
    AlreadyInstalled,
    #[error("no sink enabled: call live(), record_to() or flight_recorder()")]
    NoSink,
    #[error("invalid filter: {0}")]
    Filter(#[from] FilterError),
    #[error("could not start the collector thread: {0}")]
    Spawn(#[source] io::Error),
    #[cfg(feature = "net")]
    #[error("could not start network streaming: {0}")]
    Net(#[source] io::Error),
    #[cfg(all(feature = "stdio", unix))]
    #[error("could not capture stdout/stderr: {0}")]
    Stdio(#[source] io::Error),
}

pub(crate) struct Shared {
    pub sender: SyncSender<Message>,
    pub ring: Option<Arc<Mutex<RingBuffer>>>,
    pub dump_dir: Option<PathBuf>,
    pub app: String,
    #[cfg(feature = "net")]
    pub net: OnceLock<crate::net::NetHandle>,
}

static HANDLE: RwLock<Option<Handle>> = RwLock::new(None);
/// Serializes [`install`] and [`Handle::shutdown`].
static LIFECYCLE: Mutex<()> = Mutex::new(());

/// Control surface of the installed recorder. Cheap to clone.
#[derive(Clone)]
pub struct Handle {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("app", &self.shared.app)
            .finish_non_exhaustive()
    }
}

/// The installed recorder, if any.
#[must_use]
pub fn handle() -> Option<Handle> {
    HANDLE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn session_id(seed: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(seed);
    hasher.write_u32(std::process::id());
    hasher.finish()
}

/// Installs the global recorder and starts its collector thread. After
/// [`Handle::shutdown`] it can be installed again, as a new session.
///
/// # Errors
/// [`InstallError`] when already installed, misconfigured or a thread or
/// socket cannot start.
pub fn install(config: Config) -> Result<Handle, InstallError> {
    let _lifecycle = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
    if handle().is_some() {
        return Err(InstallError::AlreadyInstalled);
    }
    let handle = start(config)?;
    *HANDLE.write().unwrap_or_else(PoisonError::into_inner) = Some(handle.clone());
    Ok(handle)
}

fn start(config: Config) -> Result<Handle, InstallError> {
    if !config.any_sink() {
        return Err(InstallError::NoSink);
    }
    let filter = Filter::parse(&config.filter)?;
    let registry = registry();
    registry.set_filter(filter);
    #[cfg(feature = "log")]
    crate::log::refresh_max_level();
    let started = unix_ns();
    registry.set_session(SessionInfo {
        session_id: session_id(started),
        app: config.app.clone(),
        app_version: config.app_version,
        build: config.build,
        platform: crate::device::platform(),
        device: config.device.unwrap_or_else(crate::device::device),
        pid: std::process::id(),
        started_unix_ns: started.saturating_sub(crate::registry::now()),
        sismo_version: env!("CARGO_PKG_VERSION").to_owned(),
    });

    let (sender, receiver) = mpsc::sync_channel(config.queue_batches.max(16));
    let ring = config
        .flight
        .as_ref()
        .map(|f| Arc::new(Mutex::new(RingBuffer::new(f.capacity_bytes))));
    let collector = Collector::new(receiver, ring.clone());
    std::thread::Builder::new()
        .name("sismo-collector".into())
        .spawn(move || collector.run())
        .map_err(InstallError::Spawn)?;

    if let Some(record) = &config.record {
        let sink = RotatingFile::new(
            record.dir.clone(),
            config.app.clone(),
            record.max_file_bytes,
            record.keep_files,
        );
        let _ = sender.send(Message::AddSink(Box::new(sink)));
    }

    let handle = Handle {
        shared: Arc::new(Shared {
            sender,
            ring,
            dump_dir: config.flight.as_ref().and_then(|f| f.dump_dir.clone()),
            app: config.app.clone(),
            #[cfg(feature = "net")]
            net: OnceLock::new(),
        }),
    };
    #[cfg(feature = "net")]
    if let Some(live) = &config.live {
        match crate::net::start(live, &handle) {
            Ok(net) => {
                let _ = handle.shared.net.set(net);
            }
            Err(error) => {
                handle.stop_collector();
                return Err(InstallError::Net(error));
            }
        }
    }
    set_sender(Some(handle.shared.sender.clone()));
    ACTIVE.store(true, Ordering::Release);

    if config.capture_panics {
        crate::panic::install_hook();
    }
    #[cfg(all(feature = "stdio", unix))]
    if config.capture_stdio {
        crate::stdio::capture().map_err(InstallError::Stdio)?;
    }
    Ok(handle)
}

impl Handle {
    /// Pushes every buffered event of the calling thread and waits (up to
    /// two seconds) until the collector has written them out.
    pub fn flush(&self) {
        self.flush_timeout(Duration::from_secs(2));
    }

    pub(crate) fn flush_timeout(&self, timeout: Duration) -> bool {
        thread::flush_current();
        let (reply, done) = mpsc::sync_channel(1);
        if self.shared.sender.send(Message::Flush(reply)).is_err() {
            return false;
        }
        done.recv_timeout(timeout).is_ok()
    }

    /// Writes the flight recorder history to `path`.
    ///
    /// # Errors
    /// I/O errors, or an error when no flight recorder is configured.
    pub fn dump(&self, path: impl AsRef<Path>) -> io::Result<u64> {
        thread::flush_current();
        let (reply, done) = mpsc::sync_channel(1);
        self.shared
            .sender
            .send(Message::Dump {
                path: path.as_ref().to_path_buf(),
                reply,
            })
            .map_err(|_| io::Error::other("collector stopped"))?;
        done.recv_timeout(Duration::from_secs(10))
            .map_err(|_| io::Error::other("collector did not answer"))?
    }

    /// Dumps the flight recorder into the configured dump directory and
    /// returns the new file's path.
    ///
    /// # Errors
    /// I/O errors, or an error when no dump directory is configured.
    pub fn dump_flight_recorder(&self, label: &str) -> io::Result<PathBuf> {
        let dir = self
            .shared
            .dump_dir
            .clone()
            .ok_or_else(|| io::Error::other("no dump directory configured"))?;
        let path = dir.join(format!(
            "{}-{label}-{}.sismo",
            self.shared.app,
            FileStamp::now()
        ));
        self.dump(&path)?;
        Ok(path)
    }

    /// Bytes currently held by the flight recorder.
    #[must_use]
    pub fn flight_recorder_bytes(&self) -> usize {
        self.shared.ring.as_ref().map_or(0, |r| lock(r).bytes())
    }

    /// Replaces the filter at runtime.
    ///
    /// # Errors
    /// [`FilterError`] when `spec` does not parse.
    pub fn set_filter(&self, spec: &str) -> Result<(), FilterError> {
        let filter = Filter::parse(spec)?;
        registry().set_filter(filter);
        #[cfg(feature = "log")]
        crate::log::refresh_max_level();
        Ok(())
    }

    #[must_use]
    pub fn filter(&self) -> String {
        registry().filter().to_string()
    }

    #[must_use]
    pub fn session(&self) -> SessionInfo {
        registry().session()
    }

    /// Six-digit code a viewer must enter to pair, when streaming live.
    #[cfg(feature = "net")]
    #[must_use]
    pub fn pairing_code(&self) -> Option<String> {
        self.shared.net.get().map(|n| n.pairing_code().to_owned())
    }

    /// Number of events lost because the collector could not keep up.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        thread::DROPPED.load(Ordering::Relaxed)
    }

    /// Stops recording, flushes every sink and stops streaming. The
    /// recorder can be installed again afterwards.
    pub fn shutdown(&self) {
        let _lifecycle = LIFECYCLE.lock().unwrap_or_else(PoisonError::into_inner);
        thread::flush_current();
        ACTIVE.store(false, Ordering::Release);
        set_sender(None);
        self.stop_collector();
        let mut installed = HANDLE.write().unwrap_or_else(PoisonError::into_inner);
        if installed
            .as_ref()
            .is_some_and(|h| Arc::ptr_eq(&h.shared, &self.shared))
        {
            *installed = None;
        }
    }

    fn stop_collector(&self) {
        let (reply, done) = mpsc::sync_channel(1);
        if self.shared.sender.send(Message::Shutdown(reply)).is_ok() {
            let _ = done.recv_timeout(Duration::from_secs(5));
        }
    }

    pub(crate) fn ring(&self) -> Option<&Arc<Mutex<RingBuffer>>> {
        self.shared.ring.as_ref()
    }

    #[cfg(feature = "net")]
    pub(crate) fn add_sink(&self, sink: Box<dyn crate::sink::Sink>) {
        let _ = self.shared.sender.send(Message::AddSink(sink));
    }
}

use std::path::PathBuf;

/// Where and how much to record to disk.
#[derive(Debug, Clone)]
pub struct RecordConfig {
    pub dir: PathBuf,
    /// A new file starts once the current one would exceed this size.
    pub max_file_bytes: u64,
    /// Older files beyond this count are deleted.
    pub keep_files: usize,
}

/// In-memory history kept for late viewers and crash dumps.
#[derive(Debug, Clone)]
pub struct FlightConfig {
    pub capacity_bytes: usize,
    /// Where crash reports and [`crate::Handle::dump_flight_recorder`] write;
    /// with the `istmo` feature it defaults to the app's state directory.
    pub dump_dir: Option<PathBuf>,
}

/// Network streaming to viewers on the LAN.
#[cfg(feature = "net")]
#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub port: u16,
    pub auth: sismo_format::net::AuthMode,
    /// Directory for the persistent key and trusted viewers; with the
    /// `istmo` feature it defaults to the app's config directory, without
    /// it keys are ephemeral when unset.
    pub data_dir: Option<PathBuf>,
}

#[cfg(feature = "net")]
impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            port: sismo_format::net::DEFAULT_PORT,
            auth: sismo_format::net::AuthMode::Pairing,
            data_dir: None,
        }
    }
}

/// Recorder configuration. Build one with [`crate::config!`] so the app
/// name and version come from the calling crate.
#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) app: String,
    pub(crate) app_version: String,
    pub(crate) build: String,
    pub(crate) device: Option<String>,
    pub(crate) filter: String,
    pub(crate) record: Option<RecordConfig>,
    pub(crate) flight: Option<FlightConfig>,
    #[cfg(feature = "net")]
    pub(crate) live: Option<LiveConfig>,
    pub(crate) capture_panics: bool,
    pub(crate) capture_stdio: bool,
    pub(crate) queue_batches: usize,
    pub(crate) app_crates: Vec<String>,
}

/// Default flight recorder size.
pub const DEFAULT_FLIGHT_BYTES: usize = 32 * 1024 * 1024;

impl Config {
    /// Starts a configuration for `app` at `version`; nothing is recorded
    /// until a sink (`live`, `record_to`, `flight_recorder`) is enabled.
    #[must_use]
    pub fn new(app: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            app_version: version.into(),
            build: String::new(),
            device: None,
            filter: "info".to_owned(),
            record: None,
            flight: None,
            #[cfg(feature = "net")]
            live: None,
            capture_panics: true,
            capture_stdio: false,
            queue_batches: 4096,
            app_crates: Vec::new(),
        }
    }

    /// Free-form build identifier, e.g. a git hash.
    #[must_use]
    pub fn build(mut self, build: impl Into<String>) -> Self {
        self.build = build.into();
        self
    }

    /// Crates that belong to the app besides its own package, as exact
    /// names or `prefix*` patterns (e.g. workspace members). Viewers count
    /// them as app time instead of framework or other.
    #[must_use]
    pub fn app_crates<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.app_crates
            .extend(patterns.into_iter().map(|p| p.into().replace('-', "_")));
        self
    }

    /// Overrides the detected device name.
    #[must_use]
    pub fn device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    /// `RUST_LOG`-style filter for logs; spans are kept unless their target
    /// is `off`. Parsed by [`crate::install`].
    #[must_use]
    pub fn filter(mut self, spec: impl Into<String>) -> Self {
        self.filter = spec.into();
        self
    }

    /// Records to rotating `.sismo` files in `dir` (64 MB each, 8 kept).
    #[must_use]
    pub fn record_to(mut self, dir: impl Into<PathBuf>) -> Self {
        self.record = Some(RecordConfig {
            dir: dir.into(),
            max_file_bytes: 64 * 1024 * 1024,
            keep_files: 8,
        });
        self
    }

    #[must_use]
    pub fn record(mut self, record: RecordConfig) -> Self {
        self.record = Some(record);
        self
    }

    /// Keeps the last `capacity_bytes` of data in memory; dumped on panic
    /// when a dump directory is set.
    #[must_use]
    pub fn flight_recorder(mut self, capacity_bytes: usize) -> Self {
        let dump_dir = self.flight.take().and_then(|f| f.dump_dir);
        self.flight = Some(FlightConfig {
            capacity_bytes,
            dump_dir,
        });
        self
    }

    /// Directory for flight recorder dumps (implies a flight recorder).
    #[must_use]
    pub fn dump_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        let capacity_bytes = self
            .flight
            .as_ref()
            .map_or(DEFAULT_FLIGHT_BYTES, |f| f.capacity_bytes);
        self.flight = Some(FlightConfig {
            capacity_bytes,
            dump_dir: Some(dir.into()),
        });
        self
    }

    /// Streams to viewers on the LAN (implies a flight recorder for history).
    #[cfg(feature = "net")]
    #[must_use]
    pub fn live(mut self, live: LiveConfig) -> Self {
        self.live = Some(live);
        if self.flight.is_none() {
            self = self.flight_recorder(DEFAULT_FLIGHT_BYTES);
        }
        self
    }

    /// Captures stdout and stderr lines as logs (Unix only, `stdio` feature).
    #[must_use]
    pub fn capture_stdio(mut self, enabled: bool) -> Self {
        self.capture_stdio = enabled;
        self
    }

    #[must_use]
    pub fn capture_panics(mut self, enabled: bool) -> Self {
        self.capture_panics = enabled;
        self
    }

    /// Applies `SISMO` and `SISMO_FILTER` from the environment.
    ///
    /// `SISMO` is a comma list of `live`, `record=<dir>`, `flight[=<MB>]`,
    /// `dump=<dir>`, `stdio`.
    #[must_use]
    pub fn from_env(mut self) -> Self {
        if let Ok(filter) = std::env::var("SISMO_FILTER") {
            self.filter = filter;
        }
        let Ok(spec) = std::env::var("SISMO") else {
            return self;
        };
        for part in spec.split(',').map(str::trim) {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            self = match key {
                #[cfg(feature = "net")]
                "live" => self.live(LiveConfig::default()),
                "record" if !value.is_empty() => self.record_to(value),
                "flight" => {
                    let mb = value.parse::<usize>().unwrap_or(32);
                    self.flight_recorder(mb * 1024 * 1024)
                }
                "dump" if !value.is_empty() => self.dump_dir(value),
                "stdio" => self.capture_stdio(true),
                _ => self,
            };
        }
        self
    }

    pub(crate) fn any_sink(&self) -> bool {
        #[cfg(feature = "net")]
        let live = self.live.is_some();
        #[cfg(not(feature = "net"))]
        let live = false;
        live || self.record.is_some() || self.flight.is_some()
    }
}

/// Default dump directory when none is set: the app's state directory
/// for this OS (`istmo` feature), or none.
#[cfg_attr(
    feature = "istmo",
    expect(clippy::unnecessary_wraps, reason = "none without the istmo feature")
)]
pub(crate) fn default_dump_dir() -> Option<PathBuf> {
    #[cfg(feature = "istmo")]
    return Some(istmo_core::path::state_dir().join("sismo"));
    #[cfg(not(feature = "istmo"))]
    None
}

/// Default directory for the recorder's key and trusted viewers: the app's
/// config directory for this OS (`istmo` feature), or none.
#[cfg(feature = "net")]
#[cfg_attr(
    feature = "istmo",
    expect(clippy::unnecessary_wraps, reason = "none without the istmo feature")
)]
pub(crate) fn default_data_dir() -> Option<PathBuf> {
    #[cfg(feature = "istmo")]
    return Some(istmo_core::path::config_dir().join("sismo"));
    #[cfg(not(feature = "istmo"))]
    None
}

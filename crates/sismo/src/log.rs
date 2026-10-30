//! Bridge from the `log` facade.
//!
//! [`init`] installs a global logger that records into sismo and forwards
//! to the app's existing logger, so `env_logger` or `android_logger` keep
//! printing as before.

use std::sync::OnceLock;

use ::log::{LevelFilter, Log, Metadata, Record};
use sismo_format::{Level, Source};

use crate::registry::{DynamicKey, registry};
use crate::thread;

static INNER_MAX: OnceLock<LevelFilter> = OnceLock::new();

/// `log::Log` implementation recording into sismo.
pub struct Logger {
    inner: Option<Box<dyn Log>>,
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logger")
            .field("forwards", &self.inner.is_some())
            .finish()
    }
}

fn level(level: ::log::Level) -> Level {
    match level {
        ::log::Level::Error => Level::Error,
        ::log::Level::Warn => Level::Warn,
        ::log::Level::Info => Level::Info,
        ::log::Level::Debug => Level::Debug,
        ::log::Level::Trace => Level::Trace,
    }
}

fn ours(metadata: &Metadata<'_>) -> bool {
    crate::is_active() && registry().allows(metadata.target(), level(metadata.level()))
}

struct Fields(Vec<(String, String)>);

impl<'kvs> ::log::kv::VisitSource<'kvs> for Fields {
    fn visit_pair(
        &mut self,
        key: ::log::kv::Key<'kvs>,
        value: ::log::kv::Value<'kvs>,
    ) -> Result<(), ::log::kv::Error> {
        self.0.push((key.to_string(), value.to_string()));
        Ok(())
    }
}

impl Logger {
    /// Records into sismo and forwards to `inner` when it accepts the record.
    #[must_use]
    pub fn new(inner: Option<Box<dyn Log>>) -> Self {
        Self { inner }
    }

    fn record(record: &Record<'_>) {
        let target = record.target();
        let key = DynamicKey {
            kind: 1,
            name: target.to_owned(),
            target: target.to_owned(),
            file: record.file().unwrap_or_default().to_owned(),
            line: record.line().unwrap_or_default(),
        };
        let lvl = level(record.level());
        let callsite =
            registry().register_dynamic(key, record.module_path().unwrap_or(target), lvl);
        let message = record.args().to_string();
        let mut fields = Fields(Vec::new());
        let _ = record.key_values().visit(&mut fields);
        thread::record(|w, t| {
            w.log(
                t,
                callsite,
                lvl,
                Source::Log,
                &message,
                fields.0.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
        });
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        ours(metadata) || self.inner.as_ref().is_some_and(|i| i.enabled(metadata))
    }

    fn log(&self, record: &Record<'_>) {
        if ours(record.metadata()) {
            Self::record(record);
        }
        if let Some(inner) = &self.inner
            && inner.enabled(record.metadata())
        {
            inner.log(record);
        }
    }

    fn flush(&self) {
        if let Some(inner) = &self.inner {
            inner.flush();
        }
    }
}

fn to_log_filter(level: sismo_format::LevelFilter) -> LevelFilter {
    match level {
        sismo_format::LevelFilter::Off => LevelFilter::Off,
        sismo_format::LevelFilter::Error => LevelFilter::Error,
        sismo_format::LevelFilter::Warn => LevelFilter::Warn,
        sismo_format::LevelFilter::Info => LevelFilter::Info,
        sismo_format::LevelFilter::Debug => LevelFilter::Debug,
        sismo_format::LevelFilter::Trace => LevelFilter::Trace,
    }
}

pub(crate) fn refresh_max_level() {
    if let Some(inner) = INNER_MAX.get() {
        ::log::set_max_level(to_log_filter(registry().filter().max_level()).max(*inner));
    }
}

/// Installs [`Logger`] as the global `log` logger. `inner_max` is the most
/// verbose level the wrapped logger wants (use `LevelFilter::Off` without one).
///
/// # Errors
/// [`::log::SetLoggerError`] when another logger is already set.
pub fn init(
    inner: Option<Box<dyn Log>>,
    inner_max: LevelFilter,
) -> Result<(), ::log::SetLoggerError> {
    ::log::set_boxed_logger(Box::new(Logger::new(inner)))?;
    let _ = INNER_MAX.set(inner_max);
    refresh_max_level();
    Ok(())
}

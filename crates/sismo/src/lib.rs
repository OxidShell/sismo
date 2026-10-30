//! In-app recorder: scopes, logs, counters and frame marks.
//!
//! Every source (own macros, `log`, `tracing`, stdio) is opt-in via a cargo
//! feature and funnels into per-thread buffers that are flushed as
//! `sismo-format` chunks to rotating files, an in-memory flight recorder or
//! viewers on the network.
//!
//! ```no_run
//! let handle = sismo::install(sismo::config!().record_to("traces")).unwrap();
//!
//! #[sismo::function]
//! fn work() {
//!     sismo::scope!("inner");
//!     sismo::counter!("queue_len", 3);
//! }
//!
//! work();
//! sismo::frame_mark();
//! handle.flush();
//! ```

mod collector;
mod config;
mod device;
mod handle;
mod panic;
mod registry;
mod scope;
mod sink;
mod thread;
mod time;

#[cfg(feature = "log")]
pub mod log;
#[cfg(feature = "net")]
mod net;
#[cfg(all(feature = "stdio", unix))]
mod stdio;
#[cfg(feature = "tracing")]
mod tracing_layer;

#[cfg(feature = "net")]
pub use config::LiveConfig;
pub use config::{Config, DEFAULT_FLIGHT_BYTES, FlightConfig, RecordConfig};
pub use handle::{Handle, InstallError, handle, install};
pub use registry::Callsite;
pub use scope::{Instrumented, ScopeGuard};
pub use sismo_format as format;
pub use sismo_macros::function;
#[cfg(feature = "tracing")]
pub use tracing_layer::SismoLayer;

#[doc(hidden)]
pub mod __private {
    pub use sismo_format::CallsiteKind;

    pub fn counter(callsite: &'static crate::Callsite, value: f64) {
        crate::thread::record(|w, t| w.counter(t, callsite.id(), value));
    }

    pub fn instant(callsite: &'static crate::Callsite) {
        if callsite.enabled() {
            crate::thread::record(|w, t| w.instant(t, callsite.id()));
        }
    }
}

/// Starts a [`Config`] named after the calling crate.
#[macro_export]
macro_rules! config {
    () => {
        $crate::Config::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    };
}

/// Records a span named `$name` until the end of the enclosing block.
#[macro_export]
macro_rules! scope {
    () => {
        $crate::scope!(concat!(module_path!(), ":", line!()))
    };
    ($name:expr) => {
        let _sismo_scope = {
            static CALLSITE: $crate::Callsite = $crate::Callsite::new(
                $crate::__private::CallsiteKind::Span,
                $name,
                module_path!(),
                file!(),
                line!(),
            );
            $crate::ScopeGuard::enter(&CALLSITE)
        };
    };
}

/// Records the current value of a named counter.
#[macro_export]
macro_rules! counter {
    ($name:expr, $value:expr) => {{
        static CALLSITE: $crate::Callsite = $crate::Callsite::new(
            $crate::__private::CallsiteKind::Counter,
            $name,
            module_path!(),
            file!(),
            line!(),
        );
        #[allow(clippy::cast_precision_loss, clippy::cast_lossless)]
        $crate::__private::counter(&CALLSITE, ($value) as f64);
    }};
}

/// Records a zero-length event.
#[macro_export]
macro_rules! instant {
    ($name:expr) => {{
        static CALLSITE: $crate::Callsite = $crate::Callsite::new(
            $crate::__private::CallsiteKind::Event,
            $name,
            module_path!(),
            file!(),
            line!(),
        );
        $crate::__private::instant(&CALLSITE);
    }};
}

/// Marks the end of a frame on the calling thread and flushes its buffer.
pub fn frame_mark() {
    thread::record(sismo_format::BatchWriter::frame);
    thread::flush_current();
}

/// Records a user note at the current instant (shown on every timeline).
pub fn marker(note: &str) {
    thread::record(|w, t| w.marker(t, note));
    thread::flush_current();
}

/// Whether a recorder is installed and active.
#[must_use]
pub fn is_active() -> bool {
    registry::ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

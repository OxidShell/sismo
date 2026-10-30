use std::panic::PanicHookInfo;
use std::time::Duration;

use crate::handle::handle;
use crate::thread::{flush_current, record};

fn message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic with a non-string payload".to_owned())
}

/// Records panics, flushes them and dumps the flight recorder before the
/// previous hook runs.
pub(crate) fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let text = message(info);
        record(|w, t| w.panic(t, &text, &location));
        flush_current();
        if let Some(handle) = handle() {
            handle.flush_timeout(Duration::from_secs(2));
            if handle.ring().is_some() {
                // A failed dump must not mask the original panic.
                let _ = handle.dump_flight_recorder("panic");
            }
        }
        previous(info);
    }));
}

use std::backtrace::Backtrace;
use std::panic::PanicHookInfo;

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

/// Frames that only appear in the panic machinery on top of a backtrace
/// (`std::panicking::catch_unwind` sits at the bottom, so it is no mark).
fn is_machinery(frame: &str) -> bool {
    [
        "core::panicking::",
        "rust_begin_unwind",
        "std::panicking::begin_panic",
        "std::panicking::rust_panic_with_hook",
        "std::panicking::panic_with_hook",
    ]
    .iter()
    .any(|m| frame.contains(m))
}

/// The backtrace from the frame that panicked on, without the frames of
/// the panic machinery above it.
fn trimmed(backtrace: &str) -> String {
    let mut frames: Vec<String> = Vec::new();
    for line in backtrace.lines() {
        let starts_frame = line
            .trim_start()
            .split_once(": ")
            .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        match frames.last_mut() {
            Some(frame) if !starts_frame => {
                frame.push('\n');
                frame.push_str(line);
            }
            _ => frames.push(line.to_owned()),
        }
    }
    let first = frames
        .iter()
        .rposition(|f| is_machinery(f.lines().next().unwrap_or_default()))
        .map_or(0, |i| i + 1);
    if first >= frames.len() {
        return backtrace.trim_end().to_owned();
    }
    frames[first..].join("\n")
}

/// Records panics with their backtrace, gets them to viewers and dumps
/// the flight recorder before the previous hook runs.
pub(crate) fn install_hook() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(chain_hook);
}

fn chain_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let text = message(info);
        let backtrace = trimmed(&Backtrace::force_capture().to_string());
        record(|w, t| w.panic(t, &text, &location, &backtrace));
        flush_current();
        if let Some(handle) = handle() {
            handle.report_panic();
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_the_panic_machinery_frames() {
        let raw = "   0: std::backtrace::Backtrace::force_capture
             at /rustc/library/std/src/backtrace.rs:312:9
   1: sismo::panic::chain_hook::{{closure}}
   2: std::panicking::rust_panic_with_hook
   3: core::panicking::panic_fmt
   4: app::canvas::draw
             at ./src/canvas.rs:10:5
   5: app::main
   6: std::panicking::catch_unwind
";
        assert_eq!(
            trimmed(raw),
            "   4: app::canvas::draw\n             at ./src/canvas.rs:10:5\n   5: app::main\n   6: std::panicking::catch_unwind"
        );
    }

    #[test]
    fn keeps_an_unrecognised_backtrace_whole() {
        assert_eq!(trimmed("disabled backtrace\n"), "disabled backtrace");
    }
}

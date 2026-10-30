#![allow(unsafe_code)]

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, RawFd};

use sismo_format::{Level, Source};

use crate::registry::{DynamicKey, registry};
use crate::thread;

/// Redirects stdout and stderr through pipes; a reader thread per stream
/// records each line and copies it to the original descriptor. Lines are
/// only recorded while sismo is active, so this runs once per process.
pub(crate) fn capture() -> io::Result<()> {
    static CAPTURED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if CAPTURED.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Ok(());
    }
    for (fd, source, name) in [(1, Source::Stdout, "stdout"), (2, Source::Stderr, "stderr")] {
        let (read_end, original) = redirect(fd)?;
        let callsite = registry().register_dynamic(
            DynamicKey {
                kind: 1,
                name: name.to_owned(),
                target: name.to_owned(),
                file: String::new(),
                line: 0,
            },
            name,
            Level::Info,
        );
        std::thread::Builder::new()
            .name(format!("sismo-{name}"))
            .spawn(move || pump(read_end, original, source, callsite))?;
    }
    Ok(())
}

/// Returns the pipe's read end and a duplicate of the original descriptor.
fn redirect(fd: RawFd) -> io::Result<(File, File)> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` is a valid two-element array as `pipe` requires.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a standard stream descriptor owned by the process.
    let original = unsafe { libc::dup(fd) };
    if original < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both descriptors are open; dup2 atomically replaces `fd`.
    if unsafe { libc::dup2(fds[1], fd) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the write end now lives on as `fd`; this copy is closed once.
    unsafe { libc::close(fds[1]) };
    // SAFETY: `fds[0]` and `original` are fresh descriptors owned only here.
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(original)) })
}

fn pump(read_end: File, mut original: File, source: Source, callsite: u32) {
    let mut reader = BufReader::new(read_end);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let _ = original.write_all(&line);
                let text = String::from_utf8_lossy(&line);
                let text = text.trim_end_matches(['\n', '\r']);
                thread::record(|w, t| {
                    w.log(t, callsite, Level::Info, source, text, std::iter::empty());
                });
            }
        }
    }
}

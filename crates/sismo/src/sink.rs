mod file;
mod ring;

use std::sync::{Arc, Mutex, MutexGuard};

use sismo_format::Chunk;

pub(crate) use file::RotatingFile;
pub(crate) use ring::RingBuffer;

/// Destination for sealed chunks, driven by the collector thread.
pub(crate) trait Sink: Send {
    /// Called once before the first chunk with every definition known so far.
    fn open(&mut self, snapshot: &Chunk);
    /// Returns `false` when the sink is finished and should be dropped.
    fn write(&mut self, seq: u64, chunk: &Arc<Chunk>) -> bool;
    fn flush(&mut self);
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

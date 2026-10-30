use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};

use sismo_format::{BatchWriter, Encoder};

use crate::collector::Message;
use crate::registry::{ACTIVE, now, registry};

/// Flush when a thread buffer grows past this many bytes.
const FLUSH_BYTES: usize = 32 * 1024;
/// Flush when this many nanoseconds passed since the last flush.
const FLUSH_INTERVAL_NS: u64 = 50_000_000;

static SENDER: OnceLock<SyncSender<Message>> = OnceLock::new();
/// Batches lost because the collector queue was full.
pub(crate) static DROPPED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn install_sender(sender: SyncSender<Message>) -> bool {
    SENDER.set(sender).is_ok()
}

pub(crate) fn sender() -> Option<&'static SyncSender<Message>> {
    SENDER.get()
}

struct ThreadBuffer {
    id: u32,
    writer: BatchWriter,
    last_flush: u64,
    dropped: u64,
}

impl ThreadBuffer {
    fn new() -> Self {
        let current = std::thread::current();
        let name = current
            .name()
            .map_or_else(|| format!("{:?}", current.id()), str::to_owned);
        Self {
            id: registry().register_thread(name),
            writer: BatchWriter::default(),
            last_flush: now(),
            dropped: 0,
        }
    }

    fn flush(&mut self, t: u64) {
        self.last_flush = t;
        if self.dropped > 0 {
            self.writer.dropped(t, self.dropped);
        }
        if self.writer.is_empty() {
            return;
        }
        let Some(sender) = sender() else {
            self.writer.clear();
            return;
        };
        let mut enc = Encoder::with_capacity(self.writer.byte_len() + 16);
        self.writer.encode_record(self.id, &mut enc);
        let events = self.writer.event_count() as u64;
        self.writer.clear();
        match sender.try_send(Message::Batch(enc.into_inner())) {
            Ok(()) => self.dropped = 0,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped += events;
                DROPPED.fetch_add(events, Ordering::Relaxed);
            }
        }
    }

    fn maybe_flush(&mut self, t: u64) {
        if self.writer.byte_len() >= FLUSH_BYTES
            || t.saturating_sub(self.last_flush) >= FLUSH_INTERVAL_NS
        {
            self.flush(t);
        }
    }
}

impl Drop for ThreadBuffer {
    fn drop(&mut self) {
        self.flush(now());
    }
}

thread_local! {
    static BUFFER: RefCell<Option<ThreadBuffer>> = const { RefCell::new(None) };
}

/// Runs `f` against this thread's batch writer, then flushes when due.
/// Does nothing while the recorder is inactive or during thread teardown.
#[inline]
pub(crate) fn record(f: impl FnOnce(&mut BatchWriter, u64)) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let t = now();
    let _ = BUFFER.try_with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return;
        };
        let buffer = slot.get_or_insert_with(ThreadBuffer::new);
        f(&mut buffer.writer, t);
        buffer.maybe_flush(t);
    });
}

/// Forces this thread's pending events to the collector.
pub(crate) fn flush_current() {
    let _ = BUFFER.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut()
            && let Some(buffer) = slot.as_mut()
        {
            buffer.flush(now());
        }
    });
}

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sismo_format::{Chunk, Codec, Encoder};

use crate::registry::registry;
use crate::sink::{RingBuffer, Sink};

/// Seal a chunk once its raw payload reaches this size.
const SEAL_BYTES: usize = 128 * 1024;
/// Seal a non-empty chunk after this long.
const SEAL_INTERVAL: Duration = Duration::from_millis(100);
/// How often the live heap is sampled when counting allocations.
#[cfg(feature = "alloc")]
const HEAP_SAMPLE: Duration = Duration::from_millis(250);

pub(crate) enum Message {
    /// One encoded [`Record::Batch`] from a thread buffer.
    Batch(Vec<u8>),
    AddSink(Box<dyn Sink>),
    Flush(SyncSender<()>),
    Dump {
        path: PathBuf,
        reply: SyncSender<io::Result<u64>>,
    },
    Shutdown(SyncSender<()>),
}

/// Encodes every definition known so far into one chunk.
pub(crate) fn snapshot_chunk() -> Option<Chunk> {
    Chunk::from_records(&registry().snapshot()).ok()
}

pub(crate) struct Collector {
    rx: Receiver<Message>,
    sinks: Vec<Box<dyn Sink>>,
    ring: Option<Arc<Mutex<RingBuffer>>>,
    pending: Encoder,
    opened: Instant,
    seq: u64,
    #[cfg(feature = "alloc")]
    heap_sampled: Instant,
}

impl Collector {
    pub fn new(rx: Receiver<Message>, ring: Option<Arc<Mutex<RingBuffer>>>) -> Self {
        Self {
            rx,
            sinks: Vec::new(),
            ring,
            pending: Encoder::with_capacity(SEAL_BYTES),
            opened: Instant::now(),
            seq: 0,
            #[cfg(feature = "alloc")]
            heap_sampled: Instant::now(),
        }
    }

    #[cfg(feature = "alloc")]
    fn sample_heap(&mut self) {
        if !crate::alloc::installed() || self.heap_sampled.elapsed() < HEAP_SAMPLE {
            return;
        }
        self.heap_sampled = Instant::now();
        let (allocated, freed, count) = crate::alloc::totals();
        crate::counter!("alloc.live_bytes", allocated.saturating_sub(freed));
        crate::counter!("alloc.total_bytes", allocated);
        crate::counter!("alloc.count", count);
    }

    pub fn run(mut self) {
        loop {
            #[cfg(feature = "alloc")]
            self.sample_heap();
            match self.rx.recv_timeout(SEAL_INTERVAL) {
                Ok(Message::Batch(bytes)) => {
                    self.append_definitions();
                    if self.pending.is_empty() {
                        self.opened = Instant::now();
                    }
                    self.pending.raw(&bytes);
                    if self.pending.len() >= SEAL_BYTES || self.opened.elapsed() >= SEAL_INTERVAL {
                        self.seal();
                    }
                }
                Ok(Message::AddSink(mut sink)) => {
                    self.seal();
                    if let Some(snapshot) = snapshot_chunk() {
                        sink.open(&snapshot);
                    }
                    self.sinks.push(sink);
                }
                Ok(Message::Flush(reply)) => {
                    self.drain_queued();
                    self.seal();
                    self.sinks.iter_mut().for_each(|sink| sink.flush());
                    let _ = reply.send(());
                }
                Ok(Message::Dump { path, reply }) => {
                    self.drain_queued();
                    self.seal();
                    let result = match &self.ring {
                        Some(ring) => crate::sink::lock(ring).dump(&path),
                        None => Err(io::Error::other("flight recorder is not enabled")),
                    };
                    let _ = reply.send(result);
                }
                Ok(Message::Shutdown(reply)) => {
                    self.drain_queued();
                    self.seal();
                    self.sinks.iter_mut().for_each(|sink| sink.flush());
                    let _ = reply.send(());
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !self.pending.is_empty() && self.opened.elapsed() >= SEAL_INTERVAL {
                        self.seal();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.seal();
                    self.sinks.iter_mut().for_each(|sink| sink.flush());
                    return;
                }
            }
        }
    }

    /// Pulls batches that are already queued so a flush includes them.
    fn drain_queued(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Batch(bytes) => {
                    self.append_definitions();
                    self.pending.raw(&bytes);
                }
                Message::AddSink(sink) => self.sinks.push(sink),
                Message::Flush(reply) | Message::Shutdown(reply) => {
                    let _ = reply.send(());
                }
                Message::Dump { reply, .. } => {
                    let _ = reply.send(Err(io::Error::other("dump raced with another flush")));
                }
            }
        }
        self.append_definitions();
    }

    fn append_definitions(&mut self) {
        for record in registry().take_pending() {
            record.encode(&mut self.pending);
        }
    }

    fn seal(&mut self) {
        self.append_definitions();
        if self.pending.is_empty() {
            return;
        }
        let Ok(chunk) = Chunk::seal(self.pending.as_slice(), Codec::Lz4) else {
            self.pending.clear();
            return;
        };
        self.pending.clear();
        let chunk = Arc::new(chunk);
        self.seq += 1;
        let seq = self.seq;
        if let Some(ring) = &self.ring {
            crate::sink::lock(ring).push(seq, Arc::clone(&chunk));
        }
        self.sinks.retain_mut(|sink| sink.write(seq, &chunk));
    }
}

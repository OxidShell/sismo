use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;
use std::sync::Arc;

use sismo_format::{Chunk, FileWriter};

use crate::collector::snapshot_chunk;

/// Bounded in-memory history of sealed chunks, oldest evicted first.
#[derive(Debug)]
pub(crate) struct RingBuffer {
    chunks: VecDeque<(u64, Arc<Chunk>)>,
    bytes: usize,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
            capacity,
        }
    }

    pub fn push(&mut self, seq: u64, chunk: Arc<Chunk>) {
        self.bytes += chunk.len();
        self.chunks.push_back((seq, chunk));
        while self.bytes > self.capacity && self.chunks.len() > 1 {
            if let Some((_, old)) = self.chunks.pop_front() {
                self.bytes -= old.len();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Chunks currently held, oldest first.
    pub fn history(&self) -> Vec<(u64, Arc<Chunk>)> {
        self.chunks.iter().cloned().collect()
    }

    /// Writes the held history as a standalone `.sismo` file.
    pub fn dump(&self, path: &Path) -> io::Result<u64> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut writer = FileWriter::new(BufWriter::new(File::create(path)?))?;
        if let Some(snapshot) = snapshot_chunk() {
            writer.write_chunk(&snapshot)?;
        }
        for (_, chunk) in &self.chunks {
            writer.write_chunk(chunk)?;
        }
        writer.flush()?;
        Ok(writer.bytes_written())
    }
}

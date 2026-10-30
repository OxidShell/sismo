use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;

use sismo_format::{Chunk, FileWriter};

use super::Sink;
use crate::collector::snapshot_chunk;
use sismo_format::FileStamp;

/// Writes chunks to `.sismo` files, starting a new file when one grows past
/// `max_file_bytes` and keeping at most `keep` files.
pub(crate) struct RotatingFile {
    dir: PathBuf,
    max_file_bytes: u64,
    keep: usize,
    current: Option<FileWriter<BufWriter<File>>>,
    files: VecDeque<PathBuf>,
    sequence: u32,
}

impl RotatingFile {
    pub fn new(dir: PathBuf, max_file_bytes: u64, keep: usize) -> Self {
        Self {
            dir,
            max_file_bytes,
            keep: keep.max(1),
            current: None,
            files: VecDeque::new(),
            sequence: 0,
        }
    }

    fn open_next(&mut self, snapshot: Option<&Chunk>) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        self.sequence += 1;
        let stem = crate::registry::registry()
            .session()
            .file_stem(FileStamp::now());
        let name = format!("{stem}-{}.sismo", self.sequence);
        let path = self.dir.join(name);
        let mut writer = FileWriter::new(BufWriter::new(File::create(&path)?))?;
        match snapshot {
            Some(chunk) => writer.write_chunk(chunk)?,
            None => {
                if let Some(chunk) = snapshot_chunk() {
                    writer.write_chunk(&chunk)?;
                }
            }
        }
        self.files.push_back(path);
        while self.files.len() > self.keep {
            if let Some(old) = self.files.pop_front() {
                // Best effort: a file removed by the user is already gone.
                let _ = std::fs::remove_file(old);
            }
        }
        self.current = Some(writer);
        Ok(())
    }
}

impl Sink for RotatingFile {
    fn open(&mut self, snapshot: &Chunk) {
        if self.open_next(Some(snapshot)).is_err() {
            self.current = None;
        }
    }

    fn write(&mut self, _seq: u64, chunk: &Arc<Chunk>) -> bool {
        let full = self
            .current
            .as_ref()
            .is_none_or(|w| w.bytes_written() + chunk.len() as u64 > self.max_file_bytes);
        if full {
            if let Some(mut old) = self.current.take() {
                let _ = old.flush();
            }
            if self.open_next(None).is_err() {
                return false;
            }
        }
        match self.current.as_mut() {
            Some(writer) => writer.write_chunk(chunk).is_ok(),
            None => false,
        }
    }

    fn flush(&mut self) {
        if let Some(writer) = self.current.as_mut() {
            let _ = writer.flush();
        }
    }
}

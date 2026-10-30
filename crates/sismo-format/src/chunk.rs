use std::io::{self, Read, Write};

use crate::{DecodeError, FORMAT_VERSION, Record, decode_records};

/// Magic bytes at the start of every `.sismo` file.
pub const FILE_MAGIC: [u8; 8] = *b"SISMO\0\x01\n";
/// Magic bytes at the start of every chunk header.
pub const CHUNK_MAGIC: [u8; 4] = *b"SMCK";
/// Size of the fixed chunk header.
pub const CHUNK_HEADER_LEN: usize = 20;
/// Upper bound for a decompressed chunk; keeps a corrupt length from
/// allocating gigabytes.
pub const MAX_CHUNK_LEN: usize = 64 * 1024 * 1024;

/// Payload compression of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Codec {
    None = 0,
    #[default]
    Lz4 = 1,
}

impl TryFrom<u8> for Codec {
    type Error = DecodeError;

    fn try_from(value: u8) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(Codec::None),
            1 => Ok(Codec::Lz4),
            other => Err(DecodeError::InvalidTag {
                what: "codec",
                tag: other,
            }),
        }
    }
}

/// A sealed, possibly compressed group of records, identical on the wire and
/// on disk:
///
/// `magic[4] | version u8 | codec u8 | flags u8 | reserved u8 |
///  raw_len u32 | payload_len u32 | crc32(payload) u32 | payload`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    bytes: Vec<u8>,
}

impl Chunk {
    /// Seals an encoded record payload, compressing it with `codec`.
    ///
    /// # Errors
    /// [`DecodeError::TooLarge`] when the payload exceeds [`MAX_CHUNK_LEN`].
    pub fn seal(raw: &[u8], codec: Codec) -> Result<Self, DecodeError> {
        if raw.len() > MAX_CHUNK_LEN {
            return Err(DecodeError::TooLarge);
        }
        let payload = match codec {
            Codec::None => raw.to_vec(),
            Codec::Lz4 => lz4_flex::block::compress(raw),
        };
        let raw_len = u32::try_from(raw.len()).map_err(|_| DecodeError::TooLarge)?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| DecodeError::TooLarge)?;
        let mut bytes = Vec::with_capacity(CHUNK_HEADER_LEN + payload.len());
        bytes.extend_from_slice(&CHUNK_MAGIC);
        bytes.extend_from_slice(&[FORMAT_VERSION, codec as u8, 0, 0]);
        bytes.extend_from_slice(&raw_len.to_le_bytes());
        bytes.extend_from_slice(&payload_len.to_le_bytes());
        bytes.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
        bytes.extend_from_slice(&payload);
        Ok(Self { bytes })
    }

    /// Seals records with the default codec.
    ///
    /// # Errors
    /// Same as [`Chunk::seal`].
    pub fn from_records(records: &[Record]) -> Result<Self, DecodeError> {
        let mut enc = crate::Encoder::default();
        for record in records {
            record.encode(&mut enc);
        }
        Self::seal(enc.as_slice(), Codec::default())
    }

    /// Validates a full chunk received as one message.
    ///
    /// # Errors
    /// Header, length or checksum errors.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, DecodeError> {
        let header = ChunkHeader::parse(&bytes)?;
        if bytes.len() != CHUNK_HEADER_LEN + header.payload_len {
            return Err(DecodeError::Truncated);
        }
        header.verify(&bytes[CHUNK_HEADER_LEN..])?;
        Ok(Self { bytes })
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Decompresses and decodes the records.
    ///
    /// # Errors
    /// Any [`DecodeError`] for malformed content.
    pub fn records(&self) -> Result<Vec<Record>, DecodeError> {
        let header = ChunkHeader::parse(&self.bytes)?;
        let raw = header.decompress(&self.bytes[CHUNK_HEADER_LEN..])?;
        decode_records(&raw)
    }
}

#[derive(Debug, Clone, Copy)]
struct ChunkHeader {
    codec: Codec,
    raw_len: usize,
    payload_len: usize,
    crc: u32,
}

impl ChunkHeader {
    fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        let header = bytes
            .get(..CHUNK_HEADER_LEN)
            .ok_or(DecodeError::Truncated)?;
        if header[..4] != CHUNK_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if header[4] != FORMAT_VERSION {
            return Err(DecodeError::UnsupportedVersion(header[4]));
        }
        let word = |at: usize| {
            u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let raw_len = word(8) as usize;
        let payload_len = word(12) as usize;
        if raw_len > MAX_CHUNK_LEN || payload_len > MAX_CHUNK_LEN {
            return Err(DecodeError::TooLarge);
        }
        Ok(Self {
            codec: Codec::try_from(header[5])?,
            raw_len,
            payload_len,
            crc: word(16),
        })
    }

    fn verify(self, payload: &[u8]) -> Result<(), DecodeError> {
        if crc32fast::hash(payload) == self.crc {
            Ok(())
        } else {
            Err(DecodeError::Checksum)
        }
    }

    fn decompress(self, payload: &[u8]) -> Result<Vec<u8>, DecodeError> {
        self.verify(payload)?;
        match self.codec {
            Codec::None => Ok(payload.to_vec()),
            Codec::Lz4 => lz4_flex::block::decompress(payload, self.raw_len)
                .map_err(|_| DecodeError::Decompress),
        }
    }
}

/// Writes a `.sismo` file: magic followed by chunks.
#[derive(Debug)]
pub struct FileWriter<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> FileWriter<W> {
    /// # Errors
    /// I/O errors from writing the magic.
    pub fn new(mut inner: W) -> io::Result<Self> {
        inner.write_all(&FILE_MAGIC)?;
        Ok(Self {
            inner,
            written: FILE_MAGIC.len() as u64,
        })
    }

    /// # Errors
    /// I/O errors from the underlying writer.
    pub fn write_chunk(&mut self, chunk: &Chunk) -> io::Result<()> {
        self.inner.write_all(chunk.as_bytes())?;
        self.written += chunk.len() as u64;
        Ok(())
    }

    /// # Errors
    /// I/O errors from the underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    #[must_use]
    pub fn bytes_written(&self) -> u64 {
        self.written
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

/// Outcome of reading one chunk from a file.
#[derive(Debug)]
pub enum ReadOutcome {
    Chunk(Vec<Record>),
    /// A chunk header or payload that could not be used; reading stops.
    Damaged {
        offset: u64,
        error: DecodeError,
    },
}

/// Streams chunks out of a `.sismo` file, tolerating a damaged tail.
#[derive(Debug)]
pub struct FileReader<R: Read> {
    inner: R,
    offset: u64,
    done: bool,
}

impl<R: Read> FileReader<R> {
    /// # Errors
    /// [`DecodeError::BadMagic`] when the file is not a sismo recording, or
    /// [`DecodeError::Io`].
    pub fn new(mut inner: R) -> Result<Self, DecodeError> {
        let mut magic = [0u8; FILE_MAGIC.len()];
        inner.read_exact(&mut magic)?;
        if magic != FILE_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        Ok(Self {
            inner,
            offset: FILE_MAGIC.len() as u64,
            done: false,
        })
    }

    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    fn read_chunk(&mut self) -> Result<Option<Vec<Record>>, DecodeError> {
        let mut header = [0u8; CHUNK_HEADER_LEN];
        let mut filled = 0;
        while filled < header.len() {
            match self.inner.read(&mut header[filled..])? {
                0 if filled == 0 => return Ok(None),
                0 => return Err(DecodeError::Truncated),
                n => filled += n,
            }
        }
        let parsed = ChunkHeader::parse(&header)?;
        let mut payload = vec![0u8; parsed.payload_len];
        self.inner
            .read_exact(&mut payload)
            .map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
                _ => DecodeError::Io(e.to_string()),
            })?;
        let raw = parsed.decompress(&payload)?;
        let records = decode_records(&raw)?;
        self.offset += (CHUNK_HEADER_LEN + payload.len()) as u64;
        Ok(Some(records))
    }
}

impl<R: Read> Iterator for FileReader<R> {
    type Item = ReadOutcome;

    fn next(&mut self) -> Option<ReadOutcome> {
        if self.done {
            return None;
        }
        match self.read_chunk() {
            Ok(Some(records)) => Some(ReadOutcome::Chunk(records)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(error) => {
                self.done = true;
                Some(ReadOutcome::Damaged {
                    offset: self.offset,
                    error,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, ThreadDef};

    fn sample() -> Vec<Record> {
        vec![
            Record::Thread(ThreadDef {
                id: 1,
                name: "main".into(),
            }),
            Record::Batch {
                thread: 1,
                events: (0..200)
                    .flat_map(|i| {
                        [
                            Event::Begin {
                                t: i * 10,
                                callsite: 1,
                            },
                            Event::End { t: i * 10 + 5 },
                        ]
                    })
                    .collect(),
            },
        ]
    }

    #[test]
    fn file_round_trip_and_damaged_tail() {
        let chunk = Chunk::from_records(&sample()).unwrap();
        let mut writer = FileWriter::new(Vec::new()).unwrap();
        writer.write_chunk(&chunk).unwrap();
        writer.write_chunk(&chunk).unwrap();
        let mut bytes = writer.into_inner();
        let full: Vec<_> = FileReader::new(bytes.as_slice()).unwrap().collect();
        assert_eq!(full.len(), 2);
        assert!(matches!(&full[0], ReadOutcome::Chunk(r) if *r == sample()));

        bytes.truncate(bytes.len() - 3);
        let cut: Vec<_> = FileReader::new(bytes.as_slice()).unwrap().collect();
        assert_eq!(cut.len(), 2);
        assert!(matches!(
            cut[1],
            ReadOutcome::Damaged {
                error: DecodeError::Truncated,
                ..
            }
        ));
    }

    #[test]
    fn corrupted_payload_fails_checksum() {
        let mut bytes = Chunk::from_records(&sample()).unwrap().into_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert_eq!(Chunk::from_bytes(bytes), Err(DecodeError::Checksum));
    }
}

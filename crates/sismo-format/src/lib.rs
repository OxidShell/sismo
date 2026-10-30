//! Chunked wire and file format shared by the `sismo` recorder, viewer and CLI.
//!
//! The same chunk bytes travel over the network and land in `.sismo` files,
//! so a live session and a recording are read by one decoder.

mod chunk;
mod codec;
mod filter;
pub mod net;
mod record;
mod stamp;

pub use chunk::{
    CHUNK_HEADER_LEN, CHUNK_MAGIC, Chunk, Codec, FILE_MAGIC, FileReader, FileWriter, MAX_CHUNK_LEN,
    ReadOutcome,
};
pub use codec::{Decoder, Encoder};
pub use filter::{Filter, FilterError, LevelFilter};
pub use record::{
    BatchWriter, CallsiteDef, CallsiteKind, Event, Level, Record, SessionInfo, Source, ThreadDef,
    decode_records,
};
pub use stamp::{FileStamp, unix_ns};

/// Version byte written into every chunk header.
pub const FORMAT_VERSION: u8 = 1;

/// Failure to decode sismo bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("input ended in the middle of a value")]
    Truncated,
    #[error("varint longer than 64 bits")]
    VarintOverflow,
    #[error("value does not fit its target type")]
    OutOfRange,
    #[error("string is not valid UTF-8")]
    InvalidUtf8,
    #[error("unknown {what} tag {tag}")]
    InvalidTag { what: &'static str, tag: u8 },
    #[error("not sismo data (bad magic)")]
    BadMagic,
    #[error("format version {0} is not supported (this build reads {FORMAT_VERSION})")]
    UnsupportedVersion(u8),
    #[error("chunk checksum mismatch")]
    Checksum,
    #[error("chunk payload could not be decompressed")]
    Decompress,
    #[error("chunk larger than the format allows")]
    TooLarge,
    #[error("i/o error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DecodeError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::UnexpectedEof => DecodeError::Truncated,
            _ => DecodeError::Io(error.to_string()),
        }
    }
}

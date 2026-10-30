//! Chunked wire and file format shared by the `sismo` recorder, viewer and CLI.
//!
//! The same chunk bytes travel over the network and land in `.sismo` files,
//! so a live session and a recording are read by one decoder.

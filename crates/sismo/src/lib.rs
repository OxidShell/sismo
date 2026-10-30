//! In-app recorder: scopes, logs, counters and frame marks.
//!
//! Every source (own macros, `tracing`, `log`, stdio, `puffin`) is opt-in via
//! a cargo feature and funnels into per-thread buffers that are flushed as
//! `sismo-format` chunks to a file, a ring buffer or the network.

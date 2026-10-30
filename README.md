# sismo

Remote and recorded performance + log analysis for Rust apps.

Record per-function scopes, logs (`tracing`, `log`, stdout/stderr) and
counters with low overhead, then inspect them live over the LAN or from a
`.sismo` recording in a desktop viewer — filtered by text or by crate, so you
can tell whether a slowdown lives in your app or in the framework.

Status: early development.

## License

MIT OR Apache-2.0.

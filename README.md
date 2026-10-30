# sismo

Remote and recorded performance + log analysis for Rust apps.

Record per-function scopes, logs (`tracing`, `log`, stdout/stderr) and
counters with low overhead, then inspect them live over the LAN or from a
`.sismo` recording in a desktop viewer, filtered by text or by crate, so you
can tell whether a slowdown lives in your app or in the framework.

| Crate | What it is |
|---|---|
| `sismo` | In-app recorder: macros, `log`/`tracing` bridges, stdio capture, rotating files, flight recorder, LAN streaming |
| `sismo-freya` | Freya plugin: frame marks, render phases, tree metrics |
| `sismo-format` | Chunked wire/file format shared by everything |
| `sismo-analysis` | Session model, query language, stats, exports |
| `sismo-viewer` | The `sismo` binary: desktop viewer + headless CLI |

## Instrumenting an app

```toml
[dependencies]
sismo = { version = "0.1", features = ["log", "net"] }
```

```rust
fn main() {
    let handle = sismo::install(
        sismo::config!()                 // app name + version from Cargo
            .filter("info,my_app=trace")
            .record_to("traces")         // rotating .sismo files
            .dump_dir("traces/dumps")    // flight recorder dumped on panic
            .live(sismo::LiveConfig::default())
            .from_env(),                 // SISMO=live,record=dir,flight=32,stdio
    )
    .expect("sismo");

    // Keep your logger: sismo records and forwards to it.
    sismo::log::init(Some(Box::new(env_logger::Builder::from_default_env().build())), log::LevelFilter::Trace)
        .expect("logger");

    loop {
        frame();
        sismo::frame_mark();
    }
}

#[sismo::function]
fn frame() {
    sismo::scope!("frame::layout");
    sismo::counter!("queue_len", 3);
}
```

- `#[sismo::function]` records every call (async functions from first poll
  to completion). `scope!`, `counter!`, `instant!`, `marker()` and
  `frame_mark()` cover the rest.
- Disabled cost is one relaxed atomic load; enabled scopes write a few
  varint bytes into a thread-local buffer. A collector thread compresses
  chunks (lz4) off the app threads and never blocks them: if it falls
  behind, events are dropped and counted.
- With the `tracing` feature, add `sismo::SismoLayer` to your subscriber;
  spans become scopes and events become logs.
- With `stdio` (Unix, Android included), `capture_stdio(true)` turns
  `println!`/`eprintln!` lines into logs and still prints them.
- With the `alloc` feature, install the counting allocator and every scope
  reports the bytes and allocations made while it ran (the viewer shows them
  in Funciones and the span detail; `alloc>64kb` filters them), and the live
  heap is sampled as `alloc.live_bytes`:
  `#[global_allocator] static A: sismo::alloc::Counting = sismo::alloc::Counting::system();`
- `handle.dump_flight_recorder("bug")` writes the in-memory history to a
  file, ready to send from a client device that you cannot reach over LAN.

### Freya

```rust
let config = LaunchConfig::new().with_plugin(sismo_freya::SismoPlugin::default());
```

Adds `freya::frame`, `freya::layout`, `freya::render`, `freya::present`,
`freya::tree_update`, `freya::tasks` and `freya::events` spans, a frame mark
per redraw and counters for tree/layout nodes, text cache and tasks.

### Live mode and pairing

`LiveConfig` advertises the app over UDP broadcast (port 7701) and serves
viewers on 7720 through zudp with Noise encryption. With
`AuthMode::Pairing` (the default) the app logs a 6-digit code
(`target: "sismo"`, also `handle.pairing_code()`); a viewer enters it once
and is remembered (persistently when `data_dir` is set). Viewers can change
the app-side filter, add markers and pull the ring-buffer history.

## Viewer and CLI

```sh
sismo                         # viewer: LAN apps + recent files
sismo open trace.sismo        # viewer on a recording
sismo open b.sismo --compare a.sismo
sismo connect 192.168.1.34:7720   # or: sismo connect my_app

sismo summary trace.sismo [--json] [-q QUERY]
sismo functions trace.sismo --sort p95 --limit 20 [--json]
sismo frames trace.sismo --limit 10
sismo logs trace.sismo -q 'level:warn+ "gpu"'
sismo export trace.sismo -f perfetto|chrome|json|md|sismo [-o out]
sismo compare a.sismo b.sismo --metric p95
sismo scan
sismo record my_app --duration 30s -o capture.sismo [--code 123456]
sismo tail my_app -q 'level:info+'
```

The viewer has Timeline (frame strip stacked by App/Framework/Other, per
thread span lanes, zoom with W/S or the wheel, pan with A/D or drag),
Flamegraph (aggregated icicle with callers/callees), Funciones (stats table
with p95 history and worst calls), Logs and Métricas. Ctrl K opens the
command palette, `?` lists shortcuts, Ctrl R records a live app, Ctrl E
exports, Ctrl D compares.

### Query syntax

Same in every tab and in the CLI (`-q`):

| Term | Meaning |
|---|---|
| `crate:freya_*`, `crate:app,framework` | crate or group (`app`, `framework`, `other`) |
| `fn:*paint*`, `fn:/^torin::/` | function name, glob or regex |
| `dur>2ms`, `self>1ms`, `calls>100`, `alloc>64kb` | span duration, self time, call count, allocated bytes |
| `level:warn+`, `source:stdout` | log level (`+` = that or worse), log source |
| `thread:main`, `file:canvas.rs`, `frame:slow` | thread, source file, slow frames |
| `range:5m`, `range:+10s..+20s` | last N of the session, or offsets |
| `"gpu budget"`, `-crate:std`, `a OR b` | free text, negation, alternatives |
| `find:"pairing"` | highlight without filtering |

## License

MIT OR Apache-2.0. The viewer embeds IBM Plex Sans and IBM Plex Mono under
the SIL Open Font License (`crates/sismo-viewer/assets/fonts/OFL.txt`).

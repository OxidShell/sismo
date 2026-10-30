//! Synthetic app: a 60 Hz "UI" loop plus a worker thread, recorded to
//! `./demo-traces` and streamed on the LAN.
//!
//! `cargo run -p sismo --example demo --features net,log`

use std::time::{Duration, Instant};

#[sismo::function]
fn layout(nodes: u32) -> u32 {
    (0..nodes).fold(0u32, |acc, n| acc.wrapping_add(n * 31))
}

#[sismo::function]
fn paint(frame: u64) {
    sismo::scope!("demo::paint::tiles");
    let busy = if frame.is_multiple_of(90) { 9 } else { 2 };
    spin(Duration::from_millis(busy));
}

#[sismo::function]
fn sync_tick(n: u64) {
    spin(Duration::from_micros(300 + (n % 7) * 100));
    if n.is_multiple_of(50) {
        log::warn!(target: "demo::sync", queue = n % 13; "sync queue is backing up");
    }
}

fn spin(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        std::hint::spin_loop();
    }
}

fn main() {
    let config = sismo::Config::new("demo", env!("CARGO_PKG_VERSION"))
        .filter("debug")
        .record_to("demo-traces")
        .dump_dir("demo-traces/dumps");
    #[cfg(feature = "net")]
    let config = config.live(sismo::LiveConfig {
        auth: sismo::format::net::AuthMode::Open,
        ..sismo::LiveConfig::default()
    });
    let handle = sismo::install(config).expect("install sismo");
    sismo::log::init(None, log::LevelFilter::Off).expect("install logger");
    log::info!(target: "demo", "demo started");

    std::thread::Builder::new()
        .name("sync".into())
        .spawn(|| {
            for n in 0.. {
                sync_tick(n);
                sismo::counter!("demo.queue_len", n % 13);
                std::thread::sleep(Duration::from_millis(20));
            }
        })
        .expect("spawn worker");

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut frame = 0u64;
    while Instant::now() < deadline {
        let started = Instant::now();
        {
            sismo::scope!("demo::frame");
            layout(2_000);
            paint(frame);
        }
        sismo::frame_mark();
        frame += 1;
        if frame == 120 {
            sismo::marker("halfway through warm-up");
        }
        std::thread::sleep(Duration::from_millis(16).saturating_sub(started.elapsed()));
    }
    log::info!(target: "demo", frames = frame; "demo finished");
    handle.flush();
}

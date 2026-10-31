//! Synthetic app: a 60 Hz "UI" loop plus a worker thread, recorded to
//! `./demo-traces` and streamed on the LAN.
//!
//! `cargo run -p sismo --example demo --features net,log`

use std::time::{Duration, Instant};

#[cfg(feature = "alloc")]
#[global_allocator]
static ALLOC: sismo::alloc::Counting = sismo::alloc::Counting::system();

#[sismo::function]
fn layout(nodes: u32) -> u32 {
    (0..nodes).fold(0u32, |acc, n| acc.wrapping_add(n * 31))
}

#[sismo::function]
fn paint(frame: u64) {
    sismo::scope!("demo::paint::tiles");
    let tiles: Vec<Vec<u8>> = (0..16).map(|i| vec![0u8; 1024 * (i + 1)]).collect();
    std::hint::black_box(&tiles);
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

#[sismo::function]
fn import_page(page: usize) {
    let pages: Vec<u32> = vec![1, 2, 3];
    log::info!(target: "demo::import", "importing page {page}");
    std::hint::black_box(pages[page]);
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
        auth: if std::env::var_os("DEMO_PAIRING").is_some() {
            sismo::format::net::AuthMode::Pairing
        } else {
            sismo::format::net::AuthMode::Open
        },
        ..sismo::LiveConfig::default()
    });
    let handle = sismo::install(config).expect("install sismo");
    sismo::log::init(None, log::LevelFilter::Off).expect("install logger");
    log::info!(target: "demo", "demo started");
    #[cfg(feature = "net")]
    if let Some(code) = handle.pairing_code() {
        println!("pairing code: {code}");
    }

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

    // `DEMO_PANIC=<seconds>`: a worker thread panics after that long, the
    // way a crash report comes to be.
    if let Some(after) = std::env::var("DEMO_PANIC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        std::thread::Builder::new()
            .name("importer".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_secs(after));
                import_page(7);
            })
            .expect("spawn importer");
    }

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

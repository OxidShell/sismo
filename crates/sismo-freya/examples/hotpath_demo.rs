//! A Freya app instrumented only with hotpath. sismo records every
//! `#[hotpath::measure]` scope through hotpath's function sink, next to
//! Freya's frame phases and `tracing` logs, and streams them on the LAN while
//! recording to `demo-traces/`.
//!
//! The animated list keeps Freya busy on every frame, so tree updates, layout
//! and render show up next to the app's own functions; the buttons add sync
//! work on the UI thread and an async fetch that spans several frames.
//!
//! `cargo run -p sismo-freya --example hotpath_demo`, then open the viewer
//! (`cargo run -p sismo-viewer`) and pick `hotpath-demo` under Live.

use std::time::Duration;

use freya::animation::{AnimNum, OnCreation, OnFinish, use_animation};
use freya::prelude::*;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Rows added or removed per press.
const ROWS_STEP: usize = 200;

#[hotpath::measure]
fn generate(count: u64, seed: u64) -> Vec<u64> {
    (0..count)
        .map(|i| (i ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 16)
        .collect()
}

#[hotpath::measure]
fn sort(mut values: Vec<u64>) -> Vec<u64> {
    values.sort_unstable();
    values
}

#[hotpath::measure]
fn summarize(values: &[u64]) -> String {
    let median = values.get(values.len() / 2).copied().unwrap_or_default();
    format!("{} values, median {median}", values.len())
}

/// Sync work: nested `#[hotpath::measure]` calls on the UI thread.
#[hotpath::measure]
fn crunch(seed: u64) -> String {
    summarize(&sort(generate(300_000, seed)))
}

#[hotpath::measure]
fn parse_page(page: u32) -> String {
    let rows = generate(20_000, u64::from(page));
    format!("page {page}: {} rows", rows.len())
}

/// Async work: suspends on a timer, so hotpath reports it as an async scope
/// that outlives the frames it spans.
#[hotpath::measure]
async fn fetch_page(page: u32) -> String {
    timer(Duration::from_millis(150)).await;
    parse_page(page)
}

/// One label per row, rebuilt on every render of the app component.
#[hotpath::measure]
fn rows(count: usize, seed: u64) -> Vec<String> {
    (0..count)
        .map(|i| format!("row {i:>4} · seed {seed}"))
        .collect()
}

/// A label and a bar whose width follows the animation, so every frame
/// relayouts the whole list.
fn bar_row(index: usize, text: String, phase: f32) -> Rect {
    #[allow(clippy::cast_precision_loss)]
    let offset = (index % 24) as f32 / 24.;
    let width = 24. + (phase + offset).fract() * 280.;
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(12.)
        .height(Size::px(18.))
        .child(label().text(text).width(Size::px(180.)))
        .child(
            rect()
                .width(Size::px(width))
                .height(Size::px(8.))
                .corner_radius(4.)
                .background((110, 150, 230)),
        )
}

fn app() -> impl IntoElement {
    let mut status = use_state(|| String::from("Press a button"));
    let mut seed = use_state(|| 1u64);
    let mut page = use_state(|| 0u32);
    let mut count = use_state(|| 2 * ROWS_STEP);
    let pulse = use_animation(|conf| {
        conf.on_creation(OnCreation::Run);
        conf.on_finish(OnFinish::restart());
        AnimNum::new(0., 1.).time(2000)
    });

    let phase = pulse.get().value();
    let rows = rows(*count.read(), *seed.read());

    rect()
        .expanded()
        .padding(16.)
        .spacing(12.)
        .child(status.read().clone())
        .child(
            rect()
                .horizontal()
                .spacing(8.)
                .child(
                    Button::new()
                        .on_press(move |_| {
                            let current = *seed.read();
                            status.set(crunch(current));
                            seed.set(current + 1);
                        })
                        .child("Crunch (sync)"),
                )
                .child(
                    Button::new()
                        .on_press(move |_| {
                            let next = *page.read() + 1;
                            page.set(next);
                            spawn(async move { status.set(fetch_page(next).await) });
                        })
                        .child("Fetch page (async)"),
                )
                .child(
                    Button::new()
                        .on_press(move |_| {
                            let next = *count.read() + ROWS_STEP;
                            count.set(next);
                        })
                        .child(format!("+{ROWS_STEP} rows")),
                )
                .child(
                    Button::new()
                        .on_press(move |_| {
                            let next = count.read().saturating_sub(ROWS_STEP);
                            count.set(next);
                        })
                        .child(format!("-{ROWS_STEP} rows")),
                ),
        )
        .child(
            ScrollView::new().expanded().children(
                rows.into_iter()
                    .enumerate()
                    .map(|(i, row)| bar_row(i, row, phase)),
            ),
        )
}

fn main() {
    let handle = sismo::install(
        sismo::Config::new("hotpath-demo", env!("CARGO_PKG_VERSION"))
            // Event-loop internals log at trace and Freya logs every tree
            // mutation at info; keep the rest. Spans are not level-filtered,
            // so the accessibility D-Bus traffic is switched off entirely.
            .filter("debug,freya_core::tree=warn,zbus=off")
            .record_to("demo-traces")
            .live(sismo::LiveConfig {
                auth: sismo::format::net::AuthMode::Open,
                ..sismo::LiveConfig::default()
            }),
    )
    .expect("install sismo");
    tracing_subscriber::registry()
        .with(sismo::SismoLayer)
        .init();

    launch(
        LaunchConfig::new()
            .with_plugin(sismo_freya::SismoPlugin::default())
            .with_window(WindowConfig::new(app).with_title("hotpath + sismo")),
    );
    handle.flush();
}

//! Metrics tab: frame time and every recorded counter as small charts.

use freya::prelude::*;
use sismo_analysis::Session;
use sismo_analysis::units::Nanos;

use super::draw::Scene;
use super::theme::{self, Rgb};
use super::timeline::scene;
use super::widgets::{bordered, mono, text};
use crate::workspace::SessionView;

const CHART_W: f32 = 560.;
const CHART_H: f32 = 200.;

/// Downsamples `(t, value)` points to at most `columns` (max per column).
fn columns(points: &[(u64, f64)], from: u64, to: u64, columns: usize) -> Vec<Option<f64>> {
    let mut out = vec![None; columns.max(1)];
    let span = to.saturating_sub(from).max(1);
    for (t, v) in points.iter().filter(|(t, _)| *t >= from && *t <= to) {
        let i = usize::try_from((t - from) * (out.len() as u64 - 1) / span)
            .unwrap_or(0)
            .min(out.len() - 1);
        out[i] = Some(out[i].map_or(*v, |old: f64| old.max(*v)));
    }
    out
}

fn chart(p: &mut Scene, series: &[(Vec<Option<f64>>, Rgb)], guide: Option<f64>, w: f32, h: f32) {
    p.rect(0., 0., w, h, theme::BG1);
    let max = series
        .iter()
        .flat_map(|(s, _)| s.iter().flatten().copied())
        .chain(guide)
        .fold(0f64, f64::max)
        .max(1e-9);
    #[allow(clippy::cast_possible_truncation)]
    let y = |v: f64| h - 8. - (v / max * f64::from(h - 24.)) as f32;
    p.line((0., h - 8.), (w, h - 8.), 1., theme::LINE2, 255);
    if let Some(g) = guide {
        p.line((0., y(g)), (w, y(g)), 1., theme::ERROR, 130);
    }
    for (values, rgb) in series {
        #[allow(clippy::cast_precision_loss)]
        let step = w / values.len().saturating_sub(1).max(1) as f32;
        let mut points = Vec::new();
        for (i, v) in values.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let x = i as f32 * step;
            match v {
                Some(v) => points.push((x, y(*v))),
                None if points.len() > 1 => {
                    p.polyline(&points, 1.5, *rgb);
                    points.clear();
                }
                None => points.clear(),
            }
        }
        p.polyline(&points, 1.5, *rgb);
    }
}

fn card(title: String, value: String, paint: RenderCallback) -> Element {
    rect()
        .width(Size::px(CHART_W + 32.))
        .padding(16.)
        .spacing(10.)
        .corner_radius(6.)
        .background(theme::color(theme::BG1))
        .border(bordered(theme::LINE))
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .child(
                    text(title, 13., theme::TEXT)
                        .font_weight(600)
                        .width(Size::flex(1.)),
                )
                .child(mono(value, 12., theme::TEXT2)),
        )
        .child(
            rect()
                .width(Size::px(CHART_W))
                .height(Size::px(CHART_H))
                .background(paint),
        )
        .into_element()
}

fn frame_card(session: &Session, from: u64, to: u64) -> Element {
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<(u64, f64)> = session
        .frames
        .iter()
        .map(|f| (f.start, f.duration() as f64 / 1e6))
        .collect();
    let summary = session.frame_summary((from, to));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cols = columns(&points, from, to, CHART_W as usize / 3);
    let paint = scene(CHART_W, CHART_H, move |p, w, h| {
        chart(p, &[(cols, theme::FRAMEWORK)], Some(16.6), w, h);
    });
    card(
        "Frame time".into(),
        format!(
            "p95 {} · máx {} · {} lentos",
            Nanos(summary.p95_ns),
            Nanos(summary.max_ns),
            summary.slow
        ),
        paint,
    )
}

pub fn view(view: &SessionView) -> Rect {
    let session = &view.session;
    let (from, to) = view.window();
    let mut counters: Vec<(u32, String)> = session
        .counters
        .keys()
        .map(|c| (*c, session.name(*c).to_owned()))
        .collect();
    counters.sort_by(|a, b| a.1.cmp(&b.1));
    let cards = std::iter::once(frame_card(session, from, to)).chain(counters.into_iter().map(
        |(callsite, name)| {
            let samples = &session.counters[&callsite];
            let last = samples
                .iter()
                .rev()
                .find(|(t, _)| *t <= to)
                .map_or(0., |s| s.1);
            let (lo, hi) = samples
                .iter()
                .filter(|(t, _)| *t >= from && *t <= to)
                .fold((f64::MAX, f64::MIN), |(lo, hi), (_, v)| {
                    (lo.min(*v), hi.max(*v))
                });
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cols = columns(samples, from, to, CHART_W as usize / 3);
            let rgb = theme::group(session.group_of(callsite));
            let paint = scene(CHART_W, CHART_H, move |p, w, h| {
                chart(p, &[(cols, rgb)], None, w, h);
            });
            let range = if lo <= hi {
                format!(" · rango {lo:.0}–{hi:.0}")
            } else {
                String::new()
            };
            card(name, format!("ahora {last:.0}{range}"), paint)
        },
    ));

    let mut rows: Vec<Element> = Vec::new();
    let mut pending: Vec<Element> = Vec::new();
    for card in cards {
        pending.push(card);
        if pending.len() == 2 {
            rows.push(
                rect()
                    .horizontal()
                    .spacing(16.)
                    .children(std::mem::take(&mut pending))
                    .into_element(),
            );
        }
    }
    if !pending.is_empty() {
        rows.push(
            rect()
                .horizontal()
                .spacing(16.)
                .children(pending)
                .into_element(),
        );
    }
    rect().expanded().child(
        ScrollView::new().height(Size::fill()).child(
            rect()
                .width(Size::fill())
                .padding(16.)
                .spacing(16.)
                .children(rows)
                .maybe(session.counters.is_empty(), |r| {
                    r.child(rect().padding(16.).child(text(
                        "La app no registró contadores · usa sismo::counter!(\"nombre\", valor)",
                        12.,
                        theme::TEXT3,
                    )))
                }),
        ),
    )
}

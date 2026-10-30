//! Functions tab: per-callsite stats table and the selected row's history.

use freya::prelude::*;
use sismo_analysis::units::{Clock, Nanos};
use sismo_analysis::{FunctionStats, SortBy};

use super::Workspace;
use super::i18n::tr;
use super::theme;
use super::timeline::{focus_frame, scene};
use super::widgets::{Kind, button, caption, hline, mono, swatch, text, vline};
use crate::workspace::{SessionView, Tab};

const ROW_H: f32 = 28.;
const BUCKETS: usize = 40;

const HISTORY_COLUMN: &str = "functions-col-history";

/// Column: header key, sort key, width, right aligned.
const COLUMNS: [(&str, Option<SortBy>, f32); 7] = [
    ("functions-col-calls", Some(SortBy::Calls), 80.),
    ("functions-col-total", Some(SortBy::Total), 86.),
    ("functions-col-self", Some(SortBy::SelfTime), 86.),
    ("functions-col-p50", None, 76.),
    ("functions-col-p95", Some(SortBy::P95), 76.),
    ("functions-col-max", Some(SortBy::Max), 76.),
    (HISTORY_COLUMN, None, 176.),
];

type Column = (&'static str, Option<SortBy>, f32);

/// Columns for this recording: with allocation data the p50 slot shows
/// self-allocated bytes instead (p50 stays in the detail panel).
fn columns(allocations: bool) -> [Column; 7] {
    let mut cols = COLUMNS;
    if allocations {
        cols[3] = ("functions-col-alloc", Some(SortBy::Alloc), 86.);
    }
    cols
}

fn cells(row: &FunctionStats, allocations: bool) -> [String; 6] {
    [
        row.calls.to_string(),
        Nanos(row.total_ns).to_string(),
        Nanos(row.self_ns).to_string(),
        if allocations {
            sismo_analysis::units::Bytes(row.alloc_self_bytes).to_string()
        } else {
            Nanos(row.p50_ns).to_string()
        },
        Nanos(row.p95_ns).to_string(),
        Nanos(row.max_ns).to_string(),
    ]
}

fn sparkline(history: &[u64], rgb: theme::Rgb) -> RenderCallback {
    let history = history.to_vec();
    scene(160., 20., move |p, w, h| {
        let max = history.iter().copied().max().unwrap_or(1).max(1);
        #[allow(clippy::cast_precision_loss)]
        let step = w / history.len().max(1) as f32;
        let median = {
            let mut sorted: Vec<u64> = history.iter().copied().filter(|v| *v > 0).collect();
            sorted.sort_unstable();
            sismo_analysis::percentile(&sorted, 0.5)
        };
        for (i, v) in history.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let bh = (*v as f32 / max as f32 * (h - 2.)).max(if *v > 0 { 1. } else { 0. });
            let spike = median > 0 && *v > median.saturating_mul(2);
            #[allow(clippy::cast_precision_loss)]
            let x = i as f32 * step;
            p.rect_alpha(
                x,
                h - bh,
                (step - 1.).max(1.),
                bh,
                if spike { theme::WARN } else { rgb },
                210,
            );
        }
    })
}

fn header(mut ws: State<Workspace>, sort: SortBy, allocations: bool) -> Rect {
    rect()
        .width(Size::fill())
        .height(Size::px(32.))
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((0., 16.))
        .background(theme::color(theme::BG1))
        .child(
            rect()
                .width(Size::flex(1.))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        v.sort = SortBy::Name;
                    }
                })
                .child(
                    text(
                        tr!("functions-col-function"),
                        11.,
                        if sort == SortBy::Name {
                            theme::TEXT
                        } else {
                            theme::TEXT3
                        },
                    )
                    .font_weight(600),
                ),
        )
        .children(columns(allocations).into_iter().map(|(title, key, width)| {
            let on = key.is_some_and(|k| k == sort);

            rect()
                .width(Size::px(width))
                .main_align(if title == HISTORY_COLUMN {
                    Alignment::Start
                } else {
                    Alignment::End
                })
                .horizontal()
                .padding((0., 0., 0., 12.))
                .maybe(key.is_some(), |r| r.cursor(CursorIcon::Pointer))
                .on_press(move |_| {
                    if let (Some(key), Some(v)) = (key, ws.write().active_mut()) {
                        v.sort = key;
                    }
                })
                .child(
                    text(
                        if on {
                            format!("{} ▼", tr!(title))
                        } else {
                            tr!(title)
                        },
                        11.,
                        if on { theme::TEXT } else { theme::TEXT3 },
                    )
                    .font_weight(600),
                )
                .into_element()
        }))
}

fn detail(mut ws: State<Workspace>, view: &SessionView, row: &FunctionStats) -> Rect {
    let session = &view.session;
    let rgb = theme::group(row.group);
    let worst: Vec<(u64, u64, u32)> = session
        .calls_of(row.callsite, view.window())
        .into_iter()
        .take(8)
        .collect();
    let history = row.history_p95_ns.clone();
    let window = view.window();
    let chart = scene(348., 120., move |p, w, h| {
        p.rect(0., 0., w, h, theme::BG0);
        let max = history.iter().copied().max().unwrap_or(1).max(1);
        #[allow(clippy::cast_precision_loss)]
        let step = w / history.len().saturating_sub(1).max(1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let points: Vec<(f32, f32)> = history
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    i as f32 * step,
                    h - 12. - *v as f32 / max as f32 * (h - 28.),
                )
            })
            .collect();
        p.polyline(&points, 2., rgb);
        p.text("p95", 6., 4., 10., rgb, true, 30.);
        p.text(
            &Nanos(max).to_string(),
            w - 70.,
            4.,
            10.,
            theme::TEXT3,
            true,
            66.,
        );
        p.text(
            &Clock(window.0).to_string(),
            4.,
            h - 12.,
            10.,
            theme::TEXT3,
            true,
            80.,
        );
        let end = Clock(window.1).to_string();
        p.text(&end, w - 64., h - 12., 10., theme::TEXT3, true, 64.);
    });
    let callsite = row.callsite;
    rect()
        .width(Size::px(380.))
        .height(Size::fill())
        .padding(16.)
        .spacing(10.)
        .background(theme::color(theme::BG1))
        .child(
            rect()
                .horizontal()
                .spacing(8.)
                .cross_align(Alignment::Center)
                .child(swatch(rgb, 10., 2.))
                .child(mono(row.name.clone(), 13., theme::TEXT).font_weight(600)),
        )
        .child(mono(
            format!(
                "{}:{} · {} ({})",
                row.file,
                row.line,
                row.krate,
                theme::group_label(row.group)
            ),
            11.,
            theme::TEXT3,
        ))
        .child(mono(
            tr!(
                "functions-detail-calls",
                count = row.calls.to_string(),
                p50 = Nanos(row.p50_ns).to_string(),
                p95 = Nanos(row.p95_ns).to_string(),
                max = Nanos(row.max_ns).to_string(),
            ),
            11.,
            theme::TEXT2,
        ))
        .maybe(view.session.allocations, |r| {
            let per_call = row.alloc_bytes / row.calls.max(1);
            r.child(mono(
                tr!(
                    "functions-detail-allocs",
                    own = sismo_analysis::units::Bytes(row.alloc_self_bytes).to_string(),
                    total = sismo_analysis::units::Bytes(row.alloc_bytes).to_string(),
                    per_call = sismo_analysis::units::Bytes(per_call).to_string(),
                    count = row.alloc_count.to_string(),
                ),
                11.,
                theme::TEXT2,
            ))
        })
        .child(caption(&tr!("functions-history-caption")))
        .child(
            rect()
                .width(Size::px(348.))
                .height(Size::px(120.))
                .corner_radius(4.)
                .background(chart),
        )
        .child(caption(&tr!("functions-worst-calls")))
        .children(worst.into_iter().map(|(duration, start, thread)| {
            let frame = session.frames.partition_point(|f| f.end <= start);
            let frame_index = session
                .frames
                .get(frame)
                .filter(|f| f.start <= start)
                .map(|f| f.index);
            let thread_name = session.thread_name(thread).to_owned();
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .spacing(10.)
                .padding((6., 8.))
                .corner_radius(3.)
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        if let Some(index) = frame_index {
                            focus_frame(v, index);
                        } else {
                            v.view = (start.saturating_sub(duration), start + duration * 2);
                            v.follow = false;
                        }
                        v.tab = Tab::Timeline;
                    }
                })
                .child(mono(Clock(start).to_string(), 12., theme::TEXT2))
                .child(mono(Nanos(duration).to_string(), 12., theme::TEXT).width(Size::flex(1.)))
                .child(mono(
                    frame_index.map_or_else(|| thread_name.clone(), |i| format!("frame #{i} ↗")),
                    12.,
                    theme::TEXT3,
                ))
                .into_element()
        }))
        .child(text(tr!("functions-worst-hint"), 11., theme::TEXT3))
        .child(button(
            tr!("functions-filter-to"),
            Kind::Secondary,
            move |_| {
                let mut w = ws.write();
                let Some(v) = w.active_mut() else { return };
                let name = v.session.name(callsite).to_owned();
                v.set_query(format!("fn:{name}"));
            },
        ))
}

pub fn view(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    if let Some((path, other)) = &view.compare {
        return compare_view(ws, view, path, other);
    }
    let session = &view.session;
    let query = view.effective_query();
    let rows = session.functions(&query, view.window(), BUCKETS, view.sort);
    let total_functions = session
        .callsites()
        .filter(|c| c.kind == sismo_format::CallsiteKind::Span)
        .count();
    let selected = view.selected_function;
    let allocations = view.session.allocations;
    let detail_row = selected.and_then(|id| rows.iter().find(|r| r.callsite == id).cloned());
    let count = rows.len();

    let list = VirtualScrollView::new_with_data(
        (rows, selected, allocations),
        move |item, (rows, selected, allocations): &(Vec<FunctionStats>, Option<u32>, bool)| {
            let selected = *selected;
            let allocations = *allocations;
            let cols = columns(allocations);
            let Some(row) = rows.get(item.index) else {
                return rect().into_element();
            };
            let rgb = theme::group(row.group);
            let callsite = row.callsite;
            let on = selected == Some(callsite);
            rect()
                .key(callsite)
                .width(Size::fill())
                .height(Size::px(ROW_H))
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((0., 16.))
                .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        v.selected_function = Some(callsite);
                    }
                })
                .child(
                    rect()
                        .width(Size::flex(1.))
                        .horizontal()
                        .spacing(8.)
                        .cross_align(Alignment::Center)
                        .child(swatch(rgb, 6., 1.))
                        .child(mono(row.name.clone(), 12., theme::TEXT)),
                )
                .children(
                    cells(row, allocations)
                        .into_iter()
                        .enumerate()
                        .map(|(i, value)| {
                            rect()
                                .width(Size::px(cols[i].2))
                                .horizontal()
                                .main_align(Alignment::End)
                                .child(mono(
                                    value,
                                    12.,
                                    if i == 2 { theme::TEXT } else { theme::TEXT2 },
                                ))
                                .into_element()
                        }),
                )
                .child(
                    rect()
                        .width(Size::px(176.))
                        .padding((0., 0., 0., 16.))
                        .child(
                            rect()
                                .width(Size::px(160.))
                                .height(Size::px(20.))
                                .background(sparkline(&row.history_p95_ns, rgb)),
                        ),
                )
                .into_element()
        },
    )
    .length(count)
    .item_size(ROW_H)
    .height(Size::flex(1.));

    let empty = (count == 0).then(|| {
        rect()
            .width(Size::fill())
            .padding(24.)
            .spacing(8.)
            .child(
                text(
                    tr!("functions-empty-title", query = view.query_text.as_str()),
                    13.,
                    theme::TEXT,
                )
                .font_weight(600),
            )
            .child(text(tr!("functions-empty-body"), 12., theme::TEXT2))
            .child(button(
                tr!("functions-clear-query"),
                Kind::Secondary,
                move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        v.set_query(String::new());
                    }
                },
            ))
    });

    rect()
        .expanded()
        .horizontal()
        .content(Content::flex())
        .child(
            rect()
                .width(Size::flex(1.))
                .height(Size::fill())
                .content(Content::flex())
                .child(header(ws, view.sort, view.session.allocations))
                .child(hline())
                .maybe_child(empty)
                .child(list)
                .child(hline())
                .child(rect().padding((8., 16.)).child(text(
                    tr!(
                        "functions-footer",
                        count = count.to_string(),
                        total = total_functions.to_string(),
                    ),
                    11.,
                    theme::TEXT3,
                ))),
        )
        .maybe(detail_row.is_some(), |r| r.child(vline()))
        .maybe_child(detail_row.map(|row| detail(ws, view, &row)))
}

fn compare_view(
    mut ws: State<Workspace>,
    view: &SessionView,
    path: &std::path::Path,
    other: &sismo_analysis::Session,
) -> Rect {
    use sismo_analysis::Metric;
    let metric = view.compare_metric;
    let rows = sismo_analysis::compare(other, &view.session, &view.effective_query(), metric);
    let show = move |v: Option<u64>| match (v, metric) {
        (None, _) => "—".to_owned(),
        (Some(v), Metric::Calls) => v.to_string(),
        (Some(v), _) => Nanos(v).to_string(),
    };
    let max_percent = rows
        .iter()
        .filter_map(|r| r.percent)
        .map(f64::abs)
        .fold(1., f64::max)
        .min(300.);
    let metrics = [Metric::P95, Metric::P50, Metric::SelfTime, Metric::Calls];
    let header = rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((10., 16.))
        .spacing(12.)
        .background(theme::color(theme::BG1))
        .child(
            rect()
                .width(Size::flex(1.))
                .spacing(4.)
                .child(mono(format!("A  {}", path.display()), 12., theme::TEXT))
                .child(mono(format!("B  {}", view.title()), 12., theme::TEXT)),
        )
        .children(metrics.iter().map(|m| {
            let m = *m;
            let on = m == metric;
            rect()
                .padding((5., 12.))
                .corner_radius(4.)
                .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        v.compare_metric = m;
                    }
                })
                .child(text(
                    m.label(),
                    12.,
                    if on { theme::TEXT } else { theme::TEXT2 },
                ))
                .into_element()
        }))
        .child(button(
            tr!("functions-remove-compare"),
            Kind::Secondary,
            move |_| {
                if let Some(v) = ws.write().active_mut() {
                    v.compare = None;
                }
            },
        ));
    let count = rows.len();
    let list = VirtualScrollView::new_with_data(
        (rows, metric),
        move |item, (rows, _): &(Vec<sismo_analysis::CompareRow>, Metric)| {
            let Some(row) = rows.get(item.index) else {
                return rect().into_element();
            };
            let worse = row.delta.is_some_and(|d| d > 0) && !row.is_noise();
            let better = row.delta.is_some_and(|d| d < 0) && !row.is_noise();
            let rgb = if worse {
                theme::ERROR
            } else if better {
                theme::FRAMEWORK
            } else {
                theme::TEXT3
            };
            let delta = match row.delta {
                None if row.a.is_none() => tr!("functions-compare-new"),
                None => tr!("functions-compare-gone"),
                Some(d) if metric == Metric::Calls => format!("{d:+}"),
                Some(d) => format!(
                    "{}{}",
                    if d < 0 { "-" } else { "+" },
                    Nanos(d.unsigned_abs())
                ),
            };
            let percent = row
                .percent
                .map_or_else(String::new, |p| format!("{p:+.0}%"));
            let bar = row
                .percent
                .map_or(0., |p| (p.abs() / max_percent * 300.).min(300.))
                as f32;
            rect()
                .key(item.index)
                .width(Size::fill())
                .height(Size::px(ROW_H))
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((0., 16.))
                .spacing(12.)
                .child(swatch(theme::group(row.group), 6., 1.))
                .child(mono(row.name.clone(), 12., theme::TEXT).width(Size::flex(1.)))
                .child(
                    rect()
                        .width(Size::px(90.))
                        .horizontal()
                        .main_align(Alignment::End)
                        .child(mono(show(row.a), 12., theme::TEXT2)),
                )
                .child(
                    rect()
                        .width(Size::px(90.))
                        .horizontal()
                        .main_align(Alignment::End)
                        .child(mono(show(row.b), 12., theme::TEXT2)),
                )
                .child(
                    rect()
                        .width(Size::px(90.))
                        .horizontal()
                        .main_align(Alignment::End)
                        .child(mono(delta, 12., rgb)),
                )
                .child(
                    rect()
                        .width(Size::px(60.))
                        .horizontal()
                        .main_align(Alignment::End)
                        .child(mono(percent, 12., rgb)),
                )
                .child(
                    rect()
                        .width(Size::px(310.))
                        .padding((0., 0., 0., 10.))
                        .child(
                            rect()
                                .width(Size::px(bar.max(1.)))
                                .height(Size::px(8.))
                                .corner_radius(1.)
                                .background(theme::color(rgb)),
                        ),
                )
                .into_element()
        },
    )
    .length(count)
    .item_size(ROW_H)
    .height(Size::flex(1.));
    rect()
        .expanded()
        .content(Content::flex())
        .child(header)
        .child(hline())
        .child(list)
        .child(hline())
        .child(rect().padding((8., 16.)).child(text(
            tr!("functions-compare-footer", count = count.to_string()),
            11.,
            theme::TEXT3,
        )))
}

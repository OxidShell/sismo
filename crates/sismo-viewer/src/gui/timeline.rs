//! Timeline tab: frame strip, ruler with log marks, per-thread span lanes
//! and the selected span's detail.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use freya::prelude::*;
use sismo_analysis::units::{Clock, Nanos};
use sismo_analysis::{FrameSummary, Group, GroupSplit, Query, SLOW_FRAME_NS, Session, percentile};

use super::Workspace;
use super::draw::Scene;
use super::i18n::tr;
use super::theme::{self, Rgb};
use super::widgets::{Kind, button, caption, hline, mono, swatch, text};
use crate::workspace::{SessionView, Tab};

const STRIP_H: f32 = 150.;
const RULER_H: f32 = 28.;
const ROW_H: f32 = 20.;
const LANE_GAP: f32 = 22.;
const PAD: f32 = 16.;

thread_local! {
    /// Per session: cached group split of every frame already measured.
    static SPLITS: RefCell<HashMap<u64, Vec<GroupSplit>>> = RefCell::new(HashMap::new());
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// Frame summary of the whole session, keyed by session and frame count.
    static SUMMARY: RefCell<Option<((u64, usize), FrameSummary)>> = const { RefCell::new(None) };
    /// Sorted call durations and child count of the selected span.
    static DETAIL: RefCell<Option<(DetailKey, Rc<SpanStats>)>> = const { RefCell::new(None) };
}

/// Session, thread, span index and event count of a [`SpanStats`].
type DetailKey = (u64, u32, u32, u64);

/// What the detail panel shows about the selected span's function.
struct SpanStats {
    durations: Vec<u64>,
    children: usize,
}

fn frame_summary(view: &SessionView) -> FrameSummary {
    let key = (view.id, view.session.frames.len());
    SUMMARY.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((cached, summary)) = slot.as_ref()
            && *cached == key
        {
            return *summary;
        }
        let summary = view.session.frame_summary(view.session.span());
        *slot = Some((key, summary));
        summary
    })
}

fn span_stats(view: &SessionView, thread: &sismo_analysis::Thread, index: u32) -> Rc<SpanStats> {
    let key = (view.id, thread.id, index, view.session.events);
    DETAIL.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((cached, stats)) = slot.as_ref()
            && *cached == key
        {
            return Rc::clone(stats);
        }
        let session = &view.session;
        let callsite = thread.spans[index as usize].callsite;
        let mut durations: Vec<u64> = session
            .calls_of(callsite, session.span())
            .iter()
            .map(|c| c.0)
            .collect();
        durations.sort_unstable();
        let children = thread
            .spans
            .iter()
            .skip(index as usize + 1)
            .filter(|s| s.parent == Some(index))
            .count();
        let stats = Rc::new(SpanStats {
            durations,
            children,
        });
        *slot = Some((key, Rc::clone(&stats)));
        stats
    })
}

#[derive(Debug, Clone, Copy)]
enum DragKind {
    Pan { view: (u64, u64) },
    Select,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    kind: DragKind,
    origin_x: f64,
    origin_t: u64,
    moved: bool,
}

fn frame_splits(view: &SessionView) -> Vec<GroupSplit> {
    SPLITS.with(|cell| {
        let mut map = cell.borrow_mut();
        let splits = map.entry(view.id).or_default();
        let frames = &view.session.frames;
        // The last frame of a live session may still receive spans; refresh it.
        let keep = splits.len().min(frames.len().saturating_sub(1));
        splits.truncate(keep);
        for frame in &frames[keep..] {
            splits.push(view.session.frame_split(frame));
        }
        splits.clone()
    })
}

/// Maps time to x within a canvas of `width`.
#[derive(Debug, Clone, Copy)]
struct Scale {
    from: u64,
    to: u64,
    left: f32,
    width: f32,
}

impl Scale {
    #[allow(clippy::cast_precision_loss)]
    fn x(self, t: u64) -> f32 {
        let span = self.to.saturating_sub(self.from).max(1) as f64;
        let offset = t as f64 - self.from as f64;
        #[allow(clippy::cast_possible_truncation)]
        let x = (offset / span * f64::from(self.width)) as f32;
        self.left + x
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn t(self, x: f64) -> u64 {
        let span = self.to.saturating_sub(self.from) as f64;
        let ratio = ((x - f64::from(self.left)) / f64::from(self.width)).clamp(0., 1.);
        self.from + (span * ratio) as u64
    }
}

/// Vertical placement of one thread lane.
#[derive(Debug, Clone)]
struct Lane {
    thread: u32,
    name: String,
    top: f32,
    rows: u16,
}

fn lanes(view: &SessionView) -> (Vec<Lane>, f32) {
    let mut y = 12.;
    let mut out = Vec::new();
    let (from, to) = view.view;
    for thread in &view.session.threads {
        if view.hidden_threads.contains(&thread.id) {
            continue;
        }
        let rows = thread
            .spans_in(from, to)
            .map(|s| s.depth)
            .max()
            .map_or(0, |d| d + 1);
        if rows == 0 {
            continue;
        }
        out.push(Lane {
            thread: thread.id,
            name: thread.name.clone(),
            top: y + 18.,
            rows,
        });
        y += 18. + f32::from(rows) * ROW_H + LANE_GAP;
    }
    (out, y.max(120.))
}

fn group_color(session: &Session, callsite: u32) -> Rgb {
    theme::group(session.group_of(callsite))
}

fn draw_strip(p: &mut Scene, view: &SessionView, splits: &[GroupSplit], w: f32, h: f32) {
    let session = &view.session;
    let frames = &session.frames;
    p.rect(0., 0., w, h, theme::BG0);
    if frames.is_empty() {
        p.text(
            &tr!("timeline-no-frames"),
            PAD,
            h / 2. - 8.,
            12.,
            theme::TEXT3,
            false,
            w,
        );
        return;
    }
    let base = h - 14.;
    let max_ms = frames
        .iter()
        .map(sismo_analysis::Frame::duration)
        .max()
        .map_or(20., |ns| (ns as f64 / 1e6).max(20.) as f32)
        .min(80.);
    let scale_y = (base - 30.) / max_ms;
    let left = 46.;
    let width = w - left - PAD;
    let budget_y = base - 16.6 * scale_y;
    let half_y = base - 8.3 * scale_y;
    p.line((left, budget_y), (w - PAD, budget_y), 1., theme::ERROR, 120);
    p.text("16.6", 12., budget_y - 7., 10., theme::ERROR, true, 40.);
    p.line((left, half_y), (w - PAD, half_y), 1., theme::TEXT3, 80);
    p.text("8.3", 18., half_y - 7., 10., theme::TEXT3, true, 40.);
    let count = frames.len();
    #[allow(clippy::cast_precision_loss)]
    let per = (width / count as f32).max(0.1);
    let bar = (per - if per > 4. { 2. } else { 0. }).max(0.5);
    let selected = view.selected_frame;
    // Many frames: draw at most one bar per pixel, keeping the slowest.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = (width.max(1.)) as usize;
    let step = count.div_ceil(columns.max(1)).max(1);
    let mut i = 0;
    while i < count {
        let end = (i + step).min(count);
        let worst = (i..end).max_by_key(|j| frames[*j].duration()).unwrap_or(i);
        let frame = &frames[worst];
        let split = splits.get(worst).copied().unwrap_or_default();
        #[allow(clippy::cast_precision_loss)]
        let x = left + i as f32 * per;
        let total_ms = frame.duration() as f32 / 1e6;
        let tracked = split.total().max(1) as f32;
        let mut y = base;
        let total_h = (total_ms * scale_y).min(base - 4.);
        let untracked = (1. - (tracked / frame.duration().max(1) as f32)).max(0.);
        for (group, ns) in [
            (Group::Other, split.other_ns),
            (Group::Framework, split.framework_ns),
            (Group::App, split.app_ns),
        ] {
            let seg = total_h * (1. - untracked) * (ns as f32 / tracked);
            if seg > 0. {
                p.rect(x, y - seg, bar, seg, theme::group(group));
                y -= seg;
            }
        }
        if y > base - total_h {
            p.rect(x, base - total_h, bar, y - (base - total_h), theme::BG2);
        }
        if selected.is_some_and(|s| (i..end).any(|j| frames[j].index == s)) {
            p.rect(x - 1., base + 3., bar + 2., 3., theme::TEXT);
        }
        i = end;
    }
    let (start, end) = session.span();
    let scale = Scale {
        from: frames[0].start.min(start),
        to: frames[count - 1].end.max(end),
        left,
        width,
    };
    let (vf, vt) = view.selection.unwrap_or(view.view);
    let (x0, x1) = (scale.x(vf), scale.x(vt));
    p.rect_alpha(x0, 24., (x1 - x0).max(2.), base - 22., theme::TEXT, 18);
    p.outline(
        x0,
        24.,
        (x1 - x0).max(2.),
        base - 22.,
        1.,
        if view.selection.is_some() {
            theme::TEXT
        } else {
            theme::TEXT3
        },
    );
}

fn strip_scale(view: &SessionView, w: f32) -> Option<Scale> {
    let frames = &view.session.frames;
    let first = frames.first()?;
    let last = frames.last()?;
    let (start, end) = view.session.span();
    Some(Scale {
        from: first.start.min(start),
        to: last.end.max(end),
        left: 46.,
        width: w - 46. - PAD,
    })
}

fn draw_ruler(p: &mut Scene, view: &SessionView, w: f32) {
    p.rect(0., 0., w, RULER_H, theme::BG1);
    p.rect(0., RULER_H - 1., w, 1., theme::LINE);
    let scale = Scale {
        from: view.view.0,
        to: view.view.1,
        left: PAD,
        width: w - 2. * PAD,
    };
    let span = view.view.1.saturating_sub(view.view.0);
    for i in 0..10u64 {
        let t = view.view.0 + span * i / 10;
        let x = scale.x(t);
        p.rect(x, 18., 1., 9., theme::LINE2);
        p.text(
            &format!("+{}", Nanos(t - view.view.0)),
            x + 4.,
            13.,
            10.,
            theme::TEXT3,
            true,
            90.,
        );
    }
    let session = &view.session;
    let first = session.logs.partition_point(|l| l.t < view.view.0);
    for log in session.logs[first..]
        .iter()
        .take_while(|l| l.t < view.view.1)
    {
        if log.level > sismo_format::Level::Info {
            continue;
        }
        let x = scale.x(log.t);
        p.rect(x - 3., 4., 7., 7., theme::level(log.level));
    }
    for marker in session
        .markers
        .iter()
        .filter(|m| m.t >= view.view.0 && m.t < view.view.1)
    {
        let x = scale.x(marker.t);
        p.rect(x, 0., 1., RULER_H, theme::TEXT);
        p.text(&marker.note, x + 4., 2., 10., theme::TEXT, false, 160.);
    }
    for panic in session
        .panics
        .iter()
        .filter(|m| m.t >= view.view.0 && m.t < view.view.1)
    {
        let x = scale.x(panic.t);
        p.rect(x, 0., 2., RULER_H, theme::ERROR);
    }
}

fn draw_lanes(p: &mut Scene, view: &SessionView, query: &Query, lanes: &[Lane], w: f32, h: f32) {
    p.rect(0., 0., w, h, theme::BG0);
    let session = &view.session;
    let scale = Scale {
        from: view.view.0,
        to: view.view.1,
        left: PAD,
        width: w - 2. * PAD,
    };
    let filtering = !query.groups.is_empty();
    for lane in lanes {
        p.text(
            &lane.name,
            PAD,
            lane.top - 17.,
            11.,
            theme::TEXT2,
            false,
            300.,
        );
        let Some(thread) = session.thread(lane.thread) else {
            continue;
        };
        let mut last_px: Vec<f32> = vec![f32::MIN; usize::from(lane.rows)];
        for (index, span) in thread
            .indices_in(view.view.0, view.view.1)
            .into_iter()
            .map(|i| (i, &thread.spans[i as usize]))
        {
            let x0 = scale.x(span.start).max(PAD);
            let x1 = scale.x(span.end.min(session.end)).min(w - PAD);
            let row = usize::from(span.depth);
            let width = x1 - x0;
            if width < 1. && last_px.get(row).is_some_and(|l| (x0 - *l).abs() < 1.) {
                continue;
            }
            if let Some(slot) = last_px.get_mut(row) {
                *slot = x0;
            }
            let y = lane.top + f32::from(span.depth) * ROW_H;
            let rgb = group_color(session, span.callsite);
            let matches = !filtering || query.matches_span(session, &thread.name, span);
            if matches {
                p.rect(x0, y, (width - 1.).max(1.), ROW_H - 2., rgb);
            } else {
                p.rect_alpha(x0, y, (width - 1.).max(1.), ROW_H - 2., rgb, 50);
            }
            if view.selected_span == Some((lane.thread, index)) {
                p.outline(x0, y, width.max(2.), ROW_H - 2., 2., theme::TEXT);
            }
            if width > 60. {
                let label = format!(
                    "{}  {}",
                    session.name(span.callsite),
                    Nanos(span.duration(session.end))
                );
                p.text(
                    &label,
                    x0 + 6.,
                    y + 3.,
                    11.,
                    if matches { theme::BG0 } else { theme::TEXT3 },
                    true,
                    width - 12.,
                );
            }
        }
    }
}

fn hit_span(view: &SessionView, lanes: &[Lane], w: f32, x: f64, y: f64) -> Option<(u32, u32)> {
    let scale = Scale {
        from: view.view.0,
        to: view.view.1,
        left: PAD,
        width: w - 2. * PAD,
    };
    let t = scale.t(x);
    #[allow(clippy::cast_possible_truncation)]
    let y = y as f32;
    let lane = lanes
        .iter()
        .find(|l| y >= l.top && y < l.top + f32::from(l.rows) * ROW_H)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let depth = ((y - lane.top) / ROW_H) as u16;
    let thread = view.session.thread(lane.thread)?;
    // Allow a 3 px slop so thin spans stay clickable.
    let slop = scale.t(x + 3.).saturating_sub(t);
    thread
        .indices_in(t.saturating_sub(slop), t + slop + 1)
        .into_iter()
        .rev()
        .find(|i| thread.spans[*i as usize].depth == depth)
        .map(|i| (lane.thread, i))
}

/// Zooms the visible range by `factor` around `t`.
fn zoom(view: &mut SessionView, t: u64, factor: f64) {
    let (from, to) = view.view;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let scaled = |d: u64| (d as f64 * factor) as u64;
    let new_from = t.saturating_sub(scaled(t.saturating_sub(from)));
    let new_to = t + scaled(to.saturating_sub(t));
    if new_to.saturating_sub(new_from) >= 10_000 {
        view.view = (new_from, new_to);
        view.follow = false;
        view.revision += 1;
    }
}

fn pan(view: &mut SessionView, delta: i64) {
    let (from, to) = view.view;
    let (start, end) = view.session.span();
    let width = to - from;
    let new_from = if delta < 0 {
        from.saturating_sub(delta.unsigned_abs()).max(start)
    } else {
        (from + delta.unsigned_abs()).min(end.saturating_sub(width / 2))
    };
    view.view = (new_from, new_from + width);
    view.follow = false;
    view.revision += 1;
}

/// Whether a pan or range selection is being dragged right now.
pub fn dragging() -> bool {
    DRAG.with(|d| d.borrow().is_some_and(|d| d.moved))
}

pub fn keyboard_nav(view: &mut SessionView, key: &str) {
    if view.tab != Tab::Timeline {
        return;
    }
    let (from, to) = view.view;
    let center = from + (to - from) / 2;
    let step = i64::try_from((to - from) / 5).unwrap_or(i64::MAX);
    match key {
        "w" => zoom(view, center, 0.7),
        "s" => zoom(view, center, 1.4),
        "a" => pan(view, -step),
        "d" => pan(view, step),
        _ => {}
    }
}

/// Jumps to the next frame over budget after the selected one.
pub fn next_slow_frame(view: &mut SessionView) {
    let after = view.selected_frame.map_or(0, |i| i + 1);
    if let Some(frame) = view
        .session
        .frames
        .iter()
        .find(|f| f.index >= after && f.duration() > SLOW_FRAME_NS)
        .copied()
    {
        focus_frame(view, frame.index);
    }
}

pub fn focus_frame(view: &mut SessionView, index: u32) {
    if let Some(frame) = view.session.frames.get(index as usize).copied() {
        let pad = frame.duration() / 20;
        view.selected_frame = Some(frame.index);
        view.view = (frame.start.saturating_sub(pad), frame.end + pad);
        view.selection = Some((frame.start, frame.end));
        view.follow = false;
        view.revision += 1;
    }
}

/// Records a scene of `w × h` and returns its paint callback.
pub fn scene(w: f32, h: f32, paint: impl FnOnce(&mut Scene, f32, f32)) -> RenderCallback {
    let mut scene = Scene::default();
    paint(&mut scene, w, h);
    scene.callback()
}

fn detail(mut ws: State<Workspace>, view: &SessionView) -> Option<Rect> {
    let (thread_id, index) = view.selected_span?;
    let session = &view.session;
    let thread = session.thread(thread_id)?;
    let span = *thread.spans.get(index as usize)?;
    let def = session.callsite(span.callsite);
    let stats = span_stats(view, thread, index);
    let durations = &stats.durations;
    let children = stats.children;
    let p50 = percentile(durations, 0.5);
    let p95 = percentile(durations, 0.95);
    let max = durations.last().copied().unwrap_or(0);
    let bins = 24usize;
    let mut hist = vec![0u32; bins];
    for d in durations {
        let b = usize::try_from(d * (bins as u64 - 1) / max.max(1))
            .unwrap_or(0)
            .min(bins - 1);
        hist[b] += 1;
    }
    let this_bin = usize::try_from(span.duration(session.end) * (bins as u64 - 1) / max.max(1))
        .unwrap_or(0)
        .min(bins - 1);
    let rgb = group_color(session, span.callsite);
    let callsite = span.callsite;
    let kv = |k: &str, v: String| {
        rect()
            .width(Size::px(170.))
            .spacing(4.)
            .child(text(k, 11., theme::TEXT3))
            .child(mono(v, 13., theme::TEXT))
            .into_element()
    };
    Some(
        rect()
            .width(Size::fill())
            .height(Size::px(178.))
            .horizontal()
            .content(Content::flex())
            .padding((14., 16.))
            .spacing(24.)
            .background(theme::color(theme::BG1))
            .child(
                rect()
                    .width(Size::flex(1.))
                    .spacing(8.)
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::flex())
                            .spacing(8.)
                            .cross_align(Alignment::Center)
                            .child(swatch(rgb, 10., 2.))
                            .child(mono(session.name(callsite), 15., theme::TEXT).font_weight(600)),
                    )
                    .child(mono(
                        tr!(
                            "timeline-span-location",
                            file = def.map_or("?", |d| d.file.as_str()),
                            line = def.map_or(0, |d| d.line).to_string(),
                            krate = session.crate_of(callsite),
                            group = theme::group_label(session.group_of(callsite)),
                            thread = thread.name.as_str(),
                        ),
                        12.,
                        theme::TEXT3,
                    ))
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::flex())
                            .padding((6., 0.))
                            .child(kv(
                                &tr!("timeline-duration"),
                                Nanos(span.duration(session.end)).to_string(),
                            ))
                            .child(kv(
                                &tr!("timeline-self-time"),
                                Nanos(span.self_ns).to_string(),
                            ))
                            .child(kv(&tr!("timeline-children"), children.to_string()))
                            .child(kv(&tr!("timeline-start"), Clock(span.start).to_string()))
                            .maybe(session.allocations, |r| {
                                r.child(kv(
                                    &tr!("timeline-allocs"),
                                    format!(
                                        "{} · {}",
                                        sismo_analysis::units::Bytes(span.alloc_bytes),
                                        span.alloc_count
                                    ),
                                ))
                            }),
                    )
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::flex())
                            .spacing(8.)
                            .child(button(
                                tr!("common-show-in-functions"),
                                Kind::Secondary,
                                move |_| {
                                    if let Some(v) = ws.write().active_mut() {
                                        v.selected_function = Some(callsite);
                                        v.tab = Tab::Functions;
                                    }
                                },
                            ))
                            .child(button(
                                tr!("timeline-open-in-flamegraph"),
                                Kind::Secondary,
                                move |_| {
                                    if let Some(v) = ws.write().active_mut() {
                                        v.tab = Tab::Flamegraph;
                                    }
                                },
                            )),
                    ),
            )
            .child(
                rect()
                    .width(Size::px(380.))
                    .spacing(6.)
                    .child(text(
                        tr!(
                            "timeline-session-calls",
                            count = durations.len().to_string(),
                            p50 = Nanos(p50).to_string(),
                            p95 = Nanos(p95).to_string(),
                            max = Nanos(max).to_string(),
                        ),
                        11.,
                        theme::TEXT3,
                    ))
                    .child(
                        rect()
                            .width(Size::px(380.))
                            .height(Size::px(110.))
                            .background(scene(380., 110., move |p, w, h| {
                                let top = hist.iter().copied().max().unwrap_or(1).max(1) as f32;
                                #[allow(clippy::cast_precision_loss)]
                                let bw = w / bins as f32;
                                for (i, v) in hist.iter().enumerate() {
                                    #[allow(clippy::cast_precision_loss)]
                                    let bh = (*v as f32 / top) * (h - 16.);
                                    #[allow(clippy::cast_precision_loss)]
                                    let x = i as f32 * bw;
                                    p.rect(
                                        x,
                                        h - 14. - bh.max(1.),
                                        bw - 3.,
                                        bh.max(1.),
                                        if i == this_bin { theme::TEXT } else { rgb },
                                    );
                                }
                                p.text("0", 0., h - 12., 10., theme::TEXT3, true, 30.);
                                let label = Nanos(max).to_string();
                                let lw = super::draw::measure(&label, 10., true);
                                p.text(&label, w - lw, h - 12., 10., theme::TEXT3, true, lw + 2.);
                            })),
                    ),
            ),
    )
}

pub fn view(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let query = view.effective_query();
    let splits = frame_splits(view);
    let summary = frame_summary(view);
    let strip_w = view.canvas.get("strip").map_or(1000., |s| s.0);
    let lanes_w = view.canvas.get("lanes").map_or(1000., |s| s.0);
    let (lane_list, lanes_h) = lanes(view);

    let strip_paint = scene(strip_w, STRIP_H, |p, w, h| {
        draw_strip(p, view, &splits, w, h);
    });
    let ruler_paint = scene(lanes_w, RULER_H, |p, w, _| draw_ruler(p, view, w));
    let lanes_paint = scene(lanes_w, lanes_h, |p, w, h| {
        draw_lanes(p, view, &query, &lane_list, w, h);
    });

    let frames_header = rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .spacing(16.)
        .padding((10., 16., 0., 16.))
        .child(text(tr!("timeline-frames"), 13., theme::TEXT).font_weight(600))
        .child(mono(
            tr!(
                "timeline-frames-summary",
                count = summary.count.to_string(),
                p50 = Nanos(summary.p50_ns).to_string(),
                p95 = Nanos(summary.p95_ns).to_string(),
                slow = summary.slow.to_string(),
            ),
            12.,
            theme::TEXT2,
        ))
        .child(rect().width(Size::flex(1.)))
        .children(Group::ALL.iter().map(|g| {
            rect()
                .horizontal()
                .content(Content::flex())
                .spacing(6.)
                .cross_align(Alignment::Center)
                .child(swatch(theme::group(*g), 8., 1.))
                .child(text(theme::group_label(*g), 11., theme::TEXT2))
                .into_element()
        }))
        .child(text(tr!("timeline-hint"), 11., theme::TEXT3));

    let strip = rect()
        .width(Size::fill())
        .height(Size::px(STRIP_H))
        .on_sized(move |e: Event<SizedEventData>| {
            let size = (e.area.width(), e.area.height());
            if let Some(v) = ws.write().active_mut() {
                v.canvas.insert("strip", size);
            }
        })
        .background(strip_paint)
        .on_pointer_down(move |e: Event<PointerEventData>| {
            let x = e.element_location().x;
            let w = ws.peek();
            let Some(scale) = w.active().and_then(|v| strip_scale(v, strip_w)) else {
                return;
            };
            let t = scale.t(x);
            drop(w);
            DRAG.with(|d| {
                *d.borrow_mut() = Some(Drag {
                    kind: DragKind::Select,
                    origin_x: x,
                    origin_t: t,
                    moved: false,
                });
            });
        })
        .on_pointer_move(move |e: Event<PointerEventData>| {
            let Some(drag) = DRAG.with(|d| *d.borrow()) else {
                return;
            };
            if !matches!(drag.kind, DragKind::Select) {
                return;
            }
            let x = e.element_location().x;
            if (x - drag.origin_x).abs() < 3. {
                return;
            }
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let Some(scale) = strip_scale(v, strip_w) else {
                return;
            };
            let t = scale.t(x);
            let range = (drag.origin_t.min(t), drag.origin_t.max(t));
            v.selection = Some(range);
            v.view = range;
            v.follow = false;
            v.revision += 1;
            DRAG.with(|d| {
                *d.borrow_mut() = Some(Drag {
                    moved: true,
                    ..drag
                });
            });
        })
        .on_pointer_press(move |_| {
            let Some(drag) = DRAG.with(|d| d.borrow_mut().take()) else {
                return;
            };
            if drag.moved {
                return;
            }
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let index = v.session.frames.partition_point(|f| f.end <= drag.origin_t);
            if let Some(frame) = v.session.frames.get(index) {
                let frame_index = frame.index;
                focus_frame(v, frame_index);
            }
        });

    let ruler = rect()
        .width(Size::fill())
        .height(Size::px(RULER_H))
        .background(ruler_paint);

    let lanes_el = rect()
        .width(Size::fill())
        .height(Size::px(lanes_h))
        .on_sized(move |e: Event<SizedEventData>| {
            let size = (e.area.width(), e.area.height());
            if let Some(v) = ws.write().active_mut() {
                v.canvas.insert("lanes", size);
            }
        })
        .background(lanes_paint)
        .on_mouse_down(move |e: Event<MouseEventData>| {
            if e.button != Some(MouseButton::Right) {
                return;
            }
            let (x, y) = (e.element_location.x, e.element_location.y);
            let global = e.global_location;
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let (lane_list, _) = lanes(v);
            let Some((thread, index)) = hit_span(v, &lane_list, lanes_w, x, y) else {
                return;
            };
            let Some(callsite) = v
                .session
                .thread(thread)
                .and_then(|t| t.spans.get(index as usize))
                .map(|s| s.callsite)
            else {
                return;
            };
            v.selected_span = Some((thread, index));
            v.selected_function = Some(callsite);
            w.menu = Some(super::context_menu::MenuTarget {
                callsite,
                thread: Some(thread),
                span: Some(index),
                x: global.x,
                y: global.y,
            });
        })
        .on_wheel(move |e: Event<WheelEventData>| {
            if e.delta_y.abs() < f64::EPSILON {
                return;
            }
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let scale = Scale {
                from: v.view.0,
                to: v.view.1,
                left: PAD,
                width: lanes_w - 2. * PAD,
            };
            let t = scale.t(e.element_location.x);
            zoom(v, t, if e.delta_y > 0. { 0.8 } else { 1.25 });
        })
        .on_pointer_down(move |e: Event<PointerEventData>| {
            let x = e.element_location().x;
            let view = ws.peek().active().map(|v| v.view);
            let Some(view) = view else { return };
            DRAG.with(|d| {
                *d.borrow_mut() = Some(Drag {
                    kind: DragKind::Pan { view },
                    origin_x: x,
                    origin_t: 0,
                    moved: false,
                });
            });
        })
        .on_pointer_move(move |e: Event<PointerEventData>| {
            let Some(drag) = DRAG.with(|d| *d.borrow()) else {
                return;
            };
            let DragKind::Pan { view: start } = drag.kind else {
                return;
            };
            let dx = e.element_location().x - drag.origin_x;
            if dx.abs() < 3. && !drag.moved {
                return;
            }
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let span = start.1 - start.0;
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let shift = (dx / f64::from(lanes_w - 2. * PAD) * span as f64) as i64;
            let from = if shift > 0 {
                start.0.saturating_sub(shift.unsigned_abs())
            } else {
                start.0 + shift.unsigned_abs()
            };
            v.view = (from, from + span);
            v.follow = false;
            v.revision += 1;
            DRAG.with(|d| {
                *d.borrow_mut() = Some(Drag {
                    moved: true,
                    ..drag
                });
            });
        })
        .on_pointer_press(move |e: Event<PointerEventData>| {
            let moved = DRAG
                .with(|d| d.borrow_mut().take())
                .is_some_and(|d| d.moved);
            if moved {
                return;
            }
            let loc = e.element_location();
            let mut w = ws.write();
            let Some(v) = w.active_mut() else { return };
            let (lane_list, _) = lanes(v);
            v.selected_span = hit_span(v, &lane_list, lanes_w, loc.x, loc.y);
            if let Some((t, i)) = v.selected_span
                && let Some(span) = v.session.thread(t).and_then(|th| th.spans.get(i as usize))
            {
                v.selected_function = Some(span.callsite);
            }
            v.revision += 1;
        });

    let range_label = mono(
        format!(
            "{} → {} · {}",
            Clock(view.view.0),
            Clock(view.view.1),
            Nanos(view.view.1.saturating_sub(view.view.0))
        ),
        11.,
        theme::TEXT2,
    );

    rect()
        .expanded()
        .content(Content::flex())
        .child(frames_header)
        .child(strip)
        .child(hline())
        .child(ruler)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .padding((6., 16., 0., 16.))
                .child(caption(&tr!("shell-threads")).width(Size::flex(1.)))
                .child(range_label),
        )
        .child(ScrollView::new().height(Size::flex(1.)).child(lanes_el))
        .maybe(view.selected_span.is_some(), |r| r.child(hline()))
        .maybe_child(detail(ws, view))
}

//! Session window chrome shared by every tab.

use std::collections::HashMap;

use freya::prelude::*;
use sismo_analysis::Group;
use sismo_analysis::units::{Bytes, Clock, Nanos};

use super::i18n::tr;
use super::theme::{self, Rgb};
use super::widgets::{Kind, badge, bordered, button, caption, hline, mono, swatch, text, vline};
use super::{Dialog, Workspace, dialogs, flame, functions, logs, metrics, query_bar, timeline};
use crate::workspace::{LiveStatus, Origin, SessionView, Tab};

fn top_bar(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let title = view.title();
    let status: Rect = match &view.origin {
        Origin::File { report, .. } => {
            let (start, end) = view.session.span();
            let label = if view.session.panics.is_empty() {
                tr!(
                    "shell-file-badge",
                    duration = Nanos(end - start).to_string()
                )
            } else {
                tr!(
                    "shell-black-box-badge",
                    duration = Nanos(end - start).to_string()
                )
            };
            if !view.session.panics.is_empty() {
                badge(label, theme::ERROR, theme::REC_DIM)
            } else if report.damaged.is_some() {
                badge(
                    tr!("shell-damaged-badge", label = label),
                    theme::WARN,
                    theme::WARN_DIM,
                )
            } else {
                badge(label, theme::TEXT2, theme::BG2)
            }
        }
        Origin::Live(live) => match &live.status {
            LiveStatus::Connecting => badge(tr!("shell-connecting"), theme::TEXT2, theme::BG2),
            LiveStatus::NeedsPairing { .. } => {
                badge(tr!("common-state-needs-code"), theme::WARN, theme::WARN_DIM)
            }
            LiveStatus::Lost { since, .. } => badge(
                format!(
                    "{} {}",
                    tr!("shell-lost"),
                    Clock(u64::try_from(since.elapsed().as_nanos()).unwrap_or(0))
                ),
                theme::WARN,
                theme::WARN_DIM,
            ),
            LiveStatus::Streaming => {
                let (start, end) = view.session.span();
                let label = if view.paused {
                    tr!("shell-paused")
                } else {
                    tr!("shell-live")
                };
                badge(
                    format!("{label} {}", Clock(end - start)),
                    theme::TEXT,
                    theme::BG3,
                )
            }
        },
    };
    let rec = view.live().and_then(|l| l.recording.as_ref()).map(|r| {
        badge(
            tr!(
                "shell-rec-badge",
                clock =
                    Clock(u64::try_from(r.started.elapsed().as_nanos()).unwrap_or(0)).to_string(),
                size = Bytes(r.bytes()).to_string(),
                markers = r.markers.to_string(),
            ),
            theme::REC,
            theme::REC_DIM,
        )
    });
    let is_live = view.live().is_some();
    let recording = rec.is_some();
    let paused = view.paused;
    rect()
        .width(Size::fill())
        .height(Size::px(44.))
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((0., 16.))
        .spacing(12.)
        .background(theme::color(theme::BG1))
        .child(text("sismo", 14., theme::TEXT).font_weight(600))
        .child(
            rect()
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| ws.write().active = None)
                .child(text(
                    format!("{}  ›", tr!("sessions-title")),
                    13.,
                    theme::TEXT3,
                )),
        )
        .child(text(title, 13., theme::TEXT).font_weight(500).max_lines(1))
        .child(status)
        .child(rect().width(Size::flex(1.)))
        .maybe_child(rec)
        .child(button(tr!("settings-title"), Kind::Secondary, move |_| {
            ws.write().settings = Some(super::settings::Section::default());
        }))
        .maybe(is_live, |r| {
            r.child(button(tr!("shell-marker"), Kind::Secondary, move |_| {
                ws.write().dialog = Some(Dialog::Marker {
                    text: String::new(),
                });
            }))
            .child(button(
                tr!("shell-save-received"),
                Kind::Secondary,
                move |_| dialogs::save_received(ws),
            ))
            .child(button(tr!("shell-clear"), Kind::Secondary, move |_| {
                ws.write().dialog = Some(Dialog::ClearSession);
            }))
        })
        .maybe(is_live, |r| {
            r.child(button(
                if paused {
                    tr!("shell-resume")
                } else {
                    tr!("shell-pause")
                },
                Kind::Secondary,
                move |_| {
                    if let Some(view) = ws.write().active_mut() {
                        view.paused = !view.paused;
                        view.follow = !view.paused;
                    }
                },
            ))
            .child(button(
                if recording {
                    tr!("shell-stop-recording")
                } else {
                    tr!("shell-record")
                },
                if recording {
                    Kind::Danger
                } else {
                    Kind::Secondary
                },
                move |_| dialogs::toggle_record(ws),
            ))
        })
        .maybe(!is_live, |r| {
            r.child(button(tr!("shell-export"), Kind::Secondary, move |_| {
                ws.write().dialog = Some(Dialog::Export);
            }))
        })
}

fn tabs_row(mut ws: State<Workspace>, index: usize, view: &SessionView) -> Rect {
    let current = view.tab;
    rect()
        .width(Size::fill())
        .height(Size::px(44.))
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::End)
        .padding((0., 16., 0., 248.))
        .background(theme::color(theme::BG0))
        .children(Tab::ALL.iter().map(|tab| {
            let tab = *tab;
            let on = tab == current;
            let tab_label = tr!(tab.label());
            let tab_w = super::draw::measure(&tab_label, 13., false) * 1.12 + 28.;
            rect()
                .width(Size::px(tab_w))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        v.tab = tab;
                    }
                })
                .child(
                    rect().padding((10., 13., 8., 13.)).child(
                        text(tab_label, 13., if on { theme::TEXT } else { theme::TEXT2 })
                            .font_weight(if on { 600 } else { 400 }),
                    ),
                )
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::px(2.))
                        .background(theme::color(if on { theme::TEXT } else { theme::BG0 })),
                )
                .into_element()
        }))
        .child(rect().width(Size::flex(1.)))
        .child(
            rect()
                .padding((0., 0., 7., 0.))
                .child(query_bar::QueryBar { ws, index }),
        )
}

/// Time per crate and per thread inside a window.
#[derive(Clone, Default)]
struct Shares {
    per_crate: HashMap<String, u64>,
    /// Thread id and time spent in its top-level spans.
    per_thread: Vec<(u32, u64)>,
}

impl Shares {
    fn measure(session: &sismo_analysis::Session, window: (u64, u64)) -> Self {
        let mut per_callsite: HashMap<u32, u64> = HashMap::new();
        let mut per_thread = Vec::with_capacity(session.threads.len());
        for thread in &session.threads {
            let mut top = 0;
            for span in thread.spans_in(window.0, window.1) {
                *per_callsite.entry(span.callsite).or_default() += span.self_ns;
                if span.depth == 0 {
                    top += span.duration(session.end);
                }
            }
            per_thread.push((thread.id, top));
        }
        let mut per_crate: HashMap<String, u64> = HashMap::new();
        for (callsite, ns) in per_callsite {
            *per_crate
                .entry(session.crate_of(callsite).to_owned())
                .or_default() += ns;
        }
        Self {
            per_crate,
            per_thread,
        }
    }
}

/// Session, window and event count a [`Shares`] was measured for.
type SharesKey = (u64, (u64, u64), u64);

thread_local! {
    /// Last shares measured, keyed by session, window and event count.
    static SHARES: std::cell::RefCell<Option<(SharesKey, Shares)>> =
        const { std::cell::RefCell::new(None) };
}

/// Shares of `view`'s window. While a range is being dragged the last
/// result is reused, so dragging stays smooth on long sessions.
fn shares(view: &SessionView) -> Shares {
    let key = (view.id, view.window(), view.session.events);
    SHARES.with(|cell| {
        let mut slot = cell.borrow_mut();
        match slot.as_ref() {
            Some((cached, shares))
                if *cached == key || (cached.0 == key.0 && timeline::dragging()) =>
            {
                shares.clone()
            }
            _ => {
                let shares = Shares::measure(&view.session, key.1);
                *slot = Some((key, shares.clone()));
                shares
            }
        }
    })
}

fn side_panel(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let session = &view.session;
    let Shares {
        per_crate,
        per_thread,
    } = shares(view);
    let total: u64 = per_crate.values().sum::<u64>().max(1);
    let mut by_group: Vec<(Group, Vec<(String, u64)>)> =
        Group::ALL.iter().map(|g| (*g, Vec::new())).collect();
    for krate in session.crates() {
        let group = session.groups.classify(&krate);
        let ns = per_crate.get(&krate).copied().unwrap_or(0);
        if let Some((_, list)) = by_group.iter_mut().find(|(g, _)| *g == group) {
            list.push((krate, ns));
        }
    }
    for (_, list) in &mut by_group {
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    let hidden = view.hidden_crates.clone();
    let hidden_threads = view.hidden_threads.clone();
    let mut threads: Vec<(u32, String, u64)> = per_thread
        .into_iter()
        .map(|(id, ns)| (id, session.thread_name(id).to_owned(), ns))
        .collect();
    threads.sort_by_key(|t| std::cmp::Reverse(t.2));
    let thread_total: u64 = threads.iter().map(|t| t.2).sum::<u64>().max(1);

    let crate_row = move |krate: String, ns: u64, rgb: Rgb| {
        let off = hidden.contains(&krate);
        #[allow(clippy::cast_precision_loss)]
        let share = ns as f32 / total as f32;
        let toggle = krate.clone();
        rect()
            .width(Size::fill())
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(8.)
            .padding((3., 0.))
            .cursor(CursorIcon::Pointer)
            .on_press(move |_| {
                if let Some(v) = ws.write().active_mut() {
                    v.toggle_crate(&toggle);
                }
            })
            .child(
                rect()
                    .width(Size::px(12.))
                    .height(Size::px(12.))
                    .corner_radius(2.)
                    .background(theme::color(if off { theme::BG1 } else { rgb }))
                    .border(bordered(if off { theme::LINE2 } else { rgb })),
            )
            .child(
                mono(krate, 12., if off { theme::TEXT3 } else { theme::TEXT })
                    .width(Size::px(140.)),
            )
            .child(
                rect()
                    .width(Size::px(36.))
                    .height(Size::px(3.))
                    .corner_radius(1.)
                    .background(theme::color(theme::BG3))
                    .child(
                        rect()
                            .width(Size::px((share * 36.).max(1.)))
                            .height(Size::px(3.))
                            .background(theme::color(if off { theme::TEXT3 } else { rgb })),
                    ),
            )
            .into_element()
    };

    rect()
        .width(Size::px(248.))
        .height(Size::fill())
        .background(theme::color(theme::BG1))
        .child(
            ScrollView::new().height(Size::fill()).child(
                rect()
                    .width(Size::fill())
                    .padding((14., 16.))
                    .spacing(4.)
                    .child(
                        rect()
                            .horizontal()
                            .content(Content::flex())
                            .width(Size::fill())
                            .cross_align(Alignment::Center)
                            .child(caption("Crates").width(Size::flex(1.)))
                            .child(
                                rect()
                                    .padding((0., 8.))
                                    .cursor(CursorIcon::Pointer)
                                    .on_press(move |_| ws.write().dialog = Some(Dialog::Groups))
                                    .child(text(tr!("shell-edit-groups"), 11., theme::TEXT2)),
                            )
                            .child(text(tr!("shell-time-share"), 11., theme::TEXT3)),
                    )
                    .children(by_group.into_iter().filter(|(_, l)| !l.is_empty()).map(
                        |(group, list)| {
                            rect()
                                .width(Size::fill())
                                .padding((8., 0., 4., 0.))
                                .spacing(2.)
                                .child(
                                    text(theme::group_label(group), 12., theme::TEXT)
                                        .font_weight(600),
                                )
                                .children(
                                    list.into_iter()
                                        .map(|(k, ns)| crate_row(k, ns, theme::group(group))),
                                )
                                .into_element()
                        },
                    ))
                    .child(rect().height(Size::px(10.)))
                    .child(hline())
                    .child(rect().height(Size::px(10.)))
                    .child(caption(&tr!("shell-threads")))
                    .children(threads.into_iter().map(|(id, name, ns)| {
                        let off = hidden_threads.contains(&id);
                        #[allow(clippy::cast_precision_loss)]
                        let share = ns as f64 / thread_total as f64 * 100.;
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .content(Content::flex())
                            .cross_align(Alignment::Center)
                            .spacing(8.)
                            .padding((3., 0.))
                            .cursor(CursorIcon::Pointer)
                            .on_press(move |_| {
                                if let Some(v) = ws.write().active_mut() {
                                    v.toggle_thread(id);
                                }
                            })
                            .child(swatch(if off { theme::BG3 } else { theme::TEXT2 }, 12., 2.))
                            .child(
                                mono(name, 12., if off { theme::TEXT3 } else { theme::TEXT })
                                    .width(Size::px(150.)),
                            )
                            .child(mono(format!("{share:.0}%"), 11., theme::TEXT3))
                            .into_element()
                    })),
            ),
        )
}

fn status_bar(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let session = &view.session;
    let left = if let Some(live) = view.live() {
        tr!(
            "shell-status-live",
            rate = format!("{:.0}", live.events_per_sec),
            dropped = session.dropped.to_string(),
            received = Bytes(live.received_bytes).to_string(),
            addr = live.addr.to_string(),
            rtt = live
                .rtt
                .map_or_else(|| "—".to_owned(), |r| format!("{} ms", r.as_millis())),
        )
    } else {
        tr!(
            "shell-status-file",
            frames = session.frames.len().to_string(),
            events = session.events.to_string(),
            logs = session.logs.len().to_string(),
            dropped = session.dropped.to_string(),
            platform = session.info.platform.clone(),
            device = session.info.device.clone(),
            version = if session.info.build.is_empty() {
                session.info.app_version.clone()
            } else {
                format!("{} ({})", session.info.app_version, session.info.build)
            },
        )
    };
    let filter = view
        .live()
        .map(|l| tr!("shell-app-filter", filter = l.app_filter.clone()));
    rect()
        .width(Size::fill())
        .height(Size::px(24.))
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((0., 16.))
        .background(theme::color(theme::BG1))
        .child(mono(left, 11., theme::TEXT3).width(Size::flex(1.)))
        .maybe_child(filter.map(|f| {
            rect()
                .padding((2., 8.))
                .corner_radius(3.)
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| ws.write().dialog = Some(Dialog::AppFilter))
                .child(mono(f, 11., theme::TEXT2))
        }))
}

fn waiting(mut ws: State<Workspace>, view: &SessionView) -> Option<Rect> {
    let live = view.live()?;
    let (title, body, retry) = match &live.status {
        LiveStatus::Connecting => (
            tr!("shell-waiting-connecting"),
            tr!("shell-waiting-for", addr = live.addr.to_string()),
            false,
        ),
        LiveStatus::NeedsPairing { rejected } => (
            tr!("shell-waiting-pairing"),
            rejected
                .clone()
                .unwrap_or_else(|| tr!("shell-waiting-pairing-body")),
            false,
        ),
        LiveStatus::Lost { reason, .. } if view.session.events == 0 => {
            (tr!("shell-waiting-lost"), reason.clone(), true)
        }
        _ => return None,
    };
    Some(
        rect()
            .expanded()
            .center()
            .spacing(12.)
            .child(text(title, 15., theme::TEXT).font_weight(600))
            .child(text(body, 12., theme::TEXT2))
            .maybe(retry, |r| {
                r.child(button(tr!("shell-retry"), Kind::Secondary, move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        let _ = v.reconnect();
                    }
                }))
            })
            .maybe(
                matches!(live.status, LiveStatus::NeedsPairing { .. }),
                |r| {
                    r.child(button(tr!("shell-enter-code"), Kind::Primary, move |_| {
                        ws.write().dialog = Some(Dialog::Pair {
                            code: String::new(),
                        });
                    }))
                },
            ),
    )
}

fn banner(mut ws: State<Workspace>, view: &SessionView) -> Option<Rect> {
    let live = view.live()?;
    if let LiveStatus::Lost { reason, .. } = &live.status {
        let end = view.session.span().1;
        return Some(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((6., 16.))
                .spacing(8.)
                .background(theme::color(theme::WARN_DIM))
                .child(
                    text(
                        tr!(
                            "shell-lost-banner",
                            reason = reason.as_str(),
                            until = Clock(end).to_string(),
                        ),
                        12.,
                        theme::TEXT,
                    )
                    .width(Size::flex(1.)),
                )
                .child(button(tr!("shell-retry-now"), Kind::Secondary, move |_| {
                    if let Some(v) = ws.write().active_mut() {
                        let _ = v.reconnect();
                    }
                }))
                .child(button(
                    tr!("shell-save-received"),
                    Kind::Secondary,
                    move |_| dialogs::save_received(ws),
                )),
        );
    }
    view.paused.then(|| {
        rect()
            .width(Size::fill())
            .padding((8., 16.))
            .background(theme::color(theme::BG2))
            .child(text(tr!("shell-paused-banner"), 12., theme::TEXT2))
    })
}

pub fn session_screen(ws: State<Workspace>, index: usize) -> Rect {
    let w = ws.read();
    let Some(view) = w.sessions.get(index) else {
        return rect();
    };
    let top = top_bar(ws, view);
    let tabs = tabs_row(ws, index, view);
    let side = side_panel(ws, view);
    let status = status_bar(ws, view);
    let banner = banner(ws, view);
    let body = match waiting(ws, view) {
        Some(waiting) => waiting.into_element(),
        None => match view.tab {
            Tab::Timeline => timeline::view(ws, view).into_element(),
            Tab::Flamegraph => flame::view(ws, view).into_element(),
            Tab::Functions => functions::view(ws, view).into_element(),
            Tab::Logs => logs::view(ws, view).into_element(),
            Tab::Metrics => metrics::view(view).into_element(),
        },
    };
    drop(w);
    rect()
        .expanded()
        .content(Content::flex())
        .child(top)
        .child(hline())
        .child(tabs)
        .child(hline())
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.))
                .horizontal()
                .content(Content::flex())
                .child(side)
                .child(vline())
                .child(
                    rect()
                        .width(Size::flex(1.))
                        .height(Size::fill())
                        .maybe_child(banner)
                        .child(body),
                ),
        )
        .child(hline())
        .child(status)
}

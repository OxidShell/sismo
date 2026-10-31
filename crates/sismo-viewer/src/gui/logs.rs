//! Logs tab: filtered, highlighted log list with the selected entry's detail.

use std::cell::Cell;
use std::rc::Rc;

use freya::clipboard::Clipboard;
use freya::prelude::*;
use sismo_analysis::units::Clock;
use sismo_analysis::{LogEntry, Panic, Query, Session};
use sismo_format::{Level, Source};

use super::Workspace;
use super::i18n::tr;
use super::theme;
use super::timeline::focus_frame;
use super::widgets::{Kind, badge, button, caption, chip, hline, mono, text};
use crate::workspace::{SessionView, Tab};

const ROW_H: f32 = 26.;

#[derive(Clone, PartialEq)]
struct Row {
    index: usize,
    time: String,
    level: Level,
    source: Source,
    target: String,
    message: String,
    highlights: Vec<(usize, usize)>,
}

fn message(row: &Row) -> Paragraph {
    let mut p = paragraph()
        .width(Size::flex(1.))
        .font_family(theme::MONO)
        .font_size(12.)
        .color(theme::color(theme::TEXT))
        .max_lines(1)
        .text_overflow(TextOverflow::Ellipsis);
    let text = &row.message;
    let mut cursor = 0;
    for (start, end) in &row.highlights {
        if *start < cursor
            || *end > text.len()
            || !text.is_char_boundary(*start)
            || !text.is_char_boundary(*end)
        {
            continue;
        }
        if *start > cursor {
            p = p.span(Span::new(text[cursor..*start].to_owned()));
        }
        p = p.span(
            Span::new(text[*start..*end].to_owned())
                .color(theme::color(theme::WARN))
                .font_weight(FontWeight::BOLD),
        );
        cursor = *end;
    }
    if cursor < text.len() {
        p = p.span(Span::new(text[cursor..].to_owned()));
    }
    p
}

fn rows(view: &SessionView, query: &Query) -> Vec<Row> {
    let session = &view.session;
    let (from, to) = view.window();
    let first = session.logs.partition_point(|l| l.t < from);
    session.logs[first..]
        .iter()
        .enumerate()
        .take_while(|(_, l)| l.t < to)
        .filter(|(_, l)| {
            !view.hidden_levels.contains(&l.level)
                && !view.hidden_sources.contains(&l.source)
                && query.matches_log(session, l)
        })
        .map(|(i, l)| Row {
            index: first + i,
            time: Clock(l.t).to_string(),
            level: l.level,
            source: l.source,
            target: match session.panic_of(l) {
                Some(p) => p.location.clone(),
                None => session
                    .callsite(l.callsite)
                    .map_or_else(|| "?".to_owned(), |c| c.target.clone()),
            },
            highlights: query.highlights(&l.message),
            message: l.message.clone(),
        })
        .collect()
}

/// Innermost span on the log's thread that contains its timestamp.
fn enclosing(session: &Session, log: &LogEntry) -> Vec<String> {
    let Some(thread) = session.thread(log.thread) else {
        return Vec::new();
    };
    let Some(last) = thread.indices_in(log.t, log.t + 1).into_iter().rfind(|i| {
        let s = &thread.spans[*i as usize];
        s.start <= log.t && s.end > log.t
    }) else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    let mut cursor = Some(last);
    while let Some(index) = cursor {
        let span = &thread.spans[index as usize];
        chain.push(
            session
                .name(span.callsite)
                .rsplit("::")
                .next()
                .unwrap_or("?")
                .to_owned(),
        );
        cursor = span.parent;
    }
    chain.reverse();
    chain
}

fn detail(mut ws: State<Workspace>, view: &SessionView, index: usize) -> Option<Rect> {
    let session = &view.session;
    let log = session.logs.get(index)?;
    if let Some(panic) = session.panic_of(log) {
        return Some(panic_detail(ws, view, panic));
    }
    let def = session.callsite(log.callsite);
    let chain = enclosing(session, log);
    let frame = session.frames.partition_point(|f| f.end <= log.t);
    let frame = session
        .frames
        .get(frame)
        .filter(|f| f.start <= log.t)
        .copied();
    let t = log.t;
    Some(
        rect()
            .width(Size::fill())
            .height(Size::px(170.))
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
                            .spacing(10.)
                            .child(
                                mono(
                                    log.level.as_str().to_uppercase(),
                                    11.,
                                    theme::level(log.level),
                                )
                                .font_weight(600),
                            )
                            .child(mono(log.message.clone(), 13., theme::TEXT)),
                    )
                    .child(mono(
                        tr!(
                            "logs-detail-location",
                            time = Clock(log.t).to_string(),
                            target = def.map_or("?", |d| d.target.as_str()),
                            file = def.map_or("?", |d| d.file.as_str()),
                            line = def.map_or(0, |d| d.line).to_string(),
                            thread = session.thread_name(log.thread).to_string(),
                            source = log.source.as_str(),
                        ),
                        11.,
                        theme::TEXT3,
                    ))
                    .child(caption(&tr!("logs-fields")))
                    .maybe(log.fields.is_empty(), |r| {
                        r.child(text(tr!("logs-no-fields"), 12., theme::TEXT3))
                    })
                    .children(log.fields.iter().take(5).map(|(k, v)| {
                        rect()
                            .horizontal()
                            .spacing(12.)
                            .child(mono(k.clone(), 12., theme::TEXT3).width(Size::px(120.)))
                            .child(mono(v.clone(), 12., theme::TEXT))
                            .into_element()
                    })),
            )
            .child(
                rect()
                    .width(Size::px(420.))
                    .spacing(8.)
                    .child(caption(&tr!("logs-inside")))
                    .child(mono(
                        if chain.is_empty() {
                            tr!("logs-outside-span")
                        } else {
                            chain.join(" › ")
                        },
                        12.,
                        theme::TEXT2,
                    ))
                    .maybe_child(frame.map(|f| {
                        text(
                            format!(
                                "frame #{} · {}",
                                f.index,
                                sismo_analysis::units::Nanos(f.duration())
                            ),
                            11.,
                            theme::TEXT3,
                        )
                    }))
                    .child(button(
                        tr!("common-show-in-timeline"),
                        Kind::Secondary,
                        move |_| {
                            if let Some(v) = ws.write().active_mut() {
                                match frame {
                                    Some(f) => focus_frame(v, f.index),
                                    None => v.view = (t.saturating_sub(5_000_000), t + 5_000_000),
                                }
                                v.follow = false;
                                v.tab = Tab::Timeline;
                            }
                        },
                    )),
            ),
    )
}

/// A panic: what and where, the thread, and its backtrace.
fn panic_detail(mut ws: State<Workspace>, view: &SessionView, panic: &Panic) -> Rect {
    let session = &view.session;
    let t = panic.t;
    let thread = session.thread_name(panic.thread).to_owned();
    let copy = format!(
        "panicked at {} on thread {}:\n{}\n\n{}",
        panic.location, thread, panic.message, panic.backtrace
    );
    rect()
        .width(Size::fill())
        .height(Size::px(300.))
        .horizontal()
        .content(Content::flex())
        .padding((14., 16.))
        .spacing(24.)
        .background(theme::color(theme::BG1))
        .child(
            rect()
                .width(Size::px(420.))
                .spacing(8.)
                .child(
                    mono(Level::Panic.as_str().to_uppercase(), 11., theme::ERROR).font_weight(600),
                )
                .child(
                    mono(panic.message.clone(), 13., theme::TEXT)
                        .max_lines(6)
                        .width(Size::fill()),
                )
                .child(
                    mono(
                        tr!(
                            "logs-panic-where",
                            time = Clock(t).to_string(),
                            location = panic.location.as_str(),
                            thread = thread.as_str(),
                        ),
                        11.,
                        theme::TEXT3,
                    )
                    .max_lines(3)
                    .width(Size::fill()),
                )
                .child(
                    rect()
                        .horizontal()
                        .spacing(8.)
                        .child(button(tr!("logs-panic-copy"), Kind::Secondary, move |_| {
                            let _ = Clipboard::set(copy.clone());
                        }))
                        .child(button(
                            tr!("common-show-in-timeline"),
                            Kind::Secondary,
                            move |_| {
                                if let Some(v) = ws.write().active_mut() {
                                    v.view = (t.saturating_sub(50_000_000), t + 10_000_000);
                                    v.follow = false;
                                    v.tab = Tab::Timeline;
                                }
                            },
                        )),
                ),
        )
        .child(
            rect()
                .width(Size::flex(1.))
                .height(Size::fill())
                .content(Content::flex())
                .spacing(8.)
                .child(caption(&tr!("logs-panic-backtrace")))
                .child(if panic.backtrace.is_empty() {
                    text(tr!("logs-panic-no-backtrace"), 12., theme::TEXT3).into_element()
                } else {
                    ScrollView::new()
                        .width(Size::fill())
                        .height(Size::flex(1.))
                        .child(
                            label()
                                .text(panic.backtrace.clone())
                                .font_family(theme::MONO)
                                .font_size(12.)
                                .color(theme::color(theme::TEXT2))
                                .width(Size::fill()),
                        )
                        .into_element()
                }),
        )
}

pub fn view(mut ws: State<Workspace>, view: &SessionView) -> Rect {
    let query = view.effective_query();
    let rows = rows(view, &query);
    let count = rows.len();
    let total = view.session.logs.len();
    let selected = view.selected_log;
    let follow = view.logs_follow && view.live().is_some();

    let level_chips = Level::ALL.iter().map(|level| {
        let level = *level;
        let on = !view.hidden_levels.contains(&level);
        chip(level.as_str().to_uppercase(), theme::level(level), on)
            .on_press(move |_| {
                if let Some(v) = ws.write().active_mut()
                    && !v.hidden_levels.remove(&level)
                {
                    v.hidden_levels.insert(level);
                }
            })
            .into_element()
    });
    let source_chips = [
        Source::Tracing,
        Source::Log,
        Source::Stdout,
        Source::Stderr,
        Source::Sismo,
    ]
    .into_iter()
    .map(|source| {
        let on = !view.hidden_sources.contains(&source);
        chip(source.as_str(), theme::TEXT2, on)
            .on_press(move |_| {
                if let Some(v) = ws.write().active_mut() {
                    if let Some(pos) = v.hidden_sources.iter().position(|s| *s == source) {
                        v.hidden_sources.remove(pos);
                    } else {
                        v.hidden_sources.push(source);
                    }
                }
            })
            .into_element()
    });

    let filters = rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((10., 16.))
        .spacing(6.)
        .children(level_chips)
        .child(rect().width(Size::px(14.)))
        .children(source_chips)
        .child(rect().width(Size::flex(1.)))
        .child(mono(
            tr!(
                "common-count-of-total",
                count = count.to_string(),
                total = total.to_string(),
            ),
            12.,
            theme::TEXT3,
        ))
        .maybe(view.live().is_some(), |r| {
            r.child(
                rect()
                    .cursor(CursorIcon::Pointer)
                    .on_press(move |_| {
                        if let Some(v) = ws.write().active_mut() {
                            v.logs_follow = !v.logs_follow;
                        }
                    })
                    .child(if follow {
                        badge(tr!("logs-following"), theme::TEXT, theme::BG3)
                    } else {
                        badge(tr!("logs-follow-paused"), theme::TEXT3, theme::BG2)
                    }),
            )
        });

    let list = LogList {
        ws,
        rows,
        selected,
        follow,
    };

    rect()
        .expanded()
        .content(Content::flex())
        .child(filters)
        .child(hline())
        .child(list)
        .maybe(selected.is_some(), |r| r.child(hline()))
        .maybe_child(selected.and_then(|i| detail(ws, view, i)))
}

#[derive(PartialEq)]
struct LogList {
    ws: State<Workspace>,
    rows: Vec<Row>,
    selected: Option<usize>,
    follow: bool,
}

impl Component for LogList {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.ws;
        let mut controller = use_scroll_controller(ScrollConfig::default);
        let count = self.rows.len();
        use_side_effect_with_deps(&(count, self.follow), move |(_, follow)| {
            if *follow {
                controller.scroll_to(ScrollPosition::End, Direction::Vertical);
            }
        });
        // Scrolling back by hand means the reader wants to stay there: the
        // offset grows towards zero (the top) while the row count is the
        // same, which following alone never does.
        let current = use_hook(|| Rc::new(Cell::new((0usize, false))));
        current.set((count, self.follow));
        let seen = use_hook(|| Rc::new(Cell::new((0usize, i32::MIN))));
        use_side_effect(move || {
            #[allow(clippy::cast_possible_truncation)]
            let y = controller.position().y as i32;
            let (count, following) = current.get();
            let (last_count, last_y) = seen.get();
            seen.set((count, y));
            if following
                && last_count == count
                && last_y != i32::MIN
                && y > last_y
                && let Some(v) = ws.write().active_mut()
            {
                v.logs_follow = false;
            }
        });
        VirtualScrollView::new_with_data_controlled(
            (self.rows.clone(), self.selected),
            move |item, (rows, selected): &(Vec<Row>, Option<usize>)| {
                let Some(row) = rows.get(item.index) else {
                    return rect().into_element();
                };
                let index = row.index;
                let on = *selected == Some(index);
                rect()
                    .key(index)
                    .width(Size::fill())
                    .height(Size::px(ROW_H))
                    .horizontal()
                    .content(Content::flex())
                    .cross_align(Alignment::Center)
                    .padding((0., 16.))
                    .spacing(14.)
                    .background(theme::color(match (on, row.level) {
                        (true, _) => theme::BG3,
                        (false, Level::Panic) => theme::REC_DIM,
                        (false, _) => theme::BG0,
                    }))
                    .cursor(CursorIcon::Pointer)
                    .on_press(move |_| {
                        if let Some(v) = ws.write().active_mut() {
                            v.selected_log = Some(index);
                        }
                    })
                    .child(mono(row.time.clone(), 12., theme::TEXT3).width(Size::px(80.)))
                    .child(
                        mono(
                            row.level.as_str().to_uppercase(),
                            11.,
                            theme::level(row.level),
                        )
                        .font_weight(600)
                        .width(Size::px(44.)),
                    )
                    .child(mono(row.source.as_str(), 11., theme::TEXT3).width(Size::px(52.)))
                    .child(mono(row.target.clone(), 12., theme::TEXT2).width(Size::px(220.)))
                    .child(message(row))
                    .into_element()
            },
            controller,
        )
        .length(count)
        .item_size(ROW_H)
        .height(Size::flex(1.))
    }
}

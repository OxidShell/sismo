//! Right-click menu on spans and flamegraph nodes.

use freya::clipboard::Clipboard;
use freya::prelude::*;

use super::i18n::tr;
use super::theme;
use super::widgets::{bordered, kbd, mono, text};
use super::{Dialog, Workspace};
use crate::workspace::Tab;

/// What the menu was opened on, and where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuTarget {
    pub callsite: u32,
    pub thread: Option<u32>,
    pub span: Option<u32>,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy)]
enum Action {
    Functions,
    Flamegraph,
    SelectFrame,
    FilterCrate,
    HideCrate,
    OnlyFunction,
    CopyName,
    CopyJson,
    CopySource,
    Marker,
}

impl Action {
    const fn key(self) -> &'static str {
        match self {
            Action::Functions => "menu-functions",
            Action::Flamegraph => "menu-flamegraph",
            Action::SelectFrame => "menu-select-frame",
            Action::FilterCrate => "menu-filter-crate",
            Action::HideCrate => "menu-hide-crate",
            Action::OnlyFunction => "menu-only-function",
            Action::CopyName => "menu-copy-name",
            Action::CopyJson => "menu-copy-json",
            Action::CopySource => "menu-copy-source",
            Action::Marker => "menu-marker",
        }
    }

    const fn keys(self) -> &'static str {
        match self {
            Action::Functions => "Enter",
            Action::Flamegraph => "F",
            Action::HideCrate => "H",
            Action::Marker => "M",
            _ => "",
        }
    }
}

fn span_json(ws: &Workspace, target: &MenuTarget) -> String {
    let Some(view) = ws.active() else {
        return String::new();
    };
    let session = &view.session;
    let def = session.callsite(target.callsite);
    let span = target.thread.zip(target.span).and_then(|(t, i)| {
        session
            .thread(t)
            .and_then(|th| th.spans.get(i as usize))
            .copied()
    });
    let calls = session.calls_of(target.callsite, session.span());
    let mut durations: Vec<u64> = calls.iter().map(|c| c.0).collect();
    durations.sort_unstable();
    serde_json::to_string_pretty(&serde_json::json!({
        "function": session.name(target.callsite),
        "crate": session.crate_of(target.callsite),
        "group": session.group_of(target.callsite).as_str(),
        "file": def.map(|d| d.file.clone()),
        "line": def.map(|d| d.line),
        "thread": target.thread.map(|t| session.thread_name(t).to_owned()),
        "span": span.map(|s| serde_json::json!({
            "start_ns": s.start,
            "duration_ns": s.duration(session.end),
            "self_ns": s.self_ns,
            "alloc_bytes": s.alloc_bytes,
        })),
        "session": {
            "calls": durations.len(),
            "p50_ns": sismo_analysis::percentile(&durations, 0.5),
            "p95_ns": sismo_analysis::percentile(&durations, 0.95),
            "max_ns": durations.last(),
        },
        "app": session.info.app,
        "app_version": session.info.app_version,
    }))
    .unwrap_or_default()
}

fn run(mut ws: State<Workspace>, target: MenuTarget, action: Action) {
    let mut w = ws.write();
    w.menu = None;
    match action {
        Action::CopyName => {
            if let Some(v) = w.active() {
                let _ = Clipboard::set(v.session.name(target.callsite).to_owned());
            }
        }
        Action::CopyJson => {
            let json = span_json(&w, &target);
            let _ = Clipboard::set(json);
        }
        Action::CopySource => {
            if let Some(def) = w.active().and_then(|v| v.session.callsite(target.callsite)) {
                let _ = Clipboard::set(format!("{}:{}", def.file, def.line));
            }
        }
        Action::Marker => {
            w.dialog = Some(Dialog::Marker {
                text: String::new(),
            });
        }
        _ => {
            let Some(v) = w.active_mut() else { return };
            let name = v.session.name(target.callsite).to_owned();
            let krate = v.session.crate_of(target.callsite).to_owned();
            match action {
                Action::Functions => {
                    v.selected_function = Some(target.callsite);
                    v.tab = Tab::Functions;
                }
                Action::Flamegraph => {
                    v.selected_function = Some(target.callsite);
                    v.tab = Tab::Flamegraph;
                }
                Action::SelectFrame => {
                    if let Some(span) = target.thread.zip(target.span).and_then(|(t, i)| {
                        v.session
                            .thread(t)
                            .and_then(|th| th.spans.get(i as usize))
                            .copied()
                    }) {
                        let index = v.session.frames.partition_point(|f| f.end <= span.start);
                        if let Some(frame) = v.session.frames.get(index).map(|f| f.index) {
                            super::timeline::focus_frame(v, frame);
                        }
                    }
                }
                Action::FilterCrate => {
                    let query = format!("{} crate:{krate}", v.query_text).trim().to_owned();
                    v.set_query(query);
                }
                Action::HideCrate => v.toggle_crate(&krate),
                Action::OnlyFunction => v.set_query(format!("fn:{name}")),
                Action::CopyName | Action::CopyJson | Action::CopySource | Action::Marker => {}
            }
            v.revision += 1;
        }
    }
}

pub fn render(mut ws: State<Workspace>, target: MenuTarget) -> Rect {
    let w = ws.read();
    let Some(view) = w.active() else {
        return rect();
    };
    let name = view.session.name(target.callsite).to_owned();
    let is_live = view.live().is_some();
    let has_span = target.span.is_some();
    drop(w);
    let groups: [&[Action]; 4] = [
        &[Action::Functions, Action::Flamegraph, Action::SelectFrame],
        &[Action::FilterCrate, Action::HideCrate, Action::OnlyFunction],
        &[Action::CopyName, Action::CopyJson, Action::CopySource],
        &[Action::Marker],
    ];
    #[allow(clippy::cast_possible_truncation)]
    let (x, y) = (target.x as f32, target.y as f32);
    rect()
        .position(Position::new_global().top(0.).left(0.))
        .width(Size::window_percent(100.))
        .height(Size::window_percent(100.))
        .layer(Layer::Overlay)
        .on_press(move |_| ws.write().menu = None)
        .on_secondary_down(move |_| ws.write().menu = None)
        .child(
            rect()
                .position(Position::new_global().top(y).left(x))
                .width(Size::px(300.))
                .padding(4.)
                .corner_radius(6.)
                .background(theme::color(theme::BG1))
                .border(bordered(theme::LINE2))
                .shadow(
                    Shadow::new()
                        .y(8.)
                        .blur(24.)
                        .color(Color::from_argb(90, 0, 0, 0)),
                )
                .on_press(|e: Event<PressEventData>| e.stop_propagation())
                .child(
                    rect()
                        .padding((6., 10.))
                        .child(mono(name, 11., theme::TEXT3)),
                )
                .children(groups.iter().enumerate().flat_map(|(gi, group)| {
                    let separator = (gi > 0).then(|| {
                        rect()
                            .width(Size::fill())
                            .height(Size::px(1.))
                            .margin((4., 6.))
                            .background(theme::color(theme::LINE))
                            .into_element()
                    });
                    separator
                        .into_iter()
                        .chain(group.iter().filter_map(move |action| {
                            let action = *action;
                            if matches!(action, Action::Marker) && !is_live {
                                return None;
                            }
                            if matches!(action, Action::SelectFrame) && !has_span {
                                return None;
                            }
                            Some(
                                rect()
                                    .width(Size::fill())
                                    .horizontal()
                                    .content(Content::flex())
                                    .cross_align(Alignment::Center)
                                    .padding((6., 10.))
                                    .corner_radius(4.)
                                    .cursor(CursorIcon::Pointer)
                                    .on_press(move |_| run(ws, target, action))
                                    .child(
                                        text(tr!(action.key()), 12., theme::TEXT)
                                            .width(Size::flex(1.)),
                                    )
                                    .maybe(!action.keys().is_empty(), |r| {
                                        r.child(kbd(action.keys()))
                                    })
                                    .into_element(),
                            )
                        }))
                })),
        )
}

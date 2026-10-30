//! Start screen: apps streaming on the LAN and recent recordings.

use std::time::Duration;

use freya::prelude::*;
use sismo_format::FORMAT_VERSION;
use sismo_format::net::AuthMode;

use super::i18n::tr;
use super::theme::{self, Rgb};
use super::widgets::{Kind, badge, bordered, button, caption, kbd, mono, text};
use super::{Workspace, dialogs};
use crate::client::Discovered;

fn top_bar(ws: State<Workspace>) -> Rect {
    rect()
        .width(Size::fill())
        .height(Size::px(44.))
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((0., 16.))
        .spacing(16.)
        .background(theme::color(theme::BG1))
        .border(
            Border::new()
                .fill(theme::color(theme::LINE))
                .width(1.)
                .alignment(BorderAlignment::Inner),
        )
        .child(text("sismo", 14., theme::TEXT).font_weight(600))
        .child(text(tr!("sessions-title"), 13., theme::TEXT).font_weight(500))
        .child(rect().width(Size::flex(1.)))
        .child(kbd("Ctrl O"))
        .child(button(
            tr!("sessions-open-file"),
            Kind::Primary,
            move |_| {
                dialogs::pick_file(ws);
            },
        ))
}

fn age(app: &Discovered) -> String {
    let secs = app.seen.elapsed().as_secs();
    if secs < 5 {
        tr!("sessions-seen-now")
    } else {
        tr!("sessions-seen-ago", secs = secs.to_string())
    }
}

fn app_card(mut ws: State<Workspace>, app: &Discovered, connected: bool) -> Rect {
    let b = &app.beacon;
    let compatible = b.format_version == FORMAT_VERSION;
    let (state, fg, bg): (String, Rgb, Rgb) = if !compatible {
        (tr!("sessions-state-incompatible"), theme::TEXT3, theme::BG3)
    } else if connected {
        (tr!("sessions-state-connected"), theme::TEXT, theme::BG3)
    } else if b.auth == AuthMode::Pairing {
        (tr!("common-state-needs-code"), theme::WARN, theme::WARN_DIM)
    } else {
        (tr!("sessions-state-available"), theme::TEXT2, theme::BG3)
    };
    let addr = app.addr;
    let action = if !compatible {
        tr!("sessions-action-details")
    } else if connected {
        tr!("common-open")
    } else if b.auth == AuthMode::Pairing {
        tr!("sessions-action-pair")
    } else {
        tr!("sessions-action-connect")
    };
    let detail = if compatible {
        format!("{} · {} · {}", b.device, b.platform, age(app))
    } else {
        tr!(
            "sessions-incompatible-detail",
            platform = b.platform.clone(),
            theirs = b.format_version.to_string(),
            ours = FORMAT_VERSION.to_string(),
        )
    };
    let name_color = if compatible {
        theme::TEXT
    } else {
        theme::TEXT3
    };
    rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((14., 20.))
        .spacing(16.)
        .corner_radius(6.)
        .background(theme::color(if connected {
            theme::BG2
        } else {
            theme::BG1
        }))
        .border(bordered(if connected { theme::LINE2 } else { theme::LINE }))
        .child(
            rect()
                .width(Size::flex(1.))
                .spacing(6.)
                .child(
                    rect()
                        .horizontal()
                        .content(Content::flex())
                        .spacing(10.)
                        .cross_align(Alignment::End)
                        .child(mono(b.app.clone(), 15., name_color).font_weight(600))
                        .child(mono(b.app_version.clone(), 12., theme::TEXT3)),
                )
                .child(text(
                    detail,
                    12.,
                    if compatible {
                        theme::TEXT2
                    } else {
                        theme::TEXT3
                    },
                )),
        )
        .child(
            rect()
                .width(Size::px(200.))
                .spacing(6.)
                .child(badge(state, fg, bg))
                .child(mono(
                    format!("{} · key {}", addr, b.fingerprint),
                    11.,
                    theme::TEXT3,
                )),
        )
        .child(button(
            action,
            if connected {
                Kind::Primary
            } else {
                Kind::Secondary
            },
            move |_| {
                if compatible {
                    ws.write().connect(addr);
                } else {
                    ws.write().notify(
                        "toast-incompatible-format",
                        tr!("toast-incompatible-format-body"),
                        None,
                        true,
                    );
                }
            },
        ))
}

fn empty_state(ws: &Workspace) -> Rect {
    let waiting = ws.scan_started.elapsed() < Duration::from_secs(4);
    let steps = [
        ("1", "sessions-step-live-title", "sessions-step-live-body"),
        ("2", "sessions-step-wifi-title", "sessions-step-wifi-body"),
        (
            "3",
            "sessions-step-firewall-title",
            "sessions-step-firewall-body",
        ),
        (
            "4",
            "sessions-step-android-title",
            "sessions-step-android-body",
        ),
    ];
    rect()
        .width(Size::fill())
        .padding(28.)
        .spacing(14.)
        .corner_radius(6.)
        .background(theme::color(theme::BG1))
        .border(bordered(theme::LINE))
        .child(
            text(
                if waiting {
                    tr!("sessions-searching")
                } else {
                    tr!("sessions-none-found")
                },
                15.,
                theme::TEXT,
            )
            .font_weight(600),
        )
        .maybe_child(ws.scan_error.as_ref().map(|e| {
            text(
                tr!("sessions-scan-failed", error = e.as_str()),
                12.,
                theme::ERROR,
            )
        }))
        .children(steps.iter().map(|(n, title, body)| {
            rect()
                .horizontal()
                .content(Content::flex())
                .spacing(12.)
                .child(mono(*n, 13., theme::TEXT3).font_weight(600))
                .child(
                    rect()
                        .spacing(4.)
                        .child(text(tr!(title), 13., theme::TEXT).font_weight(500))
                        .child(text(tr!(body), 12., theme::TEXT3)),
                )
                .into_element()
        }))
        .child(text(tr!("sessions-no-network-hint"), 12., theme::TEXT2))
}

pub fn sessions_screen(mut ws: State<Workspace>) -> Rect {
    let w = ws.read();
    let connected: Vec<_> = w
        .sessions
        .iter()
        .filter_map(|s| s.live().map(|l| l.addr))
        .collect();
    let mut apps = w.discovered.clone();
    apps.sort_by(|a, b| a.beacon.app.cmp(&b.beacon.app).then(a.addr.cmp(&b.addr)));
    let live_list = if apps.is_empty() {
        empty_state(&w).into_element()
    } else {
        rect()
            .width(Size::fill())
            .spacing(10.)
            .children(
                apps.iter()
                    .map(|app| app_card(ws, app, connected.contains(&app.addr)).into_element()),
            )
            .into_element()
    };
    let open_sessions: Vec<(usize, String)> = w
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.title()))
        .collect();
    let recent = w.prefs.recent_files();
    drop(w);

    let left = rect()
        .width(Size::flex(1.))
        .height(Size::fill())
        .content(Content::flex())
        .spacing(14.)
        .child(
            rect()
                .horizontal()
                .content(Content::flex())
                .spacing(14.)
                .cross_align(Alignment::Center)
                .child(text(tr!("sessions-live-title"), 15., theme::TEXT).font_weight(600))
                .child(mono(
                    tr!("sessions-live-broadcast", count = apps.len().to_string()),
                    12.,
                    theme::TEXT3,
                )),
        )
        .child(ScrollView::new().height(Size::flex(1.)).child(live_list))
        .child(text(tr!("sessions-live-footer"), 11., theme::TEXT3));

    let right = rect()
        .width(Size::px(500.))
        .height(Size::fill())
        .content(Content::flex())
        .spacing(14.)
        .maybe(!open_sessions.is_empty(), |r| {
            r.child(caption(&tr!("sessions-open-sessions"))).children(
                open_sessions.into_iter().map(|(i, title)| {
                    rect()
                        .width(Size::fill())
                        .padding((9., 12.))
                        .corner_radius(4.)
                        .background(theme::color(theme::BG1))
                        .cursor(CursorIcon::Pointer)
                        .on_press(move |_| ws.write().active = Some(i))
                        .child(mono(title, 12., theme::TEXT))
                        .into_element()
                }),
            )
        })
        .child(text(tr!("sessions-files"), 15., theme::TEXT).font_weight(600))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(100.))
                .center()
                .spacing(6.)
                .corner_radius(6.)
                .background(theme::color(theme::BG1))
                .border(bordered(theme::LINE2))
                .cursor(CursorIcon::Pointer)
                .on_press(move |_| dialogs::pick_file(ws))
                .child(text(tr!("sessions-drop-title"), 13., theme::TEXT).font_weight(500))
                .child(text(tr!("sessions-drop-body"), 12., theme::TEXT3)),
        )
        .maybe(!recent.is_empty(), |r| {
            r.child(caption(&tr!("sessions-recent")))
        })
        .child(
            ScrollView::new().height(Size::flex(1.)).child(
                rect()
                    .width(Size::fill())
                    .spacing(2.)
                    .children(recent.into_iter().map(|path| {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let meta = std::fs::metadata(&path).map_or(0, |m| m.len());
                        let tag = if name.contains("panic") {
                            (tr!("sessions-tag-black-box"), theme::ERROR)
                        } else {
                            (tr!("sessions-tag-recorded"), theme::TEXT2)
                        };
                        let dir = path
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let open = path.clone();
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .content(Content::flex())
                            .cross_align(Alignment::Center)
                            .padding((9., 12.))
                            .corner_radius(4.)
                            .cursor(CursorIcon::Pointer)
                            .on_press(move |_| ws.write().open_file(&open))
                            .child(
                                rect()
                                    .width(Size::flex(1.))
                                    .spacing(4.)
                                    .child(mono(name, 12., theme::TEXT))
                                    .child(text(
                                        format!("{} · {dir}", sismo_analysis::units::Bytes(meta)),
                                        11.,
                                        theme::TEXT3,
                                    )),
                            )
                            .child(badge(tag.0, tag.1, theme::BG1))
                            .into_element()
                    })),
            ),
        )
        .child(
            rect()
                .width(Size::fill())
                .padding(16.)
                .spacing(8.)
                .corner_radius(6.)
                .border(bordered(theme::LINE))
                .child(text(tr!("sessions-headless"), 13., theme::TEXT).font_weight(600))
                .child(mono(
                    format!("sismo summary {}.sismo --json", tr!("sessions-cli-file")),
                    11.,
                    theme::TEXT2,
                ))
                .child(mono(
                    format!(
                        "sismo logs {}.sismo -q level:warn+",
                        tr!("sessions-cli-file")
                    ),
                    11.,
                    theme::TEXT2,
                ))
                .child(mono(
                    format!(
                        "sismo export {}.sismo -f perfetto",
                        tr!("sessions-cli-file")
                    ),
                    11.,
                    theme::TEXT2,
                )),
        );

    rect().expanded().child(top_bar(ws)).child(
        rect()
            .expanded()
            .horizontal()
            .content(Content::flex())
            .padding((28., 32.))
            .spacing(32.)
            .child(left)
            .child(right),
    )
}

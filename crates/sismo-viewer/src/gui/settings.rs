//! Settings screen: language, recording, network, paired apps, crate
//! groups, shortcuts and the CLI for agents.

use freya::prelude::*;

use super::i18n::{self, tr};
use super::prefs::Language;
use super::theme;
use super::widgets::{Field, Kind, bordered, button, hline, kbd, mono, text, vline};
use super::{Dialog, Workspace};

/// Sections of the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Appearance,
    Recording,
    Network,
    Devices,
    Groups,
    Shortcuts,
    Agents,
}

impl Section {
    const ALL: [Section; 7] = [
        Section::Appearance,
        Section::Recording,
        Section::Network,
        Section::Devices,
        Section::Groups,
        Section::Shortcuts,
        Section::Agents,
    ];

    const fn key(self) -> &'static str {
        match self {
            Section::Appearance => "settings-appearance",
            Section::Recording => "settings-recording",
            Section::Network => "settings-network",
            Section::Devices => "settings-devices",
            Section::Groups => "settings-groups",
            Section::Shortcuts => "settings-shortcuts",
            Section::Agents => "settings-agents",
        }
    }
}

fn viewer_fingerprint() -> Option<String> {
    let path = crate::client::viewer_key_path();
    let bytes = std::fs::read(path).ok()?;
    let hex: Vec<String> = bytes
        .get(..8)?
        .chunks(2)
        .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
        .collect();
    Some(hex.join(" "))
}

fn row(label: String, value: impl IntoElement) -> Element {
    rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .padding((10., 0.))
        .child(text(label, 13., theme::TEXT2).width(Size::flex(1.)))
        .child(value)
        .into_element()
}

fn choice(
    label: String,
    on: bool,
    on_press: impl FnMut(Event<PressEventData>) + 'static,
) -> Element {
    rect()
        .padding((5., 12.))
        .corner_radius(4.)
        .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
        .border(bordered(theme::LINE2))
        .cursor(CursorIcon::Pointer)
        .on_press(on_press)
        .child(
            text(label, 12., if on { theme::TEXT } else { theme::TEXT2 }).font_weight(if on {
                600
            } else {
                400
            }),
        )
        .into_element()
}

fn appearance(mut ws: State<Workspace>) -> Element {
    let current = ws.read().prefs.language;
    let mut pick = move |language: Option<Language>| {
        let mut w = ws.write();
        w.prefs.language = language;
        w.prefs.save();
        i18n::set_language(w.prefs.language());
    };
    rect()
        .width(Size::fill())
        .spacing(4.)
        .child(row(
            tr!("settings-language"),
            rect()
                .horizontal()
                .spacing(6.)
                .child(choice(
                    tr!("settings-language-system"),
                    current.is_none(),
                    move |_| pick(None),
                ))
                .children(Language::ALL.iter().map(|l| {
                    let l = *l;
                    choice(l.native_name().to_owned(), current == Some(l), move |_| {
                        pick(Some(l));
                    })
                })),
        ))
        .child(text(tr!("settings-language-hint"), 11., theme::TEXT3))
        .into_element()
}

#[derive(PartialEq)]
struct RecordingSection(State<Workspace>);

impl Component for RecordingSection {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let initial = ws.peek().prefs.recordings_dir().display().to_string();
        let dir = use_state(move || initial);
        let window = ws.read().prefs.live_window_ns() / 1_000_000_000;
        let mut set_window = move |secs: u32| {
            let mut w = ws.write();
            w.prefs.live_window_secs = secs;
            w.prefs.save();
        };
        rect()
            .width(Size::fill())
            .spacing(4.)
            .child(row(
                tr!("settings-recordings-dir"),
                rect()
                    .horizontal()
                    .spacing(8.)
                    .cross_align(Alignment::Center)
                    .child(Field::new(dir).width(Size::px(380.)))
                    .child(button(tr!("settings-save"), Kind::Secondary, move |_| {
                        let value = dir.peek().trim().to_owned();
                        let mut w = ws.write();
                        w.prefs.recordings_dir = (!value.is_empty()).then_some(value);
                        w.prefs.save();
                    }))
                    .child(button(
                        tr!("settings-show-folder"),
                        Kind::Secondary,
                        move |_| {
                            let path = ws.peek().prefs.recordings_dir();
                            let _ = std::fs::create_dir_all(&path);
                            super::platform::reveal(&path);
                        },
                    )),
            ))
            .child(row(
                tr!("settings-live-window"),
                rect()
                    .horizontal()
                    .spacing(6.)
                    .children([3u32, 10, 30, 60].into_iter().map(|secs| {
                        choice(format!("{secs} s"), window == u64::from(secs), move |_| {
                            set_window(secs);
                        })
                    })),
            ))
            .child(text(tr!("settings-live-window-hint"), 11., theme::TEXT3))
    }
}

fn network(ws: State<Workspace>) -> Element {
    let w = ws.read();
    let scanning = w.scanner.is_some();
    let error = w.scan_error.clone();
    let found = w.discovered.len();
    drop(w);
    rect()
        .width(Size::fill())
        .spacing(4.)
        .child(row(
            tr!("settings-discovery-port"),
            mono("UDP 7701", 12., theme::TEXT),
        ))
        .child(row(
            tr!("settings-data-port"),
            mono("UDP 7720", 12., theme::TEXT),
        ))
        .child(row(
            tr!("settings-scan-status"),
            mono(
                match (&error, scanning) {
                    (Some(e), _) => e.clone(),
                    (None, true) => tr!("settings-scan-active", count = found.to_string()),
                    (None, false) => tr!("settings-scan-off"),
                },
                12.,
                if error.is_some() {
                    theme::ERROR
                } else {
                    theme::TEXT
                },
            ),
        ))
        .child(text(tr!("settings-network-hint"), 11., theme::TEXT3))
        .into_element()
}

fn devices(mut ws: State<Workspace>) -> Element {
    let paired = ws.read().prefs.paired.clone();
    let fingerprint = viewer_fingerprint();
    let empty = paired.is_empty();
    rect()
        .width(Size::fill())
        .spacing(8.)
        .child(text(tr!("settings-devices-hint"), 12., theme::TEXT2))
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .padding((6., 12.))
                .background(theme::color(theme::BG1))
                .corner_radius(4.)
                .child(text(tr!("settings-devices-app"), 11., theme::TEXT3).width(Size::px(200.)))
                .child(
                    text(tr!("settings-devices-device"), 11., theme::TEXT3).width(Size::px(220.)),
                )
                .child(
                    text(tr!("settings-devices-fingerprint"), 11., theme::TEXT3)
                        .width(Size::flex(1.)),
                ),
        )
        .maybe(empty, |r| {
            r.child(rect().padding(12.).child(text(
                tr!("settings-devices-empty"),
                12.,
                theme::TEXT3,
            )))
        })
        .children(paired.into_iter().map(|p| {
            let fp = p.fingerprint.clone();
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((8., 12.))
                .child(mono(p.app, 12., theme::TEXT).width(Size::px(200.)))
                .child(text(p.device, 12., theme::TEXT2).width(Size::px(220.)))
                .child(mono(p.fingerprint, 12., theme::TEXT3).width(Size::flex(1.)))
                .child(button(
                    tr!("settings-devices-forget"),
                    Kind::Secondary,
                    move |_| {
                        let mut w = ws.write();
                        w.prefs.paired.retain(|x| x.fingerprint != fp);
                        w.prefs.save();
                    },
                ))
                .into_element()
        }))
        .child(hline())
        .child(row(
            tr!("settings-viewer-fingerprint"),
            mono(
                fingerprint.unwrap_or_else(|| tr!("settings-viewer-no-key")),
                12.,
                theme::TEXT,
            ),
        ))
        .child(
            rect()
                .horizontal()
                .spacing(12.)
                .cross_align(Alignment::Center)
                .child(button(
                    tr!("settings-regenerate-keys"),
                    Kind::Danger,
                    move |_| {
                        {
                            let _ = std::fs::remove_file(crate::client::viewer_key_path());
                        }
                        let mut w = ws.write();
                        w.prefs.paired.clear();
                        w.prefs.save();
                        w.notify(
                            tr!("settings-keys-regenerated"),
                            tr!("settings-keys-regenerated-body"),
                            None,
                            false,
                        );
                    },
                ))
                .child(text(tr!("settings-regenerate-hint"), 11., theme::TEXT3)),
        )
        .into_element()
}

fn groups(mut ws: State<Workspace>) -> Element {
    let w = ws.read();
    let apps: Vec<(String, usize)> = w
        .prefs
        .groups
        .iter()
        .map(|g| (g.app.clone(), g.rules.len()))
        .collect();
    let active = w
        .active()
        .map(|v| v.session.info.app.clone())
        .filter(|a| !a.is_empty());
    drop(w);
    rect()
        .width(Size::fill())
        .spacing(8.)
        .child(text(tr!("settings-groups-hint"), 12., theme::TEXT2))
        .maybe(apps.is_empty(), |r| {
            r.child(text(tr!("settings-groups-empty"), 12., theme::TEXT3))
        })
        .children(apps.into_iter().map(|(app, rules)| {
            let forget = app.clone();
            rect()
                .width(Size::fill())
                .horizontal()
                .content(Content::flex())
                .cross_align(Alignment::Center)
                .padding((8., 0.))
                .child(mono(app, 12., theme::TEXT).width(Size::flex(1.)))
                .child(text(
                    tr!("settings-groups-rules", count = rules.to_string()),
                    12.,
                    theme::TEXT2,
                ))
                .child(rect().width(Size::px(12.)))
                .child(button(
                    tr!("settings-groups-reset"),
                    Kind::Secondary,
                    move |_| {
                        let mut w = ws.write();
                        w.prefs.set_groups(&forget, &[]);
                        let groups = w.prefs.groups_for(&forget);
                        for view in w
                            .sessions
                            .iter_mut()
                            .filter(|v| v.session.info.app == forget)
                        {
                            view.session.set_groups(groups.clone());
                            view.revision += 1;
                        }
                    },
                ))
                .into_element()
        }))
        .maybe_child(active.map(|app| {
            button(
                tr!("settings-groups-edit", app = app),
                Kind::Primary,
                move |_| {
                    let mut w = ws.write();
                    w.settings = None;
                    w.dialog = Some(Dialog::Groups);
                },
            )
        }))
        .into_element()
}

fn agents() -> Element {
    let exe =
        std::env::current_exe().map_or_else(|_| "sismo".to_owned(), |p| p.display().to_string());
    rect()
        .width(Size::fill())
        .spacing(8.)
        .child(text(tr!("settings-agents-hint"), 12., theme::TEXT2))
        .child(row(
            tr!("settings-agents-binary"),
            mono(exe, 12., theme::TEXT),
        ))
        .children(
            [
                "sismo summary trace.sismo --json",
                "sismo functions trace.sismo --sort p95 --json",
                "sismo logs trace.sismo -q 'level:warn+'",
                "sismo export trace.sismo -f perfetto",
                "sismo compare a.sismo b.sismo --json",
                "sismo record my_app --duration 30s",
            ]
            .into_iter()
            .map(|cmd| {
                rect()
                    .width(Size::fill())
                    .padding((6., 10.))
                    .corner_radius(4.)
                    .background(theme::color(theme::BG1))
                    .child(mono(cmd, 12., theme::TEXT2))
                    .into_element()
            }),
        )
        .into_element()
}

fn shortcuts() -> Element {
    let items: [(&str, &str); 14] = [
        ("shortcut-palette", "Ctrl K"),
        ("shortcut-open", "Ctrl O"),
        ("shortcut-settings", "Ctrl ,"),
        ("shortcut-close", "Ctrl W"),
        ("shortcut-record", "Ctrl R"),
        ("shortcut-marker", "M"),
        ("shortcut-pause", "Space"),
        ("shortcut-export", "Ctrl E"),
        ("shortcut-compare", "Ctrl D"),
        ("shortcut-zoom", "W  S"),
        ("shortcut-pan", "A  D"),
        ("shortcut-next-slow", "N"),
        ("shortcut-live", "End"),
        ("shortcut-tabs", "Ctrl 1…5"),
    ];
    rect()
        .width(Size::px(520.))
        .spacing(2.)
        .children(
            items
                .into_iter()
                .map(|(key, keys)| row(tr!(key), kbd(keys))),
        )
        .into_element()
}

/// Settings floating over whatever screen is open; a click outside or
/// Esc closes it.
pub fn modal(mut ws: State<Workspace>) -> Rect {
    rect()
        .position(Position::new_global().top(0.).left(0.))
        .width(Size::window_percent(100.))
        .height(Size::window_percent(100.))
        .layer(Layer::OverlayLevel(super::widgets::layers::SETTINGS))
        .background(Color::from_argb(140, 0, 0, 0))
        .center()
        .on_press(move |_| ws.write().settings = None)
        .child(
            rect()
                .width(Size::window_percent(86.))
                .height(Size::window_percent(84.))
                .corner_radius(8.)
                .overflow(Overflow::Clip)
                .background(theme::color(theme::BG0))
                .border(super::widgets::bordered(theme::LINE2))
                .on_press(|e: Event<PressEventData>| e.stop_propagation())
                .child(SettingsScreen(ws)),
        )
}

#[derive(PartialEq)]
pub struct SettingsScreen(pub State<Workspace>);

impl Component for SettingsScreen {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let current = ws.read().settings.unwrap_or_default();
        let body = match current {
            Section::Appearance => appearance(ws),
            Section::Recording => RecordingSection(ws).into_element(),
            Section::Network => network(ws),
            Section::Devices => devices(ws),
            Section::Groups => groups(ws),
            Section::Shortcuts => shortcuts(),
            Section::Agents => agents(),
        };
        rect()
            .expanded()
            .content(Content::flex())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(44.))
                    .horizontal()
                    .content(Content::flex())
                    .cross_align(Alignment::Center)
                    .padding((0., 16.))
                    .spacing(16.)
                    .background(theme::color(theme::BG1))
                    .child(
                        text(tr!("settings-title"), 13., theme::TEXT)
                            .font_weight(500)
                            .width(Size::flex(1.)),
                    )
                    .child(button(tr!("settings-close"), Kind::Secondary, move |_| {
                        ws.write().settings = None;
                    })),
            )
            .child(hline())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.))
                    .horizontal()
                    .content(Content::flex())
                    .child(
                        rect()
                            .width(Size::px(248.))
                            .height(Size::fill())
                            .padding(8.)
                            .spacing(2.)
                            .background(theme::color(theme::BG1))
                            .children(Section::ALL.iter().map(|s| {
                                let s = *s;
                                let on = s == current;
                                rect()
                                    .width(Size::fill())
                                    .padding((7., 12.))
                                    .corner_radius(4.)
                                    .background(theme::color(if on {
                                        theme::BG3
                                    } else {
                                        theme::BG1
                                    }))
                                    .cursor(CursorIcon::Pointer)
                                    .on_press(move |_| ws.write().settings = Some(s))
                                    .child(
                                        text(
                                            tr!(s.key()),
                                            13.,
                                            if on { theme::TEXT } else { theme::TEXT2 },
                                        )
                                        .font_weight(if on { 600 } else { 400 }),
                                    )
                                    .into_element()
                            })),
                    )
                    .child(vline())
                    .child(
                        ScrollView::new()
                            .width(Size::flex(1.))
                            .height(Size::fill())
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .padding((32., 40.))
                                    .spacing(16.)
                                    .child(
                                        text(tr!(current.key()), 20., theme::TEXT).font_weight(600),
                                    )
                                    .child(body),
                            ),
                    ),
            )
    }
}

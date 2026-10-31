//! Modal dialogs, toasts and the file pickers behind them.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Duration;

use freya::prelude::*;
use sismo_analysis::units::Bytes;
use sismo_analysis::{ExportOptions, Format};

use super::i18n::tr;
use super::theme;
use super::widgets::{
    Kind, bordered, button, caption, hline, kbd, layers, mono, panel, scrim, text,
};
use super::{Dialog, Toast, Workspace};
use crate::client::Command;
use crate::workspace::{LiveStatus, RecordFrom, SessionView, StopRule, Tab};

/// Opens the native picker and loads the chosen recording.
pub fn pick_file(mut ws: State<Workspace>) {
    spawn_in_window(async move {
        if let Some(path) = super::platform::pick_recording(tr!("dialog-pick-recording")).await {
            ws.write().open_file(&path);
        }
    });
}

/// Stops the running recording, or opens the record dialog.
pub fn toggle_record(mut ws: State<Workspace>) {
    let mut w = ws.write();
    let Some(view) = w.active_mut() else { return };
    if view.live().is_none() {
        return;
    }
    if view.live().is_some_and(|l| l.recording.is_some()) {
        if let Some(saved) = view.stop_recording() {
            w.recording_saved(&saved);
        }
    } else {
        w.dialog = Some(Dialog::Record);
    }
}

/// Saves everything received from a live app so far.
pub fn save_received(mut ws: State<Workspace>) {
    spawn_in_window(async move {
        let name = ws.peek().active().map_or_else(
            || "sismo.sismo".into(),
            |v| format!("{}.sismo", v.file_stem()),
        );
        let Some(path) =
            super::platform::save_path(tr!("dialog-save-received"), name, "sismo").await
        else {
            return;
        };
        let result = ws.peek().active().map(|v| v.save_received(&path));
        let mut w = ws.write();
        match result {
            Some(Ok(bytes)) => w.notify("toast-saved", Bytes(bytes).to_string(), Some(path), false),
            Some(Err(error)) => w.notify("toast-save-failed", error.to_string(), None, true),
            None => {}
        }
    });
}

fn close(mut ws: State<Workspace>) {
    ws.write().dialog = None;
}

fn option_row(
    label: &str,
    sub: &str,
    on: bool,
    on_press: impl FnMut(Event<PressEventData>) + 'static,
) -> Element {
    rect()
        .width(Size::fill())
        .horizontal()
        .spacing(10.)
        .padding((4., 0.))
        .cursor(CursorIcon::Pointer)
        .on_press(on_press)
        .child(
            rect()
                .width(Size::px(14.))
                .height(Size::px(14.))
                .corner_radius(7.)
                .center()
                .border(bordered(if on { theme::TEXT } else { theme::LINE2 }))
                .maybe(on, |r| {
                    r.child(
                        rect()
                            .width(Size::px(6.))
                            .height(Size::px(6.))
                            .corner_radius(3.)
                            .background(theme::color(theme::TEXT)),
                    )
                }),
        )
        .child(
            rect()
                .spacing(2.)
                .child(text(label, 12., theme::TEXT))
                .maybe(!sub.is_empty(), |r| r.child(text(sub, 11., theme::TEXT3))),
        )
        .into_element()
}

fn field(value: State<String>, placeholder: String, width: f32) -> Rect {
    rect()
        .width(Size::px(width))
        .height(Size::px(32.))
        .corner_radius(4.)
        .background(theme::color(theme::BG0))
        .border(bordered(theme::LINE2))
        .center()
        .font_family(theme::MONO)
        .font_size(12.)
        .child(
            Input::new(value)
                .flat()
                .auto_focus(true)
                .width(Size::px(width - 4.))
                .placeholder(placeholder),
        )
}

fn title_row(title: &str) -> Rect {
    rect()
        .width(Size::fill())
        .horizontal()
        .content(Content::flex())
        .cross_align(Alignment::Center)
        .child(
            text(title, 15., theme::TEXT)
                .font_weight(600)
                .width(Size::flex(1.)),
        )
        .child(kbd("Esc"))
}

fn footer(children: Vec<Element>) -> Rect {
    rect()
        .width(Size::fill())
        .horizontal()
        .main_align(Alignment::End)
        .spacing(8.)
        .children(children)
}

#[derive(PartialEq)]
struct PairDialog(State<Workspace>);

impl Component for PairDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let code = use_state(String::new);
        let w = ws.read();
        let view = w.active();
        let app = view.map(|v| v.session.info.app.clone()).unwrap_or_default();
        let rejected = view.and_then(|v| v.live()).and_then(|l| match &l.status {
            LiveStatus::NeedsPairing { rejected } => rejected.clone(),
            _ => None,
        });
        drop(w);
        let mut submit = move || {
            let value = code.peek().trim().to_owned();
            let mut w = ws.write();
            if let Some(v) = w.active_mut() {
                v.send(Command::Pair(value));
            }
            w.dialog = None;
        };
        panel(420.)
            .padding(24.)
            .spacing(14.)
            .child(title_row(&tr!("dialog-pair-title", app = app)))
            .child(text(tr!("dialog-pair-body"), 12., theme::TEXT2).max_lines(3))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(44.))
                    .corner_radius(6.)
                    .background(theme::color(theme::BG0))
                    .border(bordered(theme::TEXT2))
                    .center()
                    .font_family(theme::MONO)
                    .font_size(22.)
                    .child(
                        Input::new(code)
                            .flat()
                            .auto_focus(true)
                            .width(Size::px(360.))
                            .placeholder("000000")
                            .on_submit(move |_| submit()),
                    ),
            )
            .maybe_child(rejected.map(|r| text(r, 12., theme::ERROR)))
            .child(text(tr!("dialog-pair-remembered"), 11., theme::TEXT3))
            .child(footer(vec![
                button(tr!("common-cancel"), Kind::Secondary, move |_| close(ws)).into_element(),
                button(tr!("dialog-pair-submit"), Kind::Primary, move |_| submit()).into_element(),
            ]))
    }
}

#[derive(PartialEq)]
struct RecordDialog(State<Workspace>);

impl Component for RecordDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let default_path = {
            let w = ws.peek();
            let stem = w
                .active()
                .map_or_else(|| "sismo".into(), SessionView::file_stem);
            w.prefs
                .recordings_dir()
                .join(format!("{stem}.sismo"))
                .display()
                .to_string()
        };
        let mut from = use_state(|| RecordFrom::Buffer);
        let mut stop = use_state(|| 0usize);
        let mut on_device = use_state(|| false);
        let path = use_state(move || default_path);
        let buffered = ws
            .read()
            .active()
            .and_then(|v| v.live())
            .map_or(0, |l| l.received_bytes);
        let stops = [
            ("dialog-stop-manual", StopRule::Manual),
            ("dialog-stop-5min", StopRule::After(Duration::from_mins(5))),
            ("dialog-stop-500mb", StopRule::Size(500 * 1024 * 1024)),
            ("dialog-stop-first-error", StopRule::FirstError),
        ];
        let current_stop = *stop.read();
        let start = move |_| {
            let target = PathBuf::from(path.peek().clone());
            let rule = stops[current_stop].1;
            let mut w = ws.write();
            let result = w
                .active_mut()
                .map(|v| v.start_recording(target.clone(), *from.peek(), rule, *on_device.peek()));
            w.dialog = None;
            if let Some(Err(error)) = result {
                w.notify("toast-record-failed", error.to_string(), None, true);
            }
        };
        panel(560.)
            .padding(24.)
            .spacing(12.)
            .child(title_row(&tr!("dialog-record-title")))
            .child(text(tr!("dialog-record-body"), 12., theme::TEXT2))
            .child(caption(&tr!("dialog-record-from")))
            .child(option_row(
                &tr!("dialog-record-include-buffer"),
                &tr!(
                    "dialog-record-include-buffer-sub",
                    size = Bytes(buffered).to_string()
                ),
                *from.read() == RecordFrom::Buffer,
                move |_| from.set(RecordFrom::Buffer),
            ))
            .child(option_row(
                &tr!("dialog-record-from-now"),
                "",
                *from.read() == RecordFrom::Now,
                move |_| from.set(RecordFrom::Now),
            ))
            .child(caption(&tr!("dialog-record-stop")))
            .child(
                rect()
                    .horizontal()
                    .spacing(6.)
                    .children(stops.iter().enumerate().map(|(i, (label, _))| {
                        let on = i == current_stop;
                        rect()
                            .padding((5., 12.))
                            .corner_radius(4.)
                            .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
                            .border(bordered(theme::LINE2))
                            .cursor(CursorIcon::Pointer)
                            .on_press(move |_| stop.set(i))
                            .child(
                                text(tr!(label), 12., if on { theme::TEXT } else { theme::TEXT2 })
                                    .font_weight(if on { 600 } else { 400 }),
                            )
                            .into_element()
                    })),
            )
            .child(text(
                tr!("dialog-record-first-error-hint"),
                11.,
                theme::TEXT3,
            ))
            .child(caption(&tr!("dialog-file")))
            .child(field(path, tr!("dialog-file-path"), 512.))
            .child(option_row(
                &tr!("dialog-record-on-device"),
                &tr!("dialog-record-on-device-sub"),
                *on_device.read(),
                move |_| on_device.toggle(),
            ))
            .child(hline())
            .child(footer(vec![
                button(tr!("common-cancel"), Kind::Secondary, move |_| close(ws)).into_element(),
                button(tr!("dialog-record-start"), Kind::Primary, start).into_element(),
            ]))
    }
}

const FORMATS: [(Format, &str, &str); 5] = [
    (
        Format::Perfetto,
        "dialog-format-perfetto",
        "dialog-format-perfetto-sub",
    ),
    (
        Format::SummaryJson,
        "dialog-format-summary",
        "dialog-format-summary-sub",
    ),
    (
        Format::Markdown,
        "dialog-format-markdown",
        "dialog-format-markdown-sub",
    ),
    (
        Format::Chrome,
        "dialog-format-chrome",
        "dialog-format-chrome-sub",
    ),
    (
        Format::Sismo,
        "dialog-format-sismo",
        "dialog-format-sismo-sub",
    ),
];

#[derive(PartialEq)]
struct ExportDialog(State<Workspace>);

impl Component for ExportDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let mut format = use_state(|| 0usize);
        let mut scope = use_state(|| 0usize);
        let mut logs = use_state(|| true);
        let mut strip = use_state(|| false);
        let has_selection = ws.read().active().is_some_and(|v| v.selection.is_some());
        let base = ws
            .peek()
            .active()
            .map_or_else(|| "sismo".into(), SessionView::file_stem);
        let export_dir = ws.peek().prefs.recordings_dir();
        let current = *format.read();
        let ext = FORMATS[current].0.extension();
        let mut path = use_state(move || {
            export_dir
                .join(format!("{base}.pftrace"))
                .display()
                .to_string()
        });
        let shown = path.read().clone();
        if !shown.ends_with(&format!(".{ext}")) {
            let fixed = PathBuf::from(&shown)
                .with_extension(ext)
                .display()
                .to_string();
            path.set(fixed);
        }
        let run = move |_| {
            let target = PathBuf::from(path.peek().clone());
            let mut w = ws.write();
            let Some(v) = w.active() else { return };
            let window = match *scope.peek() {
                1 => v.selection,
                _ => None,
            };
            let options = ExportOptions {
                query: if *scope.peek() == 2 {
                    v.effective_query()
                } else {
                    sismo_analysis::Query::default()
                },
                window: window.or_else(|| Some(v.session.span()).map(|(a, b)| (a, b + 1))),
                include_logs: *logs.peek(),
                strip_fields: *strip.peek(),
            };
            let result = target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| File::create(&target))
                .and_then(|f| {
                    v.session
                        .export(FORMATS[current].0, &options, &mut BufWriter::new(f))
                });
            w.dialog = None;
            match result {
                Ok(()) => {
                    let size = std::fs::metadata(&target).map_or(0, |m| m.len());
                    w.notify(
                        "toast-exported",
                        Bytes(size).to_string(),
                        Some(target),
                        false,
                    );
                }
                Err(error) => w.notify("toast-export-failed", error.to_string(), None, true),
            }
        };
        panel(620.)
            .padding(24.)
            .spacing(10.)
            .child(title_row(&tr!("common-export")))
            .child(caption(&tr!("dialog-export-format")))
            .children(FORMATS.iter().enumerate().map(|(i, (_, label, sub))| {
                option_row(&tr!(label), &tr!(sub), i == current, move |_| format.set(i))
            }))
            .child(caption(&tr!("dialog-export-scope")))
            .child(option_row(
                &tr!("dialog-export-scope-session"),
                "",
                *scope.read() == 0,
                move |_| scope.set(0),
            ))
            .maybe(has_selection, |r| {
                r.child(option_row(
                    &tr!("dialog-export-scope-selection"),
                    "",
                    *scope.read() == 1,
                    move |_| scope.set(1),
                ))
            })
            .child(option_row(
                &tr!("dialog-export-scope-query"),
                "",
                *scope.read() == 2,
                move |_| scope.set(2),
            ))
            .child(option_row(
                &tr!("dialog-export-include-logs"),
                "",
                *logs.read(),
                move |_| {
                    logs.toggle();
                },
            ))
            .child(option_row(
                &tr!("dialog-export-strip-fields"),
                &tr!("dialog-export-strip-fields-sub"),
                *strip.read(),
                move |_| strip.toggle(),
            ))
            .child(caption(&tr!("dialog-file")))
            .child(field(path, tr!("dialog-file-path"), 572.))
            .child(hline())
            .child(footer(vec![
                button(tr!("common-cancel"), Kind::Secondary, move |_| close(ws)).into_element(),
                button(tr!("common-export"), Kind::Primary, run).into_element(),
            ]))
    }
}

#[derive(PartialEq)]
struct FilterDialog(State<Workspace>);

impl Component for FilterDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let current = ws
            .peek()
            .active()
            .and_then(|v| v.live())
            .map(|l| l.app_filter.clone())
            .unwrap_or_default();
        let value = use_state(move || current);
        let parsed = sismo_format::Filter::parse(&value.read());
        let error = parsed.as_ref().err().map(ToString::to_string);
        let preview: Vec<(String, String)> = parsed
            .map(|f| {
                std::iter::once((
                    tr!("dialog-filter-default"),
                    f.default_level().as_str().to_owned(),
                ))
                .chain(
                    f.directives()
                        .map(|(t, l)| (t.to_owned(), l.as_str().to_owned())),
                )
                .collect()
            })
            .unwrap_or_default();
        let ok = error.is_none();
        panel(520.)
            .padding(24.)
            .spacing(12.)
            .child(title_row(&tr!("dialog-filter-title")))
            .child(text(tr!("dialog-filter-body"), 12., theme::TEXT2))
            .child(field(value, tr!("dialog-filter-placeholder"), 472.))
            .maybe_child(error.map(|e| text(e, 11., theme::ERROR)))
            .child(caption(&tr!("dialog-filter-levels")))
            .children(preview.into_iter().map(|(t, l)| {
                rect()
                    .horizontal()
                    .spacing(12.)
                    .child(mono(t, 12., theme::TEXT).width(Size::px(300.)))
                    .child(mono(l, 12., theme::TEXT2))
                    .into_element()
            }))
            .child(hline())
            .child(footer(vec![
                button(tr!("common-cancel"), Kind::Secondary, move |_| close(ws)).into_element(),
                button(tr!("dialog-filter-apply"), Kind::Primary, move |_| {
                    if !ok {
                        return;
                    }
                    let filter = value.peek().clone();
                    let mut w = ws.write();
                    if let Some(v) = w.active_mut() {
                        v.send(Command::SetFilter(filter));
                    }
                    w.dialog = None;
                })
                .into_element(),
            ]))
    }
}

#[derive(PartialEq)]
struct MarkerDialog(State<Workspace>);

impl Component for MarkerDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let note = use_state(String::new);
        let mut save = move || {
            let text = note.peek().trim().to_owned();
            let mut w = ws.write();
            if let Some(v) = w.active_mut()
                && !text.is_empty()
            {
                v.add_marker(text);
            }
            w.dialog = None;
        };
        panel(380.)
            .padding(20.)
            .spacing(12.)
            .child(title_row(&tr!("dialog-marker-title")))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(32.))
                    .corner_radius(4.)
                    .background(theme::color(theme::BG0))
                    .border(bordered(theme::TEXT2))
                    .center()
                    .child(
                        Input::new(note)
                            .flat()
                            .auto_focus(true)
                            .width(Size::px(330.))
                            .placeholder(tr!("dialog-marker-placeholder"))
                            .on_submit(move |_| save()),
                    ),
            )
            .child(text(tr!("dialog-marker-body"), 11., theme::TEXT3))
            .child(footer(vec![
                button(tr!("dialog-marker-save"), Kind::Primary, move |_| save()).into_element(),
            ]))
    }
}

/// Asks before forgetting what a live app sent, offering to save it first.
#[derive(PartialEq)]
struct ClearDialog(State<Workspace>);

/// Clears the active live session, saving it first into the recordings
/// folder when `save` is set. Nothing is cleared when the save fails.
fn clear_session(mut ws: State<Workspace>, save: bool) {
    let mut w = ws.write();
    w.dialog = None;
    let dir = w.prefs.recordings_dir();
    let Some(view) = w.active_mut() else { return };
    let saved = if save {
        let path = dir.join(format!("{}.sismo", view.file_stem()));
        let result = std::fs::create_dir_all(&dir).and_then(|()| view.save_received(&path));
        match result {
            Ok(bytes) => Some((path, bytes)),
            Err(error) => {
                w.notify("toast-save-failed", error.to_string(), None, true);
                return;
            }
        }
    } else {
        None
    };
    let cleared = view.clear_received();
    match (cleared, saved) {
        (Err(error), _) => w.notify("toast-clear-failed", error.to_string(), None, true),
        (Ok(()), Some((path, bytes))) => {
            w.notify("toast-saved", Bytes(bytes).to_string(), Some(path), false);
        }
        (Ok(()), None) => w.notify("toast-cleared", String::new(), None, false),
    }
}

impl Component for ClearDialog {
    fn render(&self) -> impl IntoElement {
        let ws = self.0;
        let body = ws.read().active().map_or_else(String::new, |v| {
            let (start, end) = v.session.span();
            tr!(
                "dialog-clear-body",
                app = v.session.info.app.as_str(),
                size = Bytes(v.live().map_or(0, |l| l.received_bytes)).to_string(),
                duration = sismo_analysis::units::Nanos(end.saturating_sub(start)).to_string(),
            )
        });
        let folder = ws.read().prefs.recordings_dir().display().to_string();
        panel(460.)
            .padding(20.)
            .spacing(12.)
            .child(title_row(&tr!("dialog-clear-title")))
            .child(
                text(body, 12., theme::TEXT2)
                    .max_lines(6)
                    .width(Size::fill()),
            )
            .child(
                text(
                    tr!("dialog-clear-save-hint", folder = folder),
                    11.,
                    theme::TEXT3,
                )
                .max_lines(3)
                .width(Size::fill()),
            )
            .child(footer(vec![
                button(tr!("common-cancel"), Kind::Secondary, move |_| close(ws)).into_element(),
                button(tr!("dialog-clear-discard"), Kind::Danger, move |_| {
                    clear_session(ws, false);
                })
                .into_element(),
                button(tr!("dialog-clear-save"), Kind::Primary, move |_| {
                    clear_session(ws, true);
                })
                .into_element(),
            ]))
    }
}

/// Asks what to do with the current run when the live app restarted.
#[derive(PartialEq)]
struct RestartDialog(State<Workspace>);

/// Moves the active live tab to the app's new run. With `save`, the run it
/// showed is saved to the recordings folder first and opened in its own
/// tab, so nothing disappears; the live tab stays the active one.
fn follow_new_run(mut ws: State<Workspace>, save: bool) {
    let mut w = ws.write();
    w.dialog = None;
    let Some(index) = w.active else { return };
    let dir = w.prefs.recordings_dir();
    let mut kept = None;
    if save {
        let view = &w.sessions[index];
        let path = dir.join(format!("{}.sismo", view.file_stem()));
        match std::fs::create_dir_all(&dir).and_then(|()| view.save_received(&path)) {
            Ok(_) => kept = Some(path),
            Err(error) => {
                w.notify("toast-save-failed", error.to_string(), None, true);
                return;
            }
        }
    }
    let recording = w.sessions[index].switch_to_next_run();
    if let Some(done) = recording {
        w.recording_saved(&done);
    }
    if let Some(path) = kept {
        w.open_file(&path);
        w.active = Some(index);
        w.notify(
            "toast-run-kept",
            path.display().to_string(),
            Some(path),
            false,
        );
    }
}

impl Component for RestartDialog {
    fn render(&self) -> impl IntoElement {
        let ws = self.0;
        let body = ws.read().active().map_or_else(String::new, |v| {
            let (start, end) = v.session.span();
            tr!(
                "dialog-restart-body",
                app = v.session.info.app.as_str(),
                device = v.session.info.device.as_str(),
                size = Bytes(v.live().map_or(0, |l| l.received_bytes)).to_string(),
                duration = sismo_analysis::units::Nanos(end.saturating_sub(start)).to_string(),
            )
        });
        let folder = ws.read().prefs.recordings_dir().display().to_string();
        panel(480.)
            .padding(20.)
            .spacing(12.)
            .child(title_row(&tr!("dialog-restart-title")))
            .child(
                text(body, 12., theme::TEXT2)
                    .max_lines(6)
                    .width(Size::fill()),
            )
            .child(
                text(
                    tr!("dialog-restart-save-hint", folder = folder),
                    11.,
                    theme::TEXT3,
                )
                .max_lines(3)
                .width(Size::fill()),
            )
            .child(footer(vec![
                button(tr!("dialog-restart-discard"), Kind::Danger, move |_| {
                    follow_new_run(ws, false);
                })
                .into_element(),
                button(tr!("dialog-restart-save"), Kind::Primary, move |_| {
                    follow_new_run(ws, true);
                })
                .into_element(),
            ]))
    }
}

/// Command palette entries: label, shortcut, action id.
const ACTIONS: [(&str, &str, u8); 14] = [
    ("palette-compare", "Ctrl D", 11),
    ("sessions-open-file", "Ctrl O", 0),
    ("palette-record-toggle", "Ctrl R", 1),
    ("shell-export", "Ctrl E", 2),
    ("palette-go-timeline", "Ctrl 1", 3),
    ("palette-go-flamegraph", "Ctrl 2", 4),
    ("palette-go-functions", "Ctrl 3", 5),
    ("palette-go-logs", "Ctrl 4", 6),
    ("palette-go-metrics", "Ctrl 5", 7),
    ("palette-app-filter", "", 8),
    ("palette-back-to-sessions", "", 9),
    ("palette-shortcuts", "?", 10),
    ("settings-title", "Ctrl ,", 12),
    ("settings-groups", "", 13),
];

fn run_action(mut ws: State<Workspace>, id: u8) {
    ws.write().dialog = None;
    match id {
        0 => pick_file(ws),
        1 => toggle_record(ws),
        2 => ws.write().dialog = Some(Dialog::Export),
        3..=7 => {
            if let Some(v) = ws.write().active_mut() {
                v.tab = Tab::ALL[usize::from(id - 3)];
            }
        }
        8 => ws.write().dialog = Some(Dialog::AppFilter),
        9 => ws.write().active = None,
        11 => pick_compare(ws),
        12 => ws.write().settings = Some(super::settings::Section::default()),
        13 => {
            if ws.peek().active().is_some() {
                ws.write().dialog = Some(Dialog::Groups);
            } else {
                ws.write().settings = Some(super::settings::Section::Groups);
            }
        }
        _ => ws.write().dialog = Some(Dialog::Shortcuts),
    }
}

#[derive(PartialEq)]
struct PaletteDialog(State<Workspace>);

impl Component for PaletteDialog {
    fn render(&self) -> impl IntoElement {
        let mut ws = self.0;
        let query = use_state(String::new);
        let needle = query.read().to_lowercase();
        let actions: Vec<(String, &str, u8)> = ACTIONS
            .iter()
            .map(|(key, keys, id)| (tr!(key), *keys, *id))
            .filter(|(l, _, _)| l.to_lowercase().contains(&needle))
            .collect();
        let functions: Vec<(u32, String)> = if needle.len() < 2 {
            Vec::new()
        } else {
            ws.read()
                .active()
                .map(|v| {
                    v.session
                        .callsites()
                        .filter(|c| {
                            c.kind == sismo_format::CallsiteKind::Span
                                && c.name.to_lowercase().contains(&needle)
                        })
                        .take(8)
                        .map(|c| (c.id, c.name.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let first = actions.first().map(|a| a.2);
        panel(600.)
            .padding(8.)
            .spacing(4.)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(40.))
                    .center()
                    .font_size(15.)
                    .child(
                        Input::new(query)
                            .flat()
                            .auto_focus(true)
                            .width(Size::px(580.))
                            .placeholder(tr!("palette-placeholder"))
                            .on_submit(move |_| {
                                if let Some(id) = first {
                                    run_action(ws, id);
                                }
                            }),
                    ),
            )
            .child(hline())
            .maybe(!actions.is_empty(), |r| {
                r.child(
                    rect()
                        .padding((6., 8.))
                        .child(caption(&tr!("palette-actions"))),
                )
            })
            .children(
                actions
                    .into_iter()
                    .enumerate()
                    .map(|(i, (label, keys, id))| {
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .content(Content::flex())
                            .cross_align(Alignment::Center)
                            .padding((7., 10.))
                            .corner_radius(4.)
                            .background(theme::color(if i == 0 { theme::BG3 } else { theme::BG1 }))
                            .cursor(CursorIcon::Pointer)
                            .on_press(move |_| run_action(ws, id))
                            .child(text(label, 13., theme::TEXT).width(Size::flex(1.)))
                            .maybe(!keys.is_empty(), |r| r.child(kbd(keys)))
                            .into_element()
                    }),
            )
            .maybe(!functions.is_empty(), |r| {
                r.child(
                    rect()
                        .padding((6., 8.))
                        .child(caption(&tr!("tab-functions"))),
                )
            })
            .children(functions.into_iter().map(|(id, name)| {
                rect()
                    .width(Size::fill())
                    .padding((7., 10.))
                    .corner_radius(4.)
                    .cursor(CursorIcon::Pointer)
                    .on_press(move |_| {
                        let mut w = ws.write();
                        if let Some(v) = w.active_mut() {
                            v.selected_function = Some(id);
                            v.tab = Tab::Functions;
                        }
                        w.dialog = None;
                    })
                    .child(mono(name, 12., theme::TEXT))
                    .into_element()
            }))
            .child(
                rect()
                    .padding((6., 10.))
                    .child(text(tr!("palette-hint"), 11., theme::TEXT3)),
            )
    }
}

fn shortcuts() -> Rect {
    let groups: [(&str, &[(&str, &str)]); 4] = [
        (
            "shortcuts-general",
            &[
                ("shortcuts-palette", "Ctrl K"),
                ("shortcuts-open-file", "Ctrl O"),
                ("shortcuts-close-session", "Ctrl W"),
                ("shortcuts-these", "?"),
            ],
        ),
        (
            "shortcuts-session",
            &[
                ("palette-record-toggle", "Ctrl R"),
                ("dialog-marker-title", "M"),
                ("shortcuts-pause-resume", "common-space"),
                ("common-export", "Ctrl E"),
                ("palette-compare", "Ctrl D"),
            ],
        ),
        (
            "tab-timeline",
            &[
                ("shortcuts-zoom", "W  S"),
                ("shortcuts-pan", "A  D"),
                ("shortcuts-range", "shortcuts-key-drag"),
                ("shortcuts-next-slow", "N"),
                ("shortcuts-back-to-live", "End"),
            ],
        ),
        (
            "shortcuts-lists",
            &[
                ("shortcuts-pick", "shortcuts-key-click"),
                ("shortcuts-tab", "Ctrl 1…5"),
                ("shortcuts-clear-selection", "Esc"),
            ],
        ),
    ];
    panel(960.)
        .padding(24.)
        .spacing(16.)
        .child(title_row(&tr!("shortcuts-title")))
        .child(
            rect()
                .horizontal()
                .spacing(24.)
                .children(groups.iter().map(|(title, items)| {
                    rect()
                        .width(Size::px(210.))
                        .spacing(10.)
                        .child(caption(&tr!(title)))
                        .children(items.iter().map(|(label, keys)| {
                            rect()
                                .width(Size::fill())
                                .horizontal()
                                .content(Content::flex())
                                .child(text(tr!(label), 12., theme::TEXT).width(Size::flex(1.)))
                                .child(kbd(&tr!(keys)))
                                .into_element()
                        }))
                        .into_element()
                })),
        )
}

pub fn render(ws: State<Workspace>, dialog: &Dialog) -> Rect {
    let content: Element = match dialog {
        Dialog::Pair { .. } => PairDialog(ws).into_element(),
        Dialog::Record => RecordDialog(ws).into_element(),
        Dialog::Export => ExportDialog(ws).into_element(),
        Dialog::AppFilter => FilterDialog(ws).into_element(),
        Dialog::Palette { .. } => PaletteDialog(ws).into_element(),
        Dialog::Shortcuts => shortcuts().into_element(),
        Dialog::Marker { .. } => MarkerDialog(ws).into_element(),
        Dialog::Groups => super::groups::GroupsDialog(ws).into_element(),
        Dialog::ClearSession => ClearDialog(ws).into_element(),
        Dialog::AppRestarted => RestartDialog(ws).into_element(),
    };
    scrim(move |_| close(ws), content)
}

pub fn toast(mut ws: State<Workspace>, toast: &Toast) -> Rect {
    let path = toast.path.clone();
    let folder = path
        .as_ref()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let openable = path
        .as_ref()
        .is_some_and(|p| p.extension().is_some_and(|e| e == "sismo"));
    rect()
        .position(Position::new_global().bottom(40.).right(16.))
        .layer(Layer::OverlayLevel(layers::TOAST))
        .width(Size::px(420.))
        .padding(16.)
        .spacing(8.)
        .corner_radius(8.)
        .background(theme::color(theme::BG1))
        .border(bordered(if toast.error {
            theme::ERROR
        } else {
            theme::LINE2
        }))
        .shadow(
            Shadow::new()
                .y(8.)
                .blur(24.)
                .color(Color::from_argb(90, 0, 0, 0)),
        )
        .child(
            text(
                tr!(&toast.title),
                13.,
                if toast.error {
                    theme::ERROR
                } else {
                    theme::TEXT
                },
            )
            .font_weight(600),
        )
        .maybe_child(
            path.as_ref()
                .map(|p| mono(p.display().to_string(), 12., theme::TEXT2)),
        )
        .child(text(toast.body.clone(), 11., theme::TEXT3))
        .child(
            rect()
                .horizontal()
                .spacing(8.)
                .maybe(openable, |r| {
                    let path = path.clone();
                    r.child(button(tr!("common-open"), Kind::Secondary, move |_| {
                        if let Some(p) = &path {
                            let mut w = ws.write();
                            w.toast = None;
                            w.open_file(p);
                        }
                    }))
                })
                .maybe_child(folder.map(|dir| {
                    button(tr!("toast-show-in-folder"), Kind::Secondary, move |_| {
                        super::platform::reveal(&dir);
                    })
                }))
                .child(button(tr!("common-close"), Kind::Secondary, move |_| {
                    ws.write().toast = None;
                })),
        )
}

/// Picks a recording to compare the active session against.
pub fn pick_compare(mut ws: State<Workspace>) {
    if ws.peek().active().is_none() {
        return;
    }
    spawn_in_window(async move {
        let Some(path) = super::platform::pick_recording(tr!("dialog-pick-compare")).await else {
            return;
        };
        match sismo_analysis::Session::load(&path) {
            Ok((session, _)) => {
                if let Some(v) = ws.write().active_mut() {
                    v.compare = Some((path, Box::new(session)));
                    v.tab = Tab::Functions;
                }
            }
            Err(error) => ws
                .write()
                .notify("toast-open-failed", error.to_string(), None, true),
        }
    });
}

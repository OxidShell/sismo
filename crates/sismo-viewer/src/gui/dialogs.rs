//! Modal dialogs, toasts and the file pickers behind them.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Duration;

use freya::prelude::*;
use sismo_analysis::units::Bytes;
use sismo_analysis::{ExportOptions, Format};

use super::theme;
use super::widgets::{Kind, bordered, button, caption, hline, kbd, mono, panel, scrim, text};
use super::{Dialog, Toast, Workspace};
use crate::client::Command;
use crate::workspace::{LiveStatus, RecordFrom, StopRule, Tab, recordings_dir};

/// Opens the native picker and loads the chosen recording.
pub fn pick_file(mut ws: State<Workspace>) {
    spawn_in_window(async move {
        if let Some(path) = super::platform::pick_recording("Abrir grabación de sismo".into()).await
        {
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
            || "recibido.sismo".into(),
            |v| format!("{}-recibido.sismo", v.session.info.app),
        );
        let Some(path) =
            super::platform::save_path("Guardar lo recibido".into(), name, "sismo").await
        else {
            return;
        };
        let result = ws.peek().active().map(|v| v.save_received(&path));
        let mut w = ws.write();
        match result {
            Some(Ok(bytes)) => w.notify("Guardado", Bytes(bytes).to_string(), Some(path), false),
            Some(Err(error)) => w.notify("No se pudo guardar", error.to_string(), None, true),
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

fn field(value: State<String>, placeholder: &'static str, width: f32) -> Rect {
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
            .child(title_row(&format!("Emparejar con {app}")))
            .child(text("Escribe el código de 6 dígitos que la app muestra en su log (target sismo) o en su pantalla de diagnóstico.", 12., theme::TEXT2).max_lines(3))
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
                    .child(Input::new(code).flat().auto_focus(true).width(Size::px(360.)).placeholder("000000").on_submit(move |_| submit())),
            )
            .maybe_child(rejected.map(|r| text(r, 12., theme::ERROR)))
            .child(text("Se recuerda este visor: la próxima vez conecta sin código.", 11., theme::TEXT3))
            .child(footer(vec![
                button("Cancelar", Kind::Secondary, move |_| close(ws)).into_element(),
                button("Emparejar", Kind::Primary, move |_| submit()).into_element(),
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
            let app = w
                .active()
                .map(|v| v.session.info.app.clone())
                .unwrap_or_default();
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            recordings_dir()
                .join(format!("{app}-{stamp}.sismo"))
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
            ("A mano", StopRule::Manual),
            ("5 min", StopRule::After(Duration::from_mins(5))),
            ("500 MB", StopRule::Size(500 * 1024 * 1024)),
            ("Al primer error", StopRule::FirstError),
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
                w.notify("No se pudo grabar", error.to_string(), None, true);
            }
        };
        panel(560.)
            .padding(24.)
            .spacing(12.)
            .child(title_row("Grabar sesión"))
            .child(text("Guarda en esta PC lo que llega de la app, en el formato que abren el visor y el CLI.", 12., theme::TEXT2))
            .child(caption("Desde"))
            .child(option_row(
                "Incluir lo ya recibido",
                &format!("{} desde que te conectaste, con el buffer de la app · sirve para grabar algo que ya pasó", Bytes(buffered)),
                *from.read() == RecordFrom::Buffer,
                move |_| from.set(RecordFrom::Buffer),
            ))
            .child(option_row("Solo desde ahora", "", *from.read() == RecordFrom::Now, move |_| from.set(RecordFrom::Now)))
            .child(caption("Parar"))
            .child(rect().horizontal().spacing(6.).children(stops.iter().enumerate().map(|(i, (label, _))| {
                let on = i == current_stop;
                rect()
                    .padding((5., 12.))
                    .corner_radius(4.)
                    .background(theme::color(if on { theme::BG3 } else { theme::BG0 }))
                    .border(bordered(theme::LINE2))
                    .cursor(CursorIcon::Pointer)
                    .on_press(move |_| stop.set(i))
                    .child(text(*label, 12., if on { theme::TEXT } else { theme::TEXT2 }).font_weight(if on { 600 } else { 400 }))
                    .into_element()
            })))
            .child(text("«Al primer error» sigue 10 s más para capturar lo que vino después.", 11., theme::TEXT3))
            .child(caption("Archivo"))
            .child(field(path, "ruta del archivo", 512.))
            .child(option_row(
                "Grabar también en el dispositivo",
                "La app escribe su propia copia y la manda al terminar; aguanta cortes de Wi-Fi.",
                *on_device.read(),
                move |_| on_device.toggle(),
            ))
            .child(hline())
            .child(footer(vec![
                button("Cancelar", Kind::Secondary, move |_| close(ws)).into_element(),
                button("Empezar a grabar  Ctrl R", Kind::Primary, start).into_element(),
            ]))
    }
}

const FORMATS: [(Format, &str, &str); 5] = [
    (
        Format::Perfetto,
        "Perfetto  .pftrace",
        "Timeline completo en ui.perfetto.dev o SQL con trace_processor",
    ),
    (
        Format::SummaryJson,
        "Resumen JSON  .json",
        "Para agentes y scripts: top funciones, frames lentos, errores",
    ),
    (
        Format::Markdown,
        "Reporte Markdown  .md",
        "Legible para un issue o un PR",
    ),
    (
        Format::Chrome,
        "Chrome trace  .json",
        "chrome://tracing y herramientas viejas",
    ),
    (
        Format::Sismo,
        ".sismo recortado",
        "Solo el rango elegido, para mandarlo a otra persona",
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
            .map_or_else(|| "sismo".into(), |v| v.session.info.app.clone());
        let current = *format.read();
        let ext = FORMATS[current].0.extension();
        let mut path = use_state(move || {
            recordings_dir()
                .join(format!("{base}-export.pftrace"))
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
                    w.notify("Exportado", Bytes(size).to_string(), Some(target), false);
                }
                Err(error) => w.notify("No se pudo exportar", error.to_string(), None, true),
            }
        };
        panel(620.)
            .padding(24.)
            .spacing(10.)
            .child(title_row("Exportar"))
            .child(caption("Formato"))
            .children(FORMATS.iter().enumerate().map(|(i, (_, label, sub))| {
                option_row(label, sub, i == current, move |_| format.set(i))
            }))
            .child(caption("Alcance"))
            .child(option_row(
                "Toda la sesión",
                "",
                *scope.read() == 0,
                move |_| scope.set(0),
            ))
            .maybe(has_selection, |r| {
                r.child(option_row(
                    "Selección del Timeline",
                    "",
                    *scope.read() == 1,
                    move |_| scope.set(1),
                ))
            })
            .child(option_row(
                "Lo que pasa la consulta actual",
                "",
                *scope.read() == 2,
                move |_| scope.set(2),
            ))
            .child(option_row("Incluir logs", "", *logs.read(), move |_| {
                logs.toggle();
            }))
            .child(option_row(
                "Quitar campos de los logs",
                "Útil si pueden tener datos de usuarios",
                *strip.read(),
                move |_| strip.toggle(),
            ))
            .child(caption("Archivo"))
            .child(field(path, "ruta del archivo", 572.))
            .child(hline())
            .child(footer(vec![
                button("Cancelar", Kind::Secondary, move |_| close(ws)).into_element(),
                button("Exportar", Kind::Primary, run).into_element(),
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
                    "(por defecto)".to_owned(),
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
            .child(title_row("Qué manda la app"))
            .child(text(
                "Se aplica en el dispositivo: bajar el nivel baja el costo, no solo lo que ves.",
                12.,
                theme::TEXT2,
            ))
            .child(field(value, "info,mi_app=trace,zudp=warn", 472.))
            .maybe_child(error.map(|e| text(e, 11., theme::ERROR)))
            .child(caption("Niveles por target"))
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
                button("Cancelar", Kind::Secondary, move |_| close(ws)).into_element(),
                button("Aplicar en la app", Kind::Primary, move |_| {
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
            .child(title_row("Marcador"))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(32.))
                    .corner_radius(4.)
                    .background(theme::color(theme::BG0))
                    .border(bordered(theme::TEXT2))
                    .center()
                    .child(Input::new(note).flat().auto_focus(true).width(Size::px(330.)).placeholder("qué estás haciendo").on_submit(move |_| save())),
            )
            .child(text("Queda en la app y en la grabación; aparece en Timeline, Logs y en el resumen del CLI.", 11., theme::TEXT3))
            .child(footer(vec![button("Guardar  Enter", Kind::Primary, move |_| save()).into_element()]))
    }
}

/// Command palette entries: label, shortcut, action id.
const ACTIONS: [(&str, &str, u8); 12] = [
    ("Comparar con…", "Ctrl D", 11),
    ("Abrir archivo…", "Ctrl O", 0),
    ("Grabar / detener", "Ctrl R", 1),
    ("Exportar…", "Ctrl E", 2),
    ("Ir a Timeline", "Ctrl 1", 3),
    ("Ir a Flamegraph", "Ctrl 2", 4),
    ("Ir a Funciones", "Ctrl 3", 5),
    ("Ir a Logs", "Ctrl 4", 6),
    ("Ir a Métricas", "Ctrl 5", 7),
    ("Qué manda la app…", "", 8),
    ("Volver a Sesiones", "", 9),
    ("Atajos de teclado", "?", 10),
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
        let actions: Vec<(&str, &str, u8)> = ACTIONS
            .iter()
            .copied()
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
                            .placeholder("Buscar acción o función…")
                            .on_submit(move |_| {
                                if let Some(id) = first {
                                    run_action(ws, id);
                                }
                            }),
                    ),
            )
            .child(hline())
            .maybe(!actions.is_empty(), |r| {
                r.child(rect().padding((6., 8.)).child(caption("Acciones")))
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
                r.child(rect().padding((6., 8.)).child(caption("Funciones")))
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
            .child(rect().padding((6., 10.)).child(text(
                "Enter ejecuta la primera · Esc cierra",
                11.,
                theme::TEXT3,
            )))
    }
}

fn shortcuts() -> Rect {
    let groups: [(&str, &[(&str, &str)]); 4] = [
        (
            "General",
            &[
                ("Paleta de comandos", "Ctrl K"),
                ("Abrir archivo", "Ctrl O"),
                ("Cerrar sesión", "Ctrl W"),
                ("Estos atajos", "?"),
            ],
        ),
        (
            "Sesión",
            &[
                ("Grabar / detener", "Ctrl R"),
                ("Marcador", "M"),
                ("Pausar / reanudar", "Espacio"),
                ("Exportar", "Ctrl E"),
                ("Comparar con…", "Ctrl D"),
            ],
        ),
        (
            "Timeline",
            &[
                ("Acercar / alejar", "W  S"),
                ("Mover", "A  D"),
                ("Rango", "arrastrar"),
                ("Siguiente frame lento", "N"),
                ("Volver a vivo", "End"),
            ],
        ),
        (
            "Listas",
            &[
                ("Elegir", "clic"),
                ("Pestaña 1–5", "Ctrl 1…5"),
                ("Quitar selección", "Esc"),
            ],
        ),
    ];
    panel(960.)
        .padding(24.)
        .spacing(16.)
        .child(title_row("Atajos"))
        .child(
            rect()
                .horizontal()
                .spacing(24.)
                .children(groups.iter().map(|(title, items)| {
                    rect()
                        .width(Size::px(210.))
                        .spacing(10.)
                        .child(caption(title))
                        .children(items.iter().map(|(label, keys)| {
                            rect()
                                .width(Size::fill())
                                .horizontal()
                                .content(Content::flex())
                                .child(text(*label, 12., theme::TEXT).width(Size::flex(1.)))
                                .child(kbd(keys))
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
        .layer(Layer::Overlay)
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
                toast.title.clone(),
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
                    r.child(button("Abrir", Kind::Secondary, move |_| {
                        if let Some(p) = &path {
                            let mut w = ws.write();
                            w.toast = None;
                            w.open_file(p);
                        }
                    }))
                })
                .maybe_child(folder.map(|dir| {
                    button("Mostrar en la carpeta", Kind::Secondary, move |_| {
                        super::platform::reveal(&dir);
                    })
                }))
                .child(button("Cerrar", Kind::Secondary, move |_| {
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
        let Some(path) = super::platform::pick_recording("Comparar con…".into()).await else {
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
                .notify("No se pudo abrir", error.to_string(), None, true),
        }
    });
}

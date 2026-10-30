//! Desktop viewer built with Freya.

// Canvas code converts freely between nanosecond timestamps and pixel
// coordinates; precision beyond a pixel is irrelevant there.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_arguments
)]

mod dialogs;
mod draw;
mod flame;
mod functions;
mod logs;
mod metrics;
mod query_bar;
mod sessions;
mod shell;
mod theme;
mod timeline;
mod widgets;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freya::prelude::*;

use crate::client::{Discovered, Scanner};
use crate::workspace::{Recent, Saved, SessionView, Tab};

/// Recording to diff the opened file against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareWith(pub PathBuf);

/// Modal surfaces; at most one is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    Pair { code: String },
    Record,
    Export,
    AppFilter,
    Palette { text: String },
    Shortcuts,
    Marker { text: String },
}

/// Transient notice in the bottom-right corner.
#[derive(Debug, Clone)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub path: Option<PathBuf>,
    pub error: bool,
    pub shown: Instant,
}

/// Everything the window shows.
pub struct Workspace {
    pub sessions: Vec<SessionView>,
    pub active: Option<usize>,
    pub scanner: Option<Scanner>,
    pub discovered: Vec<Discovered>,
    pub scan_error: Option<String>,
    pub scan_started: Instant,
    pub recent: Recent,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    next_id: u64,
}

impl Workspace {
    fn new(file: Option<PathBuf>, live: Option<SocketAddr>) -> Self {
        let scanner = Scanner::start(Duration::from_secs(1));
        let scan_error = scanner.as_ref().err().map(ToString::to_string);
        let mut workspace = Self {
            sessions: Vec::new(),
            active: None,
            scanner: scanner.ok(),
            discovered: Vec::new(),
            scan_error,
            scan_started: Instant::now(),
            recent: Recent::load(),
            dialog: None,
            toast: None,
            next_id: 1,
        };
        if let Some(file) = file {
            workspace.open_file(&file);
        }
        if let Some(addr) = live {
            workspace.connect(addr);
        }
        workspace
    }

    pub fn active(&self) -> Option<&SessionView> {
        self.active.and_then(|i| self.sessions.get(i))
    }

    pub fn active_mut(&mut self) -> Option<&mut SessionView> {
        self.active.and_then(|i| self.sessions.get_mut(i))
    }

    pub fn notify(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        path: Option<PathBuf>,
        error: bool,
    ) {
        self.toast = Some(Toast {
            title: title.into(),
            body: body.into(),
            path,
            error,
            shown: Instant::now(),
        });
    }

    pub fn open_file(&mut self, path: &Path) {
        if let Some(index) = self.sessions.iter().position(
            |s| matches!(&s.origin, crate::workspace::Origin::File { path: p, .. } if p == path),
        ) {
            self.active = Some(index);
            return;
        }
        match SessionView::open_file(self.next_id, path) {
            Ok(view) => {
                self.next_id += 1;
                self.recent.push(path);
                self.sessions.push(view);
                self.active = Some(self.sessions.len() - 1);
            }
            Err(error) => self.notify("No se pudo abrir", error.to_string(), None, true),
        }
    }

    pub fn connect(&mut self, addr: SocketAddr) {
        if let Some(index) = self
            .sessions
            .iter()
            .position(|s| s.live().is_some_and(|l| l.addr == addr))
        {
            self.active = Some(index);
            return;
        }
        match SessionView::connect(self.next_id, addr) {
            Ok(view) => {
                self.next_id += 1;
                self.sessions.push(view);
                self.active = Some(self.sessions.len() - 1);
            }
            Err(error) => self.notify("No se pudo conectar", error.to_string(), None, true),
        }
    }

    pub fn close(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.sessions.remove(index);
        }
        self.active = None;
    }

    /// Whether any background source has something new.
    fn pending(&self) -> bool {
        self.scanner.as_ref().is_some_and(|s| !s.events.is_empty())
            || self.sessions.iter().any(|s| {
                s.live()
                    .and_then(|l| l.connection.as_ref())
                    .is_some_and(|c| !c.events.is_empty())
            })
            || self
                .toast
                .as_ref()
                .is_some_and(|t| t.shown.elapsed() > Duration::from_secs(8))
    }

    fn poll(&mut self) {
        if let Some(scanner) = &self.scanner {
            for event in scanner.events.try_iter() {
                match event {
                    Ok(found) => {
                        self.discovered.retain(|d| d.addr != found.addr);
                        self.discovered.push(found);
                    }
                    Err(error) => self.scan_error = Some(error),
                }
            }
        }
        let mut saved: Vec<Saved> = Vec::new();
        let active_id = self.active.map(|i| self.sessions_id(i));
        let mut wants_pairing = false;
        for session in &mut self.sessions {
            if let Some(done) = session.poll_live() {
                saved.push(done);
            }
            let pairing = matches!(
                session.live().map(|l| &l.status),
                Some(crate::workspace::LiveStatus::NeedsPairing { .. })
            );
            wants_pairing |= pairing && active_id == Some(session.id);
        }
        if wants_pairing && self.dialog.is_none() {
            self.dialog = Some(Dialog::Pair {
                code: String::new(),
            });
        }
        for done in saved {
            self.recording_saved(&done);
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.shown.elapsed() > Duration::from_secs(8))
        {
            self.toast = None;
        }
    }

    fn sessions_id(&self, index: usize) -> u64 {
        self.sessions.get(index).map_or(0, |s| s.id)
    }

    pub fn recording_saved(&mut self, saved: &Saved) {
        use sismo_analysis::units::{Bytes, Nanos};
        let body = format!(
            "{} · {} · {} marcadores",
            Nanos(u64::try_from(saved.duration.as_nanos()).unwrap_or(u64::MAX)),
            Bytes(saved.bytes),
            saved.markers
        );
        self.recent.push(&saved.path);
        self.notify("Grabación guardada", body, Some(saved.path.clone()), false);
    }
}

fn handle_shortcut(mut ws: State<Workspace>, event: &KeyboardEventData) {
    let ctrl = event.modifiers.ctrl() || event.modifiers.meta();
    let dialog_open = ws.peek().dialog.is_some();
    match &event.key {
        Key::Named(NamedKey::Escape) => {
            let mut w = ws.write();
            if w.dialog.take().is_none()
                && let Some(view) = w.active_mut()
            {
                view.selection = None;
                view.selected_span = None;
                view.flame_focus.clear();
                view.revision += 1;
            }
        }
        Key::Character(c) if ctrl => match c.to_lowercase().as_str() {
            "k" => {
                ws.write().dialog = Some(Dialog::Palette {
                    text: String::new(),
                });
            }
            "o" => dialogs::pick_file(ws),
            "d" => dialogs::pick_compare(ws),
            "r" => dialogs::toggle_record(ws),
            "e" if ws.peek().active.is_some() => {
                ws.write().dialog = Some(Dialog::Export);
            }
            "w" => {
                let active = ws.peek().active;
                if let Some(index) = active {
                    ws.write().close(index);
                }
            }
            digit @ ("1" | "2" | "3" | "4" | "5") => {
                let index = digit.parse::<usize>().unwrap_or(1) - 1;
                if let Some(view) = ws.write().active_mut() {
                    view.tab = Tab::ALL[index];
                }
            }
            _ => {}
        },
        Key::Character(c) if !dialog_open && !ctrl => match c.as_str() {
            "?" => ws.write().dialog = Some(Dialog::Shortcuts),
            "m" | "M" if ws.peek().active().is_some_and(|v| v.live().is_some()) => {
                ws.write().dialog = Some(Dialog::Marker {
                    text: String::new(),
                });
            }
            " " => {
                if let Some(view) = ws.write().active_mut() {
                    view.paused = !view.paused;
                    view.follow = !view.paused;
                }
            }
            "w" | "s" | "a" | "d" => {
                if let Some(view) = ws.write().active_mut() {
                    timeline::keyboard_nav(view, c.as_str());
                }
            }
            "n" => {
                if let Some(view) = ws.write().active_mut() {
                    timeline::next_slow_frame(view);
                }
            }
            _ => {}
        },
        Key::Named(NamedKey::End) if !dialog_open => {
            if let Some(view) = ws.write().active_mut() {
                view.follow = true;
                view.paused = false;
            }
        }
        _ => {}
    }
}

fn app() -> impl IntoElement {
    let file = use_try_consume::<Option<PathBuf>>().flatten();
    let live = use_try_consume::<Option<SocketAddr>>().flatten();
    let tab = use_try_consume::<Tab>();
    let compare = use_try_consume::<Option<CompareWith>>().flatten();
    let mut ws = use_state(move || {
        let mut workspace = Workspace::new(file, live);
        if let (Some(CompareWith(path)), Some(view)) = (compare, workspace.active_mut()) {
            match sismo_analysis::Session::load(&path) {
                Ok((other, _)) => {
                    view.compare = Some((path, Box::new(other)));
                    view.tab = Tab::Functions;
                }
                Err(error) => {
                    workspace.notify("No se pudo comparar", error.to_string(), None, true);
                }
            }
        }
        if let (Some(tab), Some(view)) = (tab, workspace.active_mut()) {
            view.tab = tab;
        }
        workspace
    });
    use_hook(move || {
        spawn(async move {
            loop {
                timer(Duration::from_millis(200)).await;
                if ws.peek().pending() {
                    ws.write().poll();
                }
            }
        });
    });

    let w = ws.read();
    let screen = match w.active {
        Some(index) => shell::session_screen(ws, index).into_element(),
        None => sessions::sessions_screen(ws).into_element(),
    };
    let dialog = w.dialog.as_ref().map(|d| dialogs::render(ws, d));
    let toast = w.toast.as_ref().map(|t| dialogs::toast(ws, t));
    drop(w);

    rect()
        .expanded()
        .background(theme::color(theme::BG0))
        .color(theme::color(theme::TEXT))
        .font_family(theme::SANS)
        .font_size(13.)
        .on_global_key_down(move |e: Event<KeyboardEventData>| handle_shortcut(ws, &e))
        .on_file_drop(move |e: Event<FileEventData>| {
            for path in &e.file_paths {
                ws.write().open_file(path);
            }
        })
        .child(screen)
        .maybe_child(dialog)
        .maybe_child(toast)
}

/// Opens the viewer window; blocks until it closes.
pub fn run(file: Option<PathBuf>, live: Option<SocketAddr>, compare: Option<CompareWith>) {
    use theme::{
        MONO, MONO_MEDIUM, MONO_REGULAR, MONO_SEMIBOLD, SANS, SANS_REGULAR, SANS_SEMIBOLD,
    };
    launch(
        LaunchConfig::new()
            .with_font(SANS, Bytes::from_static(SANS_REGULAR))
            .with_font(SANS, Bytes::from_static(SANS_SEMIBOLD))
            .with_font(MONO, Bytes::from_static(MONO_REGULAR))
            .with_font(MONO, Bytes::from_static(MONO_MEDIUM))
            .with_font(MONO, Bytes::from_static(MONO_SEMIBOLD))
            .with_default_font(SANS)
            .with_window(
                WindowConfig::new(app)
                    .with_size(1440., 900.)
                    .with_min_size(960., 600.)
                    .with_title("sismo")
                    .with_app_id("dev.sismo.viewer")
                    .with_background((0x16, 0x16, 0x15))
                    .with_root_context(file)
                    .with_root_context(live)
                    .with_root_context(compare),
            ),
    );
}

#[cfg(test)]
mod snapshots {
    use std::path::PathBuf;

    use freya::prelude::*;
    use freya_testing::prelude::*;

    use super::theme;
    use crate::workspace::Tab;

    /// Renders every tab of `SISMO_SNAPSHOT_FILE` into `SISMO_SNAPSHOT_DIR`.
    #[test]
    #[ignore = "writes PNGs; run explicitly"]
    fn render_tabs() {
        let file = std::env::var_os("SISMO_SNAPSHOT_FILE").map(PathBuf::from);
        let dir =
            std::env::var_os("SISMO_SNAPSHOT_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let tabs: Vec<Option<Tab>> = if file.is_some() {
            Tab::ALL.iter().copied().map(Some).collect()
        } else {
            vec![None]
        };
        for tab in tabs {
            let file = file.clone();
            let compare = std::env::var_os("SISMO_SNAPSHOT_COMPARE")
                .map(|p| super::CompareWith(PathBuf::from(p)));
            let (mut runner, ()) = TestingRunner::new(
                super::app,
                (1440., 900.).into(),
                move |r| {
                    r.provide_root_context(move || file.clone());
                    r.provide_root_context(move || compare.clone());
                    if let Some(tab) = tab {
                        r.provide_root_context(move || tab);
                    }
                },
                1.,
            );
            for (name, bytes) in [
                (theme::SANS, theme::SANS_REGULAR),
                (theme::SANS, theme::SANS_SEMIBOLD),
                (theme::MONO, theme::MONO_REGULAR),
                (theme::MONO, theme::MONO_SEMIBOLD),
            ] {
                Platform::get().load_font(name, Bytes::from_static(bytes));
            }
            runner.sync_and_update();
            runner.sync_and_update();
            let name = tab.map_or("sessions", Tab::label).to_lowercase();
            runner.render_to_file(dir.join(format!("sismo-{name}.png")));
        }
    }
}

#[cfg(test)]
mod live_snapshot {
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::time::Duration;

    use freya::prelude::*;
    use freya_testing::prelude::*;

    use super::theme;
    use crate::workspace::Tab;

    /// Connects to `SISMO_SNAPSHOT_LIVE` (ip:port), streams for a few
    /// seconds and renders the Timeline and Logs tabs.
    #[test]
    #[ignore = "needs a live app; run explicitly"]
    fn render_live() {
        let Some(addr) = std::env::var("SISMO_SNAPSHOT_LIVE")
            .ok()
            .and_then(|a| a.parse::<SocketAddr>().ok())
        else {
            return;
        };
        let dir =
            std::env::var_os("SISMO_SNAPSHOT_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        for tab in [Tab::Timeline, Tab::Logs] {
            let (mut runner, ()) = TestingRunner::new(
                super::app,
                (1440., 900.).into(),
                move |r| {
                    r.provide_root_context(move || Some(addr));
                    r.provide_root_context(|| None::<PathBuf>);
                    r.provide_root_context(move || tab);
                },
                1.,
            );
            for (name, bytes) in [
                (theme::SANS, theme::SANS_REGULAR),
                (theme::SANS, theme::SANS_SEMIBOLD),
                (theme::MONO, theme::MONO_REGULAR),
            ] {
                Platform::get().load_font(name, Bytes::from_static(bytes));
            }
            runner.poll(Duration::from_millis(100), Duration::from_secs(3));
            runner
                .render_to_file(dir.join(format!("sismo-live-{}.png", tab.label().to_lowercase())));
        }
    }
}

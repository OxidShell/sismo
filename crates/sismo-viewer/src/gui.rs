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

mod context_menu;
mod dialogs;
mod draw;
mod flame;
mod functions;
mod groups;
mod i18n;
mod logs;
mod metrics;
mod platform;
mod prefs;
mod query_bar;
mod sessions;
mod settings;
mod shell;
mod theme;
mod timeline;
mod widgets;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freya::prelude::*;

use crate::client::{Discovered, ScanEvent, Scanner};
use crate::workspace::{Saved, SessionView, Tab};
use i18n::tr;
use prefs::Prefs;

/// Screen to show first (used by snapshot tests): a settings section
/// name, `groups` or `menu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartScreen(pub &'static str);

/// Recording to diff the opened file against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareWith(pub PathBuf);

/// Modal surfaces; at most one is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    Pair {
        code: String,
    },
    Record,
    Export,
    AppFilter,
    Palette {
        text: String,
    },
    Shortcuts,
    Marker {
        text: String,
    },
    Groups,
    ClearSession,
    /// The live app restarted: keep or drop what came from its last run.
    AppRestarted,
}

/// Transient notice in the bottom-right corner.
#[derive(Debug, Clone)]
pub struct Toast {
    /// Fluent key, translated when rendered.
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
    pub prefs: Prefs,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    /// Open settings section, if the settings screen is showing.
    pub settings: Option<settings::Section>,
    pub menu: Option<context_menu::MenuTarget>,
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
            prefs: Prefs::load(),
            dialog: None,
            toast: None,
            settings: None,
            menu: None,
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
            Ok(mut view) => {
                self.next_id += 1;
                self.prefs.push_recent(path);
                view.session
                    .set_groups(self.prefs.groups_for(&view.session.info.app));
                view.groups_applied = true;
                self.sessions.push(view);
                self.active = Some(self.sessions.len() - 1);
            }
            Err(error) => self.notify("toast-open-failed", error.to_string(), None, true),
        }
    }

    pub fn connect(&mut self, addr: SocketAddr) {
        // An open tab for this app: reuse it, reconnecting it if it was lost
        // (the app may have restarted at the same address).
        if let Some(index) = self
            .sessions
            .iter()
            .position(|s| s.live().is_some_and(|l| l.addr == addr))
        {
            self.active = Some(index);
            let lost = matches!(
                self.sessions[index].live().map(|l| &l.status),
                Some(crate::workspace::LiveStatus::Lost { .. })
            );
            if lost && let Err(error) = self.sessions[index].reconnect_to(addr) {
                self.notify("toast-connect-failed", error.to_string(), None, true);
            }
            return;
        }
        match SessionView::connect(self.next_id, addr) {
            Ok(view) => {
                self.next_id += 1;
                self.sessions.push(view);
                self.active = Some(self.sessions.len() - 1);
            }
            Err(error) => self.notify("toast-connect-failed", error.to_string(), None, true),
        }
    }

    /// Where the app a lost tab followed announces itself now: the same
    /// identity (key fingerprint, or app and device) may come back on
    /// another port or address.
    fn find_again(&self, view: &SessionView) -> Option<SocketAddr> {
        let info = &view.session.info;
        let fingerprint = view.live().and_then(|l| {
            self.discovered
                .iter()
                .find(|d| d.addr == l.addr)
                .map(|d| d.beacon.fingerprint.clone())
        });
        self.discovered
            .iter()
            .filter(|d| d.beacon.app == info.app && d.beacon.device == info.device)
            .max_by_key(|d| {
                (
                    fingerprint.as_deref() == Some(d.beacon.fingerprint.as_str()),
                    d.seen,
                )
            })
            .map(|d| d.addr)
    }

    /// Reconnects lost live tabs whose backoff ran out.
    fn retry_lost(&mut self) {
        let due: Vec<(usize, Option<SocketAddr>)> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| s.live().is_some_and(|l| l.retry.due()))
            .map(|(i, s)| (i, self.find_again(s)))
            .collect();
        for (index, found) in due {
            let session = &mut self.sessions[index];
            let result = match found {
                Some(addr) => session.reconnect_to(addr),
                None => session.reconnect(),
            };
            if result.is_err()
                && let Some(live) = session.live_mut()
            {
                live.retry.schedule();
            }
        }
    }

    fn start_on(&mut self, screen: &str) {
        use settings::Section;
        let section = match screen {
            "appearance" => Some(Section::Appearance),
            "recording" => Some(Section::Recording),
            "network" => Some(Section::Network),
            "devices" => Some(Section::Devices),
            "groups-settings" => Some(Section::Groups),
            "shortcuts" => Some(Section::Shortcuts),
            "agents" => Some(Section::Agents),
            _ => None,
        };
        if section.is_some() {
            self.settings = section;
        } else if screen == "groups" {
            self.dialog = Some(Dialog::Groups);
        } else if screen == "menu"
            && let Some(callsite) = self
                .active()
                .and_then(|v| v.session.threads.first()?.spans.first().map(|s| s.callsite))
        {
            self.menu = Some(context_menu::MenuTarget {
                callsite,
                thread: None,
                span: None,
                x: 600.,
                y: 300.,
            });
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
                .sessions
                .iter()
                .any(|s| s.live().is_some_and(|l| l.retry.due()))
            || self
                .toast
                .as_ref()
                .is_some_and(|t| t.shown.elapsed() > Duration::from_secs(8))
    }

    fn poll(&mut self) {
        if let Some(scanner) = &self.scanner {
            for event in scanner.events.try_iter() {
                match event {
                    Ok(ScanEvent::Found(found)) => {
                        self.discovered.retain(|d| d.addr != found.addr);
                        self.discovered.push(found);
                    }
                    Ok(ScanEvent::Lost(addr)) => self.discovered.retain(|d| d.addr != addr),
                    Err(error) => self.scan_error = Some(error),
                }
            }
        }
        self.retry_lost();
        let mut saved: Vec<Saved> = Vec::new();
        let mut crashes: Vec<std::path::PathBuf> = Vec::new();
        let active_id = self.active.map(|i| self.sessions_id(i));
        let mut wants_pairing = false;
        let mut paired: Vec<(String, String, std::net::SocketAddr)> = Vec::new();
        let live_window = self.prefs.live_window_ns();
        for session in &mut self.sessions {
            if session.live_window_ns != live_window {
                session.live_window_ns = live_window;
                session.view = (session.view.1.saturating_sub(live_window), session.view.1);
            }
            if let Some(done) = session.poll_live() {
                saved.push(done);
            }
            if let Some(path) = session.live_mut().and_then(|l| l.crash_saved.take()) {
                crashes.push(path);
            }
            if !session.groups_applied && !session.session.info.app.is_empty() {
                session
                    .session
                    .set_groups(self.prefs.groups_for(&session.session.info.app));
                session.groups_applied = true;
                session.revision += 1;
            }
            if std::mem::take(&mut session.paired_now)
                && let Some(live) = session.live()
            {
                paired.push((
                    session.session.info.app.clone(),
                    session.session.info.device.clone(),
                    live.addr,
                ));
            }
            let pairing = matches!(
                session.live().map(|l| &l.status),
                Some(crate::workspace::LiveStatus::NeedsPairing { .. })
            );
            wants_pairing |= pairing && active_id == Some(session.id);
        }
        if self.dialog.is_none() && self.active().is_some_and(|v| v.next_run().is_some()) {
            self.dialog = Some(Dialog::AppRestarted);
        }
        if wants_pairing && self.dialog.is_none() {
            self.dialog = Some(Dialog::Pair {
                code: String::new(),
            });
        }
        for done in saved {
            self.recording_saved(&done);
        }
        for path in crashes {
            self.open_crash_report(&path);
        }
        for (app, device, addr) in paired {
            let fingerprint = self
                .discovered
                .iter()
                .find(|d| d.addr == addr)
                .map_or_else(|| addr.to_string(), |d| d.beacon.fingerprint.clone());
            self.prefs.remember_pairing(&app, &device, &fingerprint);
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.shown.elapsed() > Duration::from_secs(8))
        {
            self.toast = None;
        }
    }

    /// Opens a downloaded crash report on its panic, backtrace in view.
    fn open_crash_report(&mut self, path: &Path) {
        self.open_file(path);
        if let Some(view) = self.active_mut()
            && let Some(index) = view
                .session
                .logs
                .iter()
                .rposition(|l| l.level == sismo_format::Level::Panic)
        {
            view.tab = crate::workspace::Tab::Logs;
            view.selected_log = Some(index);
        }
        self.notify(
            "toast-crash-opened",
            path.display().to_string(),
            Some(path.to_path_buf()),
            false,
        );
    }

    fn sessions_id(&self, index: usize) -> u64 {
        self.sessions.get(index).map_or(0, |s| s.id)
    }

    pub fn recording_saved(&mut self, saved: &Saved) {
        use sismo_analysis::units::{Bytes, Nanos};
        let body = tr!(
            "toast-recording-saved-body",
            duration =
                Nanos(u64::try_from(saved.duration.as_nanos()).unwrap_or(u64::MAX)).to_string(),
            size = Bytes(saved.bytes).to_string(),
            markers = saved.markers.to_string(),
        );
        self.prefs.push_recent(&saved.path);
        self.notify(
            "toast-recording-saved",
            body,
            Some(saved.path.clone()),
            false,
        );
    }
}

fn handle_shortcut(mut ws: State<Workspace>, event: &KeyboardEventData) {
    let ctrl = event.modifiers.ctrl() || event.modifiers.meta();
    let dialog_open = ws.peek().dialog.is_some();
    match &event.key {
        Key::Named(NamedKey::Escape) => {
            let mut w = ws.write();
            if w.menu.take().is_some() {
                return;
            }
            if w.dialog.is_none() && w.settings.take().is_some() {
                return;
            }
            if w.dialog.take().is_none()
                && let Some(view) = w.active_mut()
            {
                if view.tab == Tab::Flamegraph && !view.flame_focus.is_empty() {
                    view.flame_focus.pop();
                } else {
                    view.selection = None;
                    view.selected_span = None;
                    view.flame_focus.clear();
                }
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
            "," => ws.write().settings = Some(settings::Section::default()),
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
                    let paused = !view.paused;
                    view.set_paused(paused);
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
                view.reset_view();
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
    let start = use_try_consume::<StartScreen>();
    let language = use_try_consume::<prefs::Language>();
    let mut ws = use_state(move || {
        let mut workspace = Workspace::new(file, live);
        if let Some(language) = language {
            workspace.prefs.language = Some(language);
        }
        if let (Some(CompareWith(path)), Some(view)) = (compare, workspace.active_mut()) {
            match sismo_analysis::Session::load(&path) {
                Ok((other, _)) => {
                    view.compare = Some((path, Box::new(other)));
                    view.tab = Tab::Functions;
                }
                Err(error) => {
                    workspace.notify("toast-compare-failed", error.to_string(), None, true);
                }
            }
        }
        if let (Some(tab), Some(view)) = (tab, workspace.active_mut()) {
            view.tab = tab;
        }
        if let Some(StartScreen(screen)) = start {
            workspace.start_on(screen);
        }
        workspace
    });
    freya::i18n::use_init_i18n(move || i18n::config(ws.peek().prefs.language()));
    use_init_theme(dark_theme);
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
    let settings = w.settings.map(|_| settings::modal(ws));
    let menu = w.menu.map(|m| context_menu::render(ws, m));
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
        .maybe_child(settings)
        .maybe_child(menu)
        .maybe_child(dialog)
        .maybe_child(toast)
}

/// Opens the viewer window; blocks until it closes.
pub fn run(file: Option<PathBuf>, live: Option<SocketAddr>, compare: Option<CompareWith>) {
    use theme::{
        MONO, MONO_MEDIUM, MONO_REGULAR, MONO_SEMIBOLD, SANS, SANS_REGULAR, SANS_SEMIBOLD,
    };
    platform::install();
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
            let name = tab.map_or("sessions", |t| t.label().trim_start_matches("tab-"));
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
    /// Follows `SISMO_SNAPSHOT_LIVE` for `SISMO_SNAPSHOT_WAIT` seconds (restart
    /// the app meanwhile) and renders what the viewer shows at the end.
    #[test]
    #[ignore = "needs a live app; run explicitly"]
    fn render_live_wait() {
        let Some(addr) = std::env::var("SISMO_SNAPSHOT_LIVE")
            .ok()
            .and_then(|a| a.parse::<SocketAddr>().ok())
        else {
            return;
        };
        let wait = std::env::var("SISMO_SNAPSHOT_WAIT")
            .ok()
            .and_then(|w| w.parse().ok())
            .unwrap_or(20);
        let dir =
            std::env::var_os("SISMO_SNAPSHOT_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let (mut runner, ()) = TestingRunner::new(
            super::app,
            (1440., 900.).into(),
            move |r| {
                r.provide_root_context(move || Some(addr));
                r.provide_root_context(|| None::<PathBuf>);
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
        runner.poll(Duration::from_millis(100), Duration::from_secs(wait));
        runner.render_to_file(dir.join("sismo-live-wait.png"));
        if let Ok(at) = std::env::var("SISMO_SNAPSHOT_CLICK")
            && let Some((x, y)) = at.split_once(',')
            && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
        {
            runner.click_cursor((x, y));
            runner.poll(Duration::from_millis(100), Duration::from_secs(3));
            runner.render_to_file(dir.join("sismo-live-after-click.png"));
        }
    }

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
                if tab == Tab::Logs {
                    (1440., 420.)
                } else {
                    (1440., 900.)
                }
                .into(),
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
            runner.render_to_file(dir.join(format!(
                "sismo-live-{}.png",
                tab.label().trim_start_matches("tab-")
            )));
            if tab == Tab::Timeline {
                // Paused: nothing on screen moves while data keeps arriving.
                runner.press_key(Key::Character(" ".into()));
                runner.poll(Duration::from_millis(100), Duration::from_millis(500));
                runner.render_to_file(dir.join("sismo-live-paused-a.png"));
                runner.poll(Duration::from_millis(100), Duration::from_secs(2));
                runner.render_to_file(dir.join("sismo-live-paused-b.png"));
                runner.press_key(Key::Character(" ".into()));
                runner.poll(Duration::from_millis(100), Duration::from_secs(1));
                runner.render_to_file(dir.join("sismo-live-resumed.png"));
            }
            if tab == Tab::Logs {
                // Scrolling up by hand stops following the newest line.
                runner.scroll((700., 300.), (0., 400.));
                runner.poll(Duration::from_millis(100), Duration::from_secs(2));
                runner.render_to_file(dir.join("sismo-live-logs-scrolled.png"));
            }
        }
    }
}

#[cfg(test)]
mod screen_snapshots {
    use std::path::PathBuf;

    use freya::prelude::*;
    use freya_testing::prelude::*;

    use super::{StartScreen, prefs::Language, theme};

    /// Renders settings sections, the groups editor and the context menu
    /// for `SISMO_SNAPSHOT_FILE` in Spanish and English.
    #[test]
    #[ignore = "writes PNGs; run explicitly"]
    fn render_screens() {
        let file = std::env::var_os("SISMO_SNAPSHOT_FILE").map(PathBuf::from);
        let dir =
            std::env::var_os("SISMO_SNAPSHOT_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let screens = [
            "appearance",
            "recording",
            "devices",
            "agents",
            "groups",
            "menu",
        ];
        for language in [Language::Spanish, Language::English] {
            for screen in screens {
                let file = file.clone();
                let (mut runner, ()) = TestingRunner::new(
                    super::app,
                    (1440., 900.).into(),
                    move |r| {
                        r.provide_root_context(move || file.clone());
                        r.provide_root_context(move || StartScreen(screen));
                        r.provide_root_context(move || language);
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
                runner.sync_and_update();
                runner.sync_and_update();
                runner.render_to_file(dir.join(format!("screen-{}-{screen}.png", language.tag())));
            }
        }
    }
}

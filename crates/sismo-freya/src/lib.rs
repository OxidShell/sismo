//! Freya plugin feeding sismo with frame marks, render phases and tree
//! metrics, so recordings split time between the app and Freya itself.
//!
//! ```ignore
//! let config = LaunchConfig::new().with_plugin(sismo_freya::SismoPlugin::default());
//! ```

use freya_winit::plugins::{FreyaPlugin, PluginEvent, PluginHandle};
use sismo::format::CallsiteKind;
use sismo::{Callsite, ScopeGuard};

macro_rules! phase {
    ($name:literal) => {{
        static CALLSITE: Callsite =
            Callsite::new(CallsiteKind::Span, $name, "freya::winit", file!(), line!());
        &CALLSITE
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Frame,
    Layout,
    Render,
    Present,
    Accessibility,
    Tree,
    Tasks,
    Events,
}

impl Phase {
    fn callsite(self) -> &'static Callsite {
        match self {
            Phase::Frame => phase!("freya::frame"),
            Phase::Layout => phase!("freya::layout"),
            Phase::Render => phase!("freya::render"),
            Phase::Present => phase!("freya::present"),
            Phase::Accessibility => phase!("freya::accessibility"),
            Phase::Tree => phase!("freya::tree_update"),
            Phase::Tasks => phase!("freya::tasks"),
            Phase::Events => phase!("freya::events"),
        }
    }
}

/// Records one span per Freya phase and a frame mark after every redraw.
///
/// Spans close innermost-first, so open phases live on a stack: closing a
/// phase also closes whatever was opened after it, and a phase that starts
/// again without having finished (Freya skipped its end, e.g. while the
/// app was suspended) is closed before it reopens. Dropping guards in any
/// other order would end the wrong span and leave another open forever.
#[derive(Debug, Default)]
pub struct SismoPlugin {
    open: Vec<(Phase, ScopeGuard)>,
}

impl SismoPlugin {
    fn start(&mut self, phase: Phase) {
        self.finish(phase);
        self.open.push((phase, ScopeGuard::enter(phase.callsite())));
    }

    fn finish(&mut self, phase: Phase) {
        if let Some(index) = self.open.iter().position(|(p, _)| *p == phase) {
            while self.open.len() > index {
                drop(self.open.pop());
            }
        }
    }

    fn open_frame(&mut self) {
        if !self.open.iter().any(|(p, _)| *p == Phase::Frame) {
            self.start(Phase::Frame);
        }
    }
}

impl FreyaPlugin for SismoPlugin {
    fn plugin_id(&self) -> &'static str {
        "sismo"
    }

    fn on_event(&mut self, event: &mut PluginEvent, _handle: PluginHandle) {
        match event {
            PluginEvent::StartedMeasuringLayout { .. } => {
                self.open_frame();
                self.start(Phase::Layout);
            }
            PluginEvent::FinishedMeasuringLayout { .. } => self.finish(Phase::Layout),
            PluginEvent::BeforeRender { .. } => {
                self.open_frame();
                self.start(Phase::Render);
            }
            PluginEvent::AfterRender { metrics, .. } => {
                self.finish(Phase::Render);
                sismo::counter!("freya.tree_nodes", metrics.tree_nodes);
                sismo::counter!("freya.layout_nodes", metrics.layout_nodes);
                sismo::counter!("freya.text_cache", metrics.text_cache_size);
                sismo::counter!("freya.alive_tasks", metrics.alive_tasks);
                sismo::counter!("freya.scopes", metrics.scopes);
            }
            PluginEvent::BeforePresenting { .. } => self.start(Phase::Present),
            PluginEvent::AfterPresenting { .. } => self.finish(Phase::Present),
            PluginEvent::BeforeAccessibility { .. } => self.start(Phase::Accessibility),
            PluginEvent::AfterAccessibility { .. } => self.finish(Phase::Accessibility),
            PluginEvent::AfterRedraw { .. } => {
                self.finish(Phase::Frame);
                sismo::frame_mark();
            }
            PluginEvent::StartedUpdatingTree { .. } => self.start(Phase::Tree),
            PluginEvent::FinishedUpdatingTree { .. } => self.finish(Phase::Tree),
            PluginEvent::StartedPollingTasks { .. } => self.start(Phase::Tasks),
            PluginEvent::FinishedPollingTasks { .. } => self.finish(Phase::Tasks),
            PluginEvent::StartedMeasuringEvents { .. } => self.start(Phase::Events),
            PluginEvent::FinishedMeasuringEvents { .. } => self.finish(Phase::Events),
            _ => {}
        }
    }
}

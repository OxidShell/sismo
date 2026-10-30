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

/// Records one span per Freya phase and a frame mark after every redraw.
#[derive(Debug, Default)]
pub struct SismoPlugin {
    frame: Option<ScopeGuard>,
    layout: Option<ScopeGuard>,
    render: Option<ScopeGuard>,
    present: Option<ScopeGuard>,
    accessibility: Option<ScopeGuard>,
    tree: Option<ScopeGuard>,
    tasks: Option<ScopeGuard>,
    events: Option<ScopeGuard>,
}

impl SismoPlugin {
    fn open_frame(&mut self) {
        if self.frame.is_none() {
            self.frame = Some(ScopeGuard::enter(phase!("freya::frame")));
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
                self.layout = Some(ScopeGuard::enter(phase!("freya::layout")));
            }
            PluginEvent::FinishedMeasuringLayout { .. } => self.layout = None,
            PluginEvent::BeforeRender { .. } => {
                self.open_frame();
                self.render = Some(ScopeGuard::enter(phase!("freya::render")));
            }
            PluginEvent::AfterRender { metrics, .. } => {
                self.render = None;
                sismo::counter!("freya.tree_nodes", metrics.tree_nodes);
                sismo::counter!("freya.layout_nodes", metrics.layout_nodes);
                sismo::counter!("freya.text_cache", metrics.text_cache_size);
                sismo::counter!("freya.alive_tasks", metrics.alive_tasks);
                sismo::counter!("freya.scopes", metrics.scopes);
            }
            PluginEvent::BeforePresenting { .. } => {
                self.present = Some(ScopeGuard::enter(phase!("freya::present")));
            }
            PluginEvent::AfterPresenting { .. } => self.present = None,
            PluginEvent::BeforeAccessibility { .. } => {
                self.accessibility = Some(ScopeGuard::enter(phase!("freya::accessibility")));
            }
            PluginEvent::AfterAccessibility { .. } => self.accessibility = None,
            PluginEvent::AfterRedraw { .. } => {
                self.accessibility = None;
                self.present = None;
                self.frame = None;
                sismo::frame_mark();
            }
            PluginEvent::StartedUpdatingTree { .. } => {
                self.tree = Some(ScopeGuard::enter(phase!("freya::tree_update")));
            }
            PluginEvent::FinishedUpdatingTree { .. } => self.tree = None,
            PluginEvent::StartedPollingTasks { .. } => {
                self.tasks = Some(ScopeGuard::enter(phase!("freya::tasks")));
            }
            PluginEvent::FinishedPollingTasks { .. } => self.tasks = None,
            PluginEvent::StartedMeasuringEvents { .. } => {
                self.events = Some(ScopeGuard::enter(phase!("freya::events")));
            }
            PluginEvent::FinishedMeasuringEvents { .. } => self.events = None,
            _ => {}
        }
    }
}

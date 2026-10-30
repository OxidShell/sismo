use std::fmt;

use serde::{Deserialize, Serialize};

/// Coarse ownership of a crate, used to split time into "your code" and
/// "the framework".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    App,
    Framework,
    Other,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::App, Group::Framework, Group::Other];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Group::App => "app",
            Group::Framework => "framework",
            Group::Other => "other",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "app" => Some(Group::App),
            "framework" | "fw" => Some(Group::Framework),
            "other" | "otros" | "otro" => Some(Group::Other),
            _ => None,
        }
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One classification rule: a crate name or a `prefix*` pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub pattern: String,
    pub group: Group,
}

impl Rule {
    fn matches(&self, krate: &str) -> bool {
        match self.pattern.strip_suffix('*') {
            Some(prefix) => krate.starts_with(prefix),
            None => krate == self.pattern,
        }
    }
}

const FRAMEWORK: &[&str] = &[
    "freya*",
    "torin",
    "ragnarok",
    "skia*",
    "winit",
    "wgpu*",
    "glutin*",
    "egui*",
    "eframe",
    "epaint",
    "emath",
    "ecolor",
    "gpui*",
    "iced*",
    "slint*",
    "i_slint*",
    "dioxus*",
    "tauri*",
    "bevy*",
    "accesskit*",
    "cosmic*",
    "vello*",
    "femtovg",
    "taffy",
    "parley",
    "cushy",
    "xilem*",
    "masonry",
    "floem*",
    "makepad*",
    "druid*",
    "kurbo",
    "peniko",
    "glow",
    "glyphon",
    "cosmic_text",
];

const OTHER: &[&str] = &[
    "std",
    "core",
    "alloc",
    "tokio*",
    "futures*",
    "async_*",
    "smol*",
    "rayon*",
    "crossbeam*",
    "log",
    "tracing*",
    "zudp",
    "mio",
    "hyper*",
    "reqwest",
    "h2",
    "rustls*",
    "ring",
    "serde*",
    "bincode",
    "rkyv",
    "flume",
    "parking_lot*",
    "dashmap",
    "sismo*",
    "image",
    "png",
    "zune*",
    "lyon*",
    "sqlx*",
    "rusqlite",
    "diesel*",
    "ndk*",
    "jni",
    "objc2*",
    "android*",
    "?",
];

/// Ordered classification rules; the first match wins and unknown crates
/// count as the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Groups {
    pub rules: Vec<Rule>,
}

impl Default for Groups {
    fn default() -> Self {
        let rules = FRAMEWORK
            .iter()
            .map(|p| (p, Group::Framework))
            .chain(OTHER.iter().map(|p| (p, Group::Other)))
            .map(|(pattern, group)| Rule {
                pattern: (*pattern).to_owned(),
                group,
            })
            .collect();
        Self { rules }
    }
}

impl Groups {
    /// Puts the recorded app's own crate first as [`Group::App`].
    pub fn set_app(&mut self, app: &str) {
        let pattern = app.replace('-', "_");
        if self.rules.first().is_some_and(|r| r.pattern == pattern) {
            return;
        }
        self.rules.insert(
            0,
            Rule {
                pattern,
                group: Group::App,
            },
        );
    }

    #[must_use]
    pub fn classify(&self, krate: &str) -> Group {
        let krate = krate.replace('-', "_");
        self.rules
            .iter()
            .find(|r| r.matches(&krate))
            .map_or(Group::App, |r| r.group)
    }

    /// Moves `krate` to `group` ahead of every pattern rule.
    pub fn assign(&mut self, krate: &str, group: Group) {
        let pattern = krate.replace('-', "_");
        self.rules.retain(|r| r.pattern != pattern);
        self.rules.insert(0, Rule { pattern, group });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_crates() {
        let mut groups = Groups::default();
        groups.set_app("notinplus");
        assert_eq!(groups.classify("freya_core"), Group::Framework);
        assert_eq!(groups.classify("tokio"), Group::Other);
        assert_eq!(groups.classify("notinplus"), Group::App);
        assert_eq!(groups.classify("notein-sync"), Group::App);
        groups.assign("lyon_tessellation", Group::App);
        assert_eq!(groups.classify("lyon_tessellation"), Group::App);
    }
}

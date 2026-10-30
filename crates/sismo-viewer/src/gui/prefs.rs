//! Viewer preferences, stored as one bincode file in the istmo config dir.

use std::path::PathBuf;

use sismo_analysis::{Group, Groups, Rule};

const CODEC: bincode::config::Configuration = bincode::config::standard();
const FILE: &str = "viewer.bin";
const MAX_RECENT: usize = 12;
const MAX_RECENT_QUERIES: usize = 10;

/// Interface language; `None` follows the system.
#[istmo::message]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Spanish,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Spanish, Language::English];

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Language::Spanish => "es-ES",
            Language::English => "en-US",
        }
    }

    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Language::Spanish => "Español",
            Language::English => "English",
        }
    }

    /// Spanish for `es*`, English otherwise.
    #[must_use]
    pub fn from_subtag(subtag: &str) -> Self {
        if subtag.to_ascii_lowercase().starts_with("es") {
            Language::Spanish
        } else {
            Language::English
        }
    }
}

/// One crate rule, as stored (`group`: 0 app, 1 framework, 2 other).
#[istmo::message]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRule {
    pub pattern: String,
    pub group: u8,
}

/// Rules an app's crates follow ahead of the built-in defaults.
#[istmo::message]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGroups {
    pub app: String,
    pub rules: Vec<StoredRule>,
}

/// An app this viewer paired with.
#[istmo::message]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedApp {
    pub app: String,
    pub device: String,
    pub fingerprint: String,
    pub last_seen_unix: u64,
}

#[istmo::message]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedQuery {
    pub name: String,
    pub query: String,
}

#[istmo::message]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Prefs {
    pub language: Option<Language>,
    pub recordings_dir: Option<String>,
    pub recent: Vec<String>,
    pub groups: Vec<AppGroups>,
    pub paired: Vec<PairedApp>,
    pub saved_queries: Vec<SavedQuery>,
    pub recent_queries: Vec<String>,
    /// Seconds a live timeline shows by default.
    pub live_window_secs: u32,
}

fn group_code(group: Group) -> u8 {
    match group {
        Group::App => 0,
        Group::Framework => 1,
        Group::Other => 2,
    }
}

fn code_group(code: u8) -> Group {
    match code {
        0 => Group::App,
        1 => Group::Framework,
        _ => Group::Other,
    }
}

impl Prefs {
    fn path() -> PathBuf {
        istmo::path::config_dir().join(FILE)
    }

    /// Reads the preferences; anything missing or unreadable is the default.
    #[must_use]
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|bytes| bincode::decode_from_slice::<Self, _>(&bytes, CODEC).ok())
            .map(|(prefs, _)| prefs)
            .unwrap_or_default()
    }

    /// Writes atomically (temp file + rename); failures are only logged.
    pub fn save(&self) {
        let path = Self::path();
        let Ok(bytes) = bincode::encode_to_vec(self, CODEC) else {
            return;
        };
        let tmp = path.with_extension("tmp");
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(error) = result {
            eprintln!("could not save preferences to {}: {error}", path.display());
        }
    }

    /// Effective language: the chosen one, else the system's.
    #[must_use]
    pub fn language(&self) -> Language {
        self.language.unwrap_or_else(|| {
            super::platform::system_language()
                .map_or(Language::English, |s| Language::from_subtag(&s))
        })
    }

    #[must_use]
    pub fn recordings_dir(&self) -> PathBuf {
        self.recordings_dir
            .as_ref()
            .map_or_else(crate::workspace::recordings_dir, PathBuf::from)
    }

    #[must_use]
    pub fn live_window_ns(&self) -> u64 {
        u64::from(if self.live_window_secs == 0 {
            3
        } else {
            self.live_window_secs
        }) * 1_000_000_000
    }

    pub fn push_recent(&mut self, path: &std::path::Path) {
        let path = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string();
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        self.save();
    }

    #[must_use]
    pub fn recent_files(&self) -> Vec<PathBuf> {
        self.recent
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect()
    }

    pub fn push_query(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.recent_queries.retain(|q| q != query);
        self.recent_queries.insert(0, query.to_owned());
        self.recent_queries.truncate(MAX_RECENT_QUERIES);
        self.save();
    }

    /// The app's rules applied on top of the defaults.
    #[must_use]
    pub fn groups_for(&self, app: &str) -> Groups {
        let mut groups = Groups::default();
        groups.set_app(app);
        if let Some(stored) = self.groups.iter().find(|g| g.app == app) {
            for rule in stored.rules.iter().rev() {
                groups.rules.insert(
                    0,
                    Rule {
                        pattern: rule.pattern.clone(),
                        group: code_group(rule.group),
                    },
                );
            }
        }
        groups
    }

    /// Replaces the app's custom rules.
    pub fn set_groups(&mut self, app: &str, rules: &[(String, Group)]) {
        self.groups.retain(|g| g.app != app);
        if !rules.is_empty() {
            self.groups.push(AppGroups {
                app: app.to_owned(),
                rules: rules
                    .iter()
                    .map(|(pattern, group)| StoredRule {
                        pattern: pattern.clone(),
                        group: group_code(*group),
                    })
                    .collect(),
            });
        }
        self.save();
    }

    #[must_use]
    pub fn custom_rules(&self, app: &str) -> Vec<(String, Group)> {
        self.groups
            .iter()
            .find(|g| g.app == app)
            .map(|g| {
                g.rules
                    .iter()
                    .map(|r| (r.pattern.clone(), code_group(r.group)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn remember_pairing(&mut self, app: &str, device: &str, fingerprint: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.paired.retain(|p| p.fingerprint != fingerprint);
        self.paired.insert(
            0,
            PairedApp {
                app: app.to_owned(),
                device: device.to_owned(),
                fingerprint: fingerprint.to_owned(),
                last_seen_unix: now,
            },
        );
        self.save();
    }
}

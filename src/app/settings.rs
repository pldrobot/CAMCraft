//! User preferences, stored as `settings.json` in the config folder.

use crate::common::controls::Controls;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) controls: Controls,
}

impl Settings {
    fn file() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("CAMCraft").join("settings.json"))
    }

    pub(crate) fn load() -> Self {
        Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// A failed write only loses the preferences, so it is ignored.
    pub(crate) fn save(&self) {
        let Some(f) = Self::file() else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(f, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::controls::Act;
    use eframe::egui::Key;

    #[test]
    fn partial_file_keeps_defaults_and_keys_round_trip() {
        let s: Settings = serde_json::from_str(r#"{"controls":{"feed":900.0}}"#).unwrap();
        assert_eq!(s.controls.feed, 900.0);
        assert_eq!(s.controls.key(Act::HomeAll), Some(Key::H));
        let mut c = Controls::default();
        c.set_key(Act::HomeAll, Key::OpenBracket);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&Settings { controls: c }).unwrap()).unwrap();
        assert_eq!(back.controls.key(Act::HomeAll), Some(Key::OpenBracket));
    }
}

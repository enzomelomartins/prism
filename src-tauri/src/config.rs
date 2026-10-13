//! The handful of preferences that survive between runs, stored as JSON next
//! to the application data so the user never has to retype them.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub last_folder: String,
    pub container: String,
    pub audio_export_format: String,
    pub embed_metadata: bool,
    pub embed_thumbnail: bool,
    pub write_subs: bool,
    pub sub_langs: String,
    pub filename_pattern: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_folder: String::new(),
            container: "mp4".into(),
            audio_export_format: "mp3".into(),
            embed_metadata: true,
            embed_thumbnail: false,
            write_subs: false,
            sub_langs: "pt,en".into(),
            filename_pattern: crate::ytdlp::DEFAULT_PATTERN.into(),
        }
    }
}

/// Reads the stored preferences, falling back to the defaults for a first run
/// or a file that was edited into something unreadable.
pub fn load(path: &Path) -> AppConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Preferences are a convenience, so a failed write is never worth an error.
pub fn save(path: &Path, config: &AppConfig) {
    if let Ok(raw) = serde_json::to_string_pretty(config) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, raw);
    }
}

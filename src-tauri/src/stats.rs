use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AppStats {
    pub total_words: u64,
    pub total_dictation_seconds: f64,
    pub total_transcriptions: u64,
}

pub fn get_stats_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path.push("stats.json");
    path
}

pub fn load_stats(app: &AppHandle) -> AppStats {
    let path = get_stats_path(app);
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(stats) = serde_json::from_str(&content) {
                return stats;
            }
        }
    }
    AppStats::default()
}

pub fn save_stats(app: &AppHandle, stats: &AppStats) {
    let path = get_stats_path(app);
    match serde_json::to_string_pretty(stats) {
        Ok(content) => {
            if let Err(e) = fs::write(&path, content) {
                log::error!("Failed to write stats to {:?}: {}", path, e);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize stats: {}", e);
        }
    }
}

pub fn record_transcription(app: &AppHandle, text: &str, audio_duration_seconds: f64) {
    let mut stats = load_stats(app);
    let word_count = text.split_whitespace().count() as u64;
    stats.total_words += word_count;
    stats.total_dictation_seconds += audio_duration_seconds;
    stats.total_transcriptions += 1;
    save_stats(app, &stats);
}

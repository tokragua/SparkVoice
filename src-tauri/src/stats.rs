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

    // Log transcription to daily file if enabled
    let logging_enabled = {
        let state = app.state::<crate::AppState>();
        let enabled = state.settings.lock().transcription_logging_enabled;
        enabled
    };
    if logging_enabled {
        log_transcription(app, text, audio_duration_seconds);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TranscriptionLogEntry {
    timestamp: String,
    text: String,
    duration: f64,
}

fn log_transcription(app: &AppHandle, text: &str, audio_duration_seconds: f64) {
    use chrono::Local;

    let log_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("transcription_logs");

    if !log_dir.exists() {
        let _ = fs::create_dir_all(&log_dir);
    }

    // Use local time so the filename matches the user's calendar day
    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let timestamp = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();

    let log_path = log_dir.join(format!("{}.json", date_str));

    let entry = TranscriptionLogEntry {
        timestamp,
        text: text.to_string(),
        duration: audio_duration_seconds,
    };

    // Read existing entries or start fresh
    let mut entries: Vec<TranscriptionLogEntry> = if log_path.exists() {
        fs::read_to_string(&log_path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    entries.push(entry);

    match serde_json::to_string_pretty(&entries) {
        Ok(content) => {
            if let Err(e) = fs::write(&log_path, content) {
                log::error!("Failed to write transcription log to {:?}: {}", log_path, e);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize transcription log: {}", e);
        }
    }
}


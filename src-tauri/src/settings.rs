use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AppSettings {
    pub selected_language: String,
    pub languages: Vec<String>, // User's custom list of languages
    pub device: String,         // "cuda" or "cpu"
    pub input_device: Option<String>,
    pub pill_x: f32,
    pub pill_y: f32,
    pub selected_model: String,
    pub launch_on_startup: bool,
    pub recording_shortcut: String,
    pub max_recording_seconds: u32,
    pub pill_collapsed: bool,
    pub show_pill: bool,
    pub network_trigger_enabled: bool,
    pub network_trigger_port: u16,
    pub network_trigger_password: String,
    pub network_trigger_return_text: bool,
    pub transcription_logging_enabled: bool,
    // --- LLM Mind Map ---
    pub llm_mind_map_enabled: bool,
    pub llm_api_url: String,
    pub llm_model: String,
    pub llm_node_cap: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            selected_language: "en".to_string(),
            languages: vec!["en".to_string()],
            device: "cpu".to_string(),
            input_device: None,
            pill_x: 100.0,
            pill_y: 100.0,
            selected_model: "tiny".to_string(),
            launch_on_startup: false,
            recording_shortcut: "F2".to_string(),
            max_recording_seconds: 180,
            pill_collapsed: false,
            show_pill: true,
            network_trigger_enabled: false,
            network_trigger_port: 9876,
            network_trigger_password: "".to_string(),
            network_trigger_return_text: false,
            transcription_logging_enabled: false,
            llm_mind_map_enabled: false,
            llm_api_url: "http://localhost:11434/api/generate".to_string(),
            llm_model: "qwen2.5:3b-instruct-q4_k_m".to_string(),
            llm_node_cap: 4000,
        }
    }
}

pub fn get_settings_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path.push("settings.json");
    path
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let path = get_settings_path(app);
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(settings) = serde_json::from_str(&content) {
                return settings;
            }
        }
    }
    AppSettings::default()
}

pub fn save_settings(app: &AppHandle, settings: &AppSettings) {
    let path = get_settings_path(app);
    match serde_json::to_string_pretty(settings) {
        Ok(content) => {
            if let Err(e) = fs::write(&path, content) {
                log::error!("Failed to write settings to {:?}: {}", path, e);
            }
        }
        Err(e) => {
            log::error!("Failed to serialize settings: {}", e);
        }
    }
}

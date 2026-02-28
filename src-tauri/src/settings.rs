use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    pub selected_language: String,
    pub languages: Vec<String>, // User's custom list of languages
    pub device: String,         // "cuda" or "cpu"
    pub input_device: Option<String>,
    pub pill_x: f32,
    pub pill_y: f32,
    pub selected_model: String,
    pub launch_on_startup: bool,
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
    if let Ok(content) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(path, content);
    }
}

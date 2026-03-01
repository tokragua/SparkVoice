use log::{error, info};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

use crate::AppState;
use crate::settings::save_settings;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelMetadata {
    pub name: String,
    pub size: String,
    pub description: String,
}

/// (name, size_display, description, sha256_hash)
/// Hashes from: https://huggingface.co/ggerganov/whisper.cpp/tree/main
const AVAILABLE_MODELS: &[(&str, &str, &str, &str)] = &[
    ("tiny", "75 MiB", "Fastest, least accurate, multi-language", "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"),
    ("tiny.en", "75 MiB", "Fastest, least accurate, English only", "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"),
    ("base", "142 MiB", "Fast, reasonably accurate, multi-language", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"),
    ("base.en", "142 MiB", "Fast, reasonably accurate, English only", "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"),
    ("small", "466 MiB", "Good balance, multi-language", "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"),
    ("small.en", "466 MiB", "Good balance, English only", "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d"),
    ("medium", "1.5 GiB", "High accuracy, multi-language", "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208"),
    ("medium.en", "1.5 GiB", "High accuracy, English only", "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356"),
    ("large-v3", "2.9 GiB", "State of the art, multi-language", "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2"),
    ("large-v3-q5_0", "1.1 GiB", "Quantized large-v3", "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1"),
    ("large-v3-turbo", "1.5 GiB", "Fast large-v3, multi-language", "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"),
    ("large-v3-turbo-q5_0", "547 MiB", "Quantized large-v3-turbo", "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"),
];

/// Validates that a model name is safe (no path traversal, only allowed characters)
pub fn validate_model_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Model name cannot be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Invalid model name: contains path separators".to_string());
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err("Invalid model name: contains disallowed characters".to_string());
    }
    // Must be a known model
    if !AVAILABLE_MODELS.iter().any(|(n, _, _, _)| *n == name) {
        return Err(format!("Unknown model: {}", name));
    }
    Ok(())
}

/// Get the expected SHA256 hash for a model
pub fn get_model_hash(name: &str) -> Option<&'static str> {
    AVAILABLE_MODELS.iter()
        .find(|(n, _, _, _)| *n == name)
        .map(|(_, _, _, h)| *h)
}

/// Get the display size for a model
pub fn get_model_size_display(name: &str) -> &str {
    AVAILABLE_MODELS.iter().find(|(n, _, _, _)| *n == name).map(|(_, s, _, _)| *s).unwrap_or("unknown size")
}

/// Verify a file's SHA256 hash matches the expected value
pub fn verify_file_hash(path: &std::path::Path, expected_hash: &str) -> Result<bool, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Failed to open file for verification: {}", e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to read file for hash: {}", e))?;
    let result = hasher.finalize();
    let hash_hex = format!("{:x}", result);
    Ok(hash_hex == expected_hash)
}

// ── Tauri Commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_available_models() -> Vec<ModelMetadata> {
    AVAILABLE_MODELS
        .iter()
        .map(|(name, size, desc, _hash)| ModelMetadata {
            name: name.to_string(),
            size: size.to_string(),
            description: desc.to_string(),
        })
        .collect()
}

#[tauri::command]
pub fn get_downloaded_models(app: tauri::AppHandle) -> Vec<String> {
    let model_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    if !model_dir.exists() {
        return vec![];
    }

    let mut downloaded = vec![];
    if let Ok(entries) = std::fs::read_dir(model_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("ggml-") && name.ends_with(".bin") {
                let model_name = name.trim_start_matches("ggml-").trim_end_matches(".bin");
                downloaded.push(model_name.to_string());
            }
        }
    }
    downloaded
}

#[tauri::command]
pub fn select_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    model: String,
) -> Result<(), String> {
    validate_model_name(&model)?;
    let mut settings = state.settings.lock();
    settings.selected_model = model;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
pub fn delete_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    validate_model_name(&model)?;
    let model_filename = format!("ggml-{}.bin", model);
    let model_path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(model_filename);

    if model_path.exists() {
        std::fs::remove_file(model_path).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        Err("Model not found".to_string())
    }
}

#[tauri::command]
pub fn download_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
    validate_model_name(&model)?;
    let app_clone = app.clone();
    std::thread::spawn(move || {
        let model_filename = format!("ggml-{}.bin", model);
        let model_dir = app_clone
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        if !model_dir.exists() {
            let _ = std::fs::create_dir_all(&model_dir);
        }
        let model_path = model_dir.join(&model_filename);

        let size_display = get_model_size_display(&model);
        info!("Downloading model {} ({})...", model, size_display);
        let _ = app_clone.emit("model-download-status", format!("Downloading {} ({})...", model, size_display));

        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
            model
        );
        match reqwest::blocking::get(url) {
            Ok(mut response) if response.status().is_success() => {
                if let Ok(mut f) = std::fs::File::create(&model_path) {
                    if let Ok(_) = std::io::copy(&mut response, &mut f) {
                        // Verify hash integrity
                        if let Some(expected_hash) = get_model_hash(&model) {
                            match verify_file_hash(&model_path, expected_hash) {
                                Ok(true) => {
                                    info!("Download complete and verified: {:?}", model_path);
                                    let _ = app_clone.emit("model-download-status", "ready".to_string());
                                }
                                Ok(false) => {
                                    error!("Hash mismatch for downloaded model {}!", model);
                                    let _ = std::fs::remove_file(&model_path);
                                    let _ = app_clone.emit("model-download-status", "error: integrity check failed".to_string());
                                }
                                Err(e) => {
                                    error!("Hash verification error: {}", e);
                                    let _ = std::fs::remove_file(&model_path);
                                    let _ = app_clone.emit("model-download-status", "error: verification failed".to_string());
                                }
                            }
                        } else {
                            info!("Download complete (no hash): {:?}", model_path);
                            let _ = app_clone.emit("model-download-status", "ready".to_string());
                        }
                    } else {
                        let _ = std::fs::remove_file(&model_path); // Clean up partial
                        let _ = app_clone.emit("model-download-status", "error: write failed".to_string());
                    }
                }
            }
            _ => {
                let _ = app_clone.emit(
                    "model-download-status",
                    "error: download failed".to_string(),
                );
            }
        }
    });
    Ok(())
}

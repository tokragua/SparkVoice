mod settings;
mod whisper;

use crate::settings::{load_settings, save_settings, AppSettings};
use crate::whisper::{capture_audio, list_input_devices, transcribe, WhisperState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use whisper_rs::WhisperContext;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelMetadata {
    pub name: String,
    pub size: String,
    pub description: String,
}

/// (name, size_display, description, sha256_hash)
/// To enable hash verification, populate with real hashes from:
/// https://huggingface.co/ggerganov/whisper.cpp/tree/main
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
fn validate_model_name(name: &str) -> Result<(), String> {
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
fn get_model_hash(name: &str) -> Option<&'static str> {
    AVAILABLE_MODELS.iter()
        .find(|(n, _, _, _)| *n == name)
        .map(|(_, _, _, h)| *h)
}

/// Get the display size for a model
fn get_model_size_display(name: &str) -> &str {
    AVAILABLE_MODELS.iter().find(|(n, _, _, _)| *n == name).map(|(_, s, _, _)| *s).unwrap_or("unknown size")
}

/// Verify a file's SHA256 hash matches the expected value
fn verify_file_hash(path: &std::path::Path, expected_hash: &str) -> Result<bool, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Failed to open file for verification: {}", e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to read file for hash: {}", e))?;
    let result = hasher.finalize();
    let hash_hex = format!("{:x}", result);
    Ok(hash_hex == expected_hash)
}
use enigo::{Enigo, Keyboard, Settings};
use log::{error, info, warn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_log::{Target, TargetKind};

struct AppState {
    whisper_state: Arc<Mutex<WhisperState>>,
    settings: Mutex<AppSettings>,
    device_tx: Mutex<mpsc::Sender<Option<String>>>,
    typer_tx: mpsc::Sender<String>,
    is_transcribing: Mutex<bool>,
    is_cancelled: Arc<AtomicBool>,
    // (model_name, use_gpu, context)
    model_cache: Mutex<Option<(String, bool, Arc<WhisperContext>)>>,
    /// Last time pill position was persisted to disk (for debouncing)
    pill_save_timer: Mutex<Instant>,
}

#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn get_input_devices() -> Vec<String> {
    list_input_devices()
}

#[tauri::command]
fn set_input_device(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    device: String,
) -> Result<(), String> {
    // Update live stream
    let tx = state.device_tx.lock();
    tx.send(Some(device.clone())).map_err(|e| e.to_string())?;

    // Save to settings
    let mut settings = state.settings.lock();
    settings.input_device = if device.is_empty() {
        None
    } else {
        Some(device)
    };
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    state.settings.lock().clone()
}

#[tauri::command]
fn set_shortcut(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    shortcut_str: String,
) -> Result<(), String> {
    let new_shortcut: Shortcut = shortcut_str
        .parse()
        .map_err(|e| format!("Invalid shortcut: {}", e))?;

    let mut settings = state.settings.lock();
    let old_shortcut_str = settings.recording_shortcut.clone();

    // Unregister old shortcut if it exists
    if let Ok(old_shortcut) = old_shortcut_str.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(old_shortcut);
    }

    // Register new shortcut
    app.global_shortcut()
        .register(new_shortcut)
        .map_err(|e| format!("Failed to register shortcut: {}", e))?;

    settings.recording_shortcut = shortcut_str;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_language(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    settings.selected_language = lang;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_pill_collapsed(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    collapsed: bool,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    settings.pill_collapsed = collapsed;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn add_language(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    if !settings.languages.contains(&lang) {
        settings.languages.push(lang);
        save_settings(&app, &settings);
    }
    Ok(())
}

#[tauri::command]
fn remove_language(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    settings.languages.retain(|l| l != &lang);
    if settings.selected_language == lang {
        settings.selected_language = settings
            .languages
            .first()
            .cloned()
            .unwrap_or_else(|| "en".to_string());
    }
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_device(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    device: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    settings.device = device;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn toggle_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    // Read settings BEFORE locking whisper_state to maintain consistent lock ordering
    let max_recording_seconds = state.settings.lock().max_recording_seconds;

    let is_recording = {
        let mut ws = state.whisper_state.lock();
        ws.is_recording = !ws.is_recording;

        if ws.is_recording {
            ws.max_samples = Some(max_recording_seconds as usize * 16000);
            ws.audio_buffer.clear();
        }

        ws.is_recording
    };

    // Notify frontend with absolute state
    let _ = app.emit("recording-toggled", is_recording);

    if !is_recording {
        stop_and_transcribe(app.clone());
    }
}

fn stop_and_transcribe(app: tauri::AppHandle) {
    let state = app.state::<AppState>();

    // Check if we are already transcribing
    {
        let mut transcribing = state.is_transcribing.lock();
        if *transcribing {
            warn!("Transcription already in progress, ignoring stop request.");
            return;
        }
        *transcribing = true;
        state.is_cancelled.store(false, Ordering::SeqCst);
    }

    // Notify pill UI that processing started
    let _ = app.emit("transcribing-toggled", true);

    let audio_data = {
        let mut ws = state.whisper_state.lock();
        let data = ws.audio_buffer.clone();
        ws.audio_buffer.clear();
        ws.is_recording = false;
        data
    };

    let tx = state.typer_tx.clone();
    let settings = state.settings.lock().clone();
    let app_handle = app.clone();
    let is_cancelled = state.is_cancelled.clone();

    std::thread::spawn(move || {
        let state = app_handle.state::<AppState>();
        let model_name = settings.selected_model.clone();
        let use_gpu = settings.device == "cuda";

        info!(
            "Transcription started: model={}, device={}, audio_duration={:.2}s",
            model_name,
            if use_gpu { "CUDA" } else { "CPU" },
            audio_data.len() as f32 / 16000.0
        );

        // Safety check: English-only model with non-English language selected
        if (settings.selected_language != "en" && settings.selected_language != "auto")
            && model_name.ends_with(".en")
        {
            let _ = app_handle.emit(
                "status-update",
                format!(
                    "Warning: {} is English-only. Transcription may fail.",
                    model_name
                ),
            );
        }
        if settings.selected_language == "auto" && model_name.ends_with(".en") {
            let _ = app_handle.emit(
                "status-update",
                format!(
                    "Warning: Auto-detect requires a multilingual model. {} is English-only.",
                    model_name
                ),
            );
        }

        // Validate model name to prevent path traversal
        if let Err(e) = validate_model_name(&model_name) {
            error!("Invalid model name: {}", e);
            let _ = app_handle.emit("status-update", format!("Invalid model: {}", e));
            *state.is_transcribing.lock() = false;
            let _ = app_handle.emit("transcribing-toggled", false);
            return;
        }

        let model_filename = format!("ggml-{}.bin", model_name);
        let model_dir = app_handle.path().app_data_dir().unwrap_or_default();
        if !model_dir.exists() {
            let _ = std::fs::create_dir_all(&model_dir);
        }
        let model_path = model_dir.join(&model_filename);
        let dev_path = std::path::PathBuf::from("src-tauri").join(&model_filename);

        if !model_path.exists() && !dev_path.exists() {
            // Try to download it if truly missing
            let size_display = get_model_size_display(&model_name);
            info!(
                "Required model {} missing. Attempting download...",
                model_filename
            );
            let _ = app_handle.emit(
                "status-update",
                format!("Downloading {} model ({})...", model_name, size_display),
            );

            let url = format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
                model_name
            );
            match reqwest::blocking::get(url) {
                Ok(mut response) if response.status().is_success() => {
                    match std::fs::File::create(&model_path) {
                        Ok(mut f) => {
                            if let Ok(_) = std::io::copy(&mut response, &mut f) {
                                // Verify hash integrity
                                if let Some(expected_hash) = get_model_hash(&model_name) {
                                    match verify_file_hash(&model_path, expected_hash) {
                                        Ok(true) => {
                                            info!("Download complete and verified: {:?}", model_path);
                                            let _ = app_handle.emit("status-update", format!("{} model ready.", model_name));
                                        }
                                        Ok(false) => {
                                            error!("Hash mismatch for model {}! File may be corrupted or tampered with.", model_name);
                                            let _ = std::fs::remove_file(&model_path);
                                            let _ = app_handle.emit("status-update", "Download failed: integrity check failed.");
                                        }
                                        Err(e) => {
                                            error!("Hash verification error: {}", e);
                                            let _ = std::fs::remove_file(&model_path);
                                            let _ = app_handle.emit("status-update", "Download failed: verification error.");
                                        }
                                    }
                                } else {
                                    info!("Download complete (no hash available): {:?}", model_path);
                                    let _ = app_handle.emit("status-update", format!("{} model ready.", model_name));
                                }
                            } else {
                                error!("Failed to write model file at {:?}", model_path);
                                let _ = std::fs::remove_file(&model_path); // Clean up partial download
                            }
                        }
                        Err(e) => {
                            error!("Failed to create model file at {:?}: {}", model_path, e);
                            let _ = app_handle.emit("status-update", "Failed to create model file.");
                        }
                    }
                }
                Ok(response) => error!("Download failed with status: {}", response.status()),
                Err(e) => error!("Download request error: {}", e),
            }
        }

        let final_model_path = if model_path.exists() {
            model_path
        } else if dev_path.exists() {
            dev_path
        } else {
            error!("Model not found: {}", model_filename);
            let _ = app_handle.emit("status-update", "Model not found.");
            *state.is_transcribing.lock() = false;
            let _ = app_handle.emit("transcribing-toggled", false);
            return;
        };

        // Check cache or initialize
        let mut cache = state.model_cache.lock();
        let context = if let Some((cached_name, cached_gpu, ctx)) = &*cache {
            if cached_name == &model_name && *cached_gpu == use_gpu {
                Some(ctx.clone())
            } else {
                None
            }
        } else {
            None
        };

        let context = if let Some(ctx) = context {
            ctx
        } else {
            let mut params = whisper_rs::WhisperContextParameters::default();
            params.use_gpu = use_gpu;
            match whisper_rs::WhisperContext::new_with_params(
                final_model_path.to_str().unwrap(),
                params,
            ) {
                Ok(ctx) => {
                    let arc_ctx = Arc::new(ctx);
                    *cache = Some((model_name, use_gpu, arc_ctx.clone()));
                    arc_ctx
                }
                Err(e) => {
                    error!("Failed to initialize Whisper: {}", e);
                    let _ = app_handle.emit("status-update", "Engine error.");
                    *state.is_transcribing.lock() = false;
                    let _ = app_handle.emit("transcribing-toggled", false);
                    return;
                }
            }
        };
        drop(cache);

        perform_transcription(
            &app_handle,
            &tx,
            &context,
            &audio_data,
            &settings.selected_language,
            &settings.languages,
            is_cancelled,
        );

        // Reset guards
        *state.is_transcribing.lock() = false;
        let _ = app_handle.emit("transcribing-toggled", false);
    });
}

#[tauri::command]
fn set_max_recording_duration(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    duration: u32,
) -> Result<(), String> {
    // Clamp to [10, 3600] to prevent excessive memory allocation
    let clamped = duration.clamp(10, 3600);
    let mut settings = state.settings.lock();
    settings.max_recording_seconds = clamped;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_launch_on_startup(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = state.settings.lock();
    settings.launch_on_startup = enabled;
    save_settings(&app, &settings);

    let autostart_manager = app.autolaunch();
    if enabled {
        let _ = autostart_manager.enable();
    } else {
        let _ = autostart_manager.disable();
    }
    Ok(())
}

fn perform_transcription(
    app: &tauri::AppHandle,
    tx: &mpsc::Sender<String>,
    ctx: &whisper_rs::WhisperContext,
    audio_data: &[f32],
    lang: &str,
    allowed_langs: &[String],
    is_cancelled: Arc<AtomicBool>,
) {
    match transcribe(ctx, audio_data, lang, allowed_langs, is_cancelled) {
        Ok(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty()
                && !trimmed.contains("[BLANK_AUDIO]")
                && !trimmed.contains("[SILENCE]")
            {
                info!("Transcribed text: {}", trimmed);
                let _ = app.emit("transcribed-text", trimmed.to_string());
                let _ = tx.send(trimmed.to_string());
            }
        }
        Err(e) => {
            error!("Transcription error: {}", e);
            let _ = app.emit("status-update", "Transcription error.");
        }
    }
}

#[tauri::command]
fn get_available_models() -> Vec<ModelMetadata> {
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
fn get_downloaded_models(app: tauri::AppHandle) -> Vec<String> {
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
fn select_model(
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
fn delete_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
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

// download_model is already partly implemented in the spawn loop logic but we should move it to a standalone command
#[tauri::command]
fn download_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
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

#[tauri::command]
fn start_dragging(window: tauri::Window) {
    let _ = window.start_dragging();
}

#[tauri::command]
fn is_cuda_supported() -> bool {
    cfg!(feature = "cuda")
}

#[tauri::command]
fn open_settings(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

#[tauri::command]
fn cancel_transcription(state: tauri::State<'_, AppState>) {
    state.is_cancelled.store(true, Ordering::SeqCst);
    info!("Transcription cancellation requested.");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(unused_variables, unused_assignments)]
pub fn run() {
    // Custom logging callback for whisper.cpp (must be C-compatible)
    unsafe extern "C" fn whisper_log_callback(
        level: i32,
        message: *const std::ffi::c_char,
        _user_data: *mut std::ffi::c_void,
    ) {
        if message.is_null() {
            return;
        }
        let c_str = std::ffi::CStr::from_ptr(message);
        let msg = c_str.to_string_lossy();
        let trimmed = msg.trim();
        if trimmed.is_empty() {
            return;
        }

        // whisper.cpp log levels: 0=error, 1=warn, 2=info, 3=debug
        match level {
            0 => error!("[whisper.cpp] {}", trimmed),
            1 => warn!("[whisper.cpp] {}", trimmed),
            2 => info!("[whisper.cpp] {}", trimmed),
            _ => info!("[whisper.cpp] {}", trimmed), // Treat debug as info for now
        }
    }
    unsafe {
        whisper_rs::set_log_callback(Some(whisper_log_callback), std::ptr::null_mut());
    }

    let (tx, _rx) = mpsc::channel::<Option<String>>();

    #[cfg(target_os = "windows")]
    let log_dir = std::env::var("APPDATA")
        .map(|p| {
            std::path::PathBuf::from(p)
                .join("com.sparkvoice.app")
                .join("logs")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    #[cfg(not(target_os = "windows"))]
    let log_dir = std::path::PathBuf::from(".");

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::Folder {
                        path: log_dir,
                        file_name: None,
                    }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<AppState>();
                        let settings = state.settings.lock();
                        if let Ok(current_shortcut) =
                            settings.recording_shortcut.parse::<Shortcut>()
                        {
                            if shortcut == &current_shortcut {
                                drop(settings);
                                toggle_recording(app.clone(), state);
                            }
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let settings = load_settings(app.handle());

            app.manage(AppState {
                whisper_state: Arc::new(Mutex::new(WhisperState {
                    is_recording: false,
                    audio_buffer: Vec::new(),
                    current_amplitude: 0.0,
                    max_samples: Some(settings.max_recording_seconds as usize * 16000),
                })),
                settings: Mutex::new(settings.clone()),
                device_tx: Mutex::new(tx),
                typer_tx: {
                    let (typer_tx, typer_rx) = mpsc::channel::<String>();
                    std::thread::spawn(move || {
                        let mut enigo =
                            Enigo::new(&Settings::default()).expect("Failed to initialize Enigo");
                        while let Ok(text) = typer_rx.recv() {
                            let _ = enigo.text(&text);
                        }
                    });
                    typer_tx
                },
                model_cache: Mutex::new(None),
                is_transcribing: Mutex::new(false),
                is_cancelled: Arc::new(AtomicBool::new(false)),
                pill_save_timer: Mutex::new(Instant::now()),
            });

            // Log system capabilities for performance debugging
            info!("Whisper System Info: {}", whisper_rs::print_system_info());

            let app_handle_for_listener = app.handle().clone();
            app.listen("recording-auto-stopped", move |_| {
                info!("Automatic recording stop triggered by limit.");
                stop_and_transcribe(app_handle_for_listener.clone());
                let _ = app_handle_for_listener.emit("recording-toggled", false);
            });

            let state = app.state::<AppState>();
            let whisper_state = state.whisper_state.clone();

            // Audio Manager Thread
            let (device_tx, device_rx) = mpsc::channel::<Option<String>>();
            *state.device_tx.lock() = device_tx;

            let whisper_state_for_audio = whisper_state.clone();
            let initial_device = settings.input_device.clone();

            let app_handle_for_audio = app.handle().clone();
            std::thread::spawn(move || {
                let mut current_stream: Option<cpal::Stream> = None;

                // Initialize with saved device or default
                if let Ok(stream) = capture_audio(
                    app_handle_for_audio.clone(),
                    whisper_state_for_audio.clone(),
                    initial_device,
                ) {
                    current_stream = Some(stream);
                }

                while let Ok(device_name) = device_rx.recv() {
                    current_stream = None; // Drop old stream
                    if let Ok(stream) = capture_audio(
                        app_handle_for_audio.clone(),
                        whisper_state_for_audio.clone(),
                        device_name,
                    ) {
                        current_stream = Some(stream);
                    }
                }
            });

            // Stream amplitude to pill UI
            let whisper_state_for_amp = state.whisper_state.clone();
            let app_handle_for_amp = app.handle().clone();
            std::thread::spawn(move || loop {
                let amp = {
                    let mut ws = whisper_state_for_amp.lock();
                    let a = ws.current_amplitude;
                    ws.current_amplitude = 0.0;
                    a
                };

                let _ = app_handle_for_amp.emit("audio-amplitude", amp);
                std::thread::sleep(std::time::Duration::from_millis(50));
            });

            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "settings" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // Register shortcut from settings
            let shortcut_str = settings.recording_shortcut.clone();
            if let Ok(shortcut) = shortcut_str.parse::<Shortcut>() {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    error!("Failed to register shortcut {}: {}", shortcut_str, e);
                }
            }

            // Apply saved pill position
            if let Some(pill) = app.get_webview_window("pill") {
                let _ = pill.set_focusable(false);
                let state = app.state::<AppState>();
                let settings = state.settings.lock();
                let _ = pill.set_position(tauri::PhysicalPosition::new(
                    settings.pill_x as i32,
                    settings.pill_y as i32,
                ));
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "pill" {
                if let tauri::WindowEvent::Moved(pos) = event {
                    let app = window.app_handle();
                    let state = app.state::<AppState>();
                    // Update in memory immediately
                    let mut settings = state.settings.lock();
                    settings.pill_x = pos.x as f32;
                    settings.pill_y = pos.y as f32;
                    // Debounce: only save to disk at most once per 500ms
                    let mut last_save = state.pill_save_timer.lock();
                    if last_save.elapsed() >= std::time::Duration::from_millis(500) {
                        save_settings(app, &settings);
                        *last_save = Instant::now();
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            toggle_recording,
            get_input_devices,
            set_input_device,
            get_settings,
            set_language,
            add_language,
            remove_language,
            start_dragging,
            get_available_models,
            get_downloaded_models,
            select_model,
            delete_model,
            download_model,
            set_device,
            set_launch_on_startup,
            set_shortcut,
            set_max_recording_duration,
            get_app_version,
            is_cuda_supported,
            cancel_transcription,
            open_settings,
            set_pill_collapsed,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

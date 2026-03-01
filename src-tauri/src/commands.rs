use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::AppState;
use crate::models::{validate_model_name, get_model_hash, get_model_size_display, verify_file_hash};
use crate::settings::{save_settings, AppSettings};
use crate::whisper::{list_input_devices, transcribe};

// ── Utility Commands ────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
pub fn get_input_devices() -> Vec<String> {
    list_input_devices()
}

#[tauri::command]
pub fn start_dragging(window: tauri::Window) {
    let _ = window.start_dragging();
}

#[tauri::command]
pub fn is_cuda_supported() -> bool {
    cfg!(feature = "cuda")
}

#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) {
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
pub fn cancel_transcription(state: tauri::State<'_, AppState>) {
    state.is_cancelled.store(true, Ordering::SeqCst);
    info!("Transcription cancellation requested.");
}

// ── Settings Commands ───────────────────────────────────────────────────────

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    state.settings.lock().clone()
}

#[tauri::command]
pub fn set_input_device(
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
pub fn set_shortcut(
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
pub fn set_language(
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
pub fn set_pill_collapsed(
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
pub fn add_language(
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
pub fn remove_language(
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
pub fn set_device(
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
pub fn set_max_recording_duration(
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
pub fn set_launch_on_startup(
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

// ── Recording & Transcription ───────────────────────────────────────────────

#[tauri::command]
pub fn toggle_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
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

pub fn stop_and_transcribe(app: tauri::AppHandle) {
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

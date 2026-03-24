use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::{AppState, AudioCommand};
use crate::errors::AppError;
use crate::models::{download_model_to_path, get_model_size_display, validate_model_name};
use crate::settings::{save_settings, AppSettings};
use crate::stats::{self, AppStats};
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
pub fn is_metal_supported() -> bool {
    cfg!(feature = "metal")
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

#[tauri::command]
pub fn get_stats(app: tauri::AppHandle) -> AppStats {
    stats::load_stats(&app)
}

#[tauri::command]
pub fn set_network_trigger(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) {
    {
        let mut settings = state.settings.lock();
        settings.network_trigger_enabled = enabled;
        save_settings(&app, &settings);
    }
    if enabled {
        crate::network_trigger::start_server(&app);
    } else {
        crate::network_trigger::stop_server();
    }
    info!("Network Trigger {}", if enabled { "enabled" } else { "disabled" });
}

#[tauri::command]
pub fn set_network_trigger_password(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    password: String,
) {
    let mut settings = state.settings.lock();
    settings.network_trigger_password = password;
    save_settings(&app, &settings);
    // Restart server if running to pick up new password
    if settings.network_trigger_enabled {
        drop(settings);
        crate::network_trigger::stop_server();
        crate::network_trigger::start_server(&app);
    }
}

#[tauri::command]
pub fn set_network_trigger_port(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    port: u16,
) {
    let mut settings = state.settings.lock();
    settings.network_trigger_port = port;
    save_settings(&app, &settings);
    // Restart server if running to pick up new port
    if settings.network_trigger_enabled {
        drop(settings);
        crate::network_trigger::stop_server();
        crate::network_trigger::start_server(&app);
    }
}

#[tauri::command]
pub fn set_network_trigger_return_text(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) {
    let mut settings = state.settings.lock();
    settings.network_trigger_return_text = enabled;
    save_settings(&app, &settings);
    info!("Network Trigger return text {}", if enabled { "enabled" } else { "disabled" });
}

#[tauri::command]
pub fn get_local_ip() -> String {
    crate::network_trigger::get_local_ip()
}

#[tauri::command]
pub fn set_transcription_logging(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) {
    let mut settings = state.settings.lock();
    settings.transcription_logging_enabled = enabled;
    save_settings(&app, &settings);
    info!("Transcription logging {}", if enabled { "enabled" } else { "disabled" });
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
) -> Result<(), AppError> {
    let tx = state.audio_cmd_tx.lock();
    let device_opt = if device.is_empty() { None } else { Some(device.clone()) };
    tx.send(AudioCommand::SetDevice(device_opt.clone()))
        .map_err(|e| AppError::AudioDevice(e.to_string()))?;

    let mut settings = state.settings.lock();
    settings.input_device = device_opt;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
pub fn set_shortcut(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    shortcut_str: String,
) -> Result<(), AppError> {
    let new_shortcut: Shortcut = shortcut_str
        .parse()
        .map_err(|e| AppError::Config(format!("Invalid shortcut: {}", e)))?;

    let mut settings = state.settings.lock();
    let old_shortcut_str = settings.recording_shortcut.clone();

    if let Ok(old_shortcut) = old_shortcut_str.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(old_shortcut);
    }

    app.global_shortcut()
        .register(new_shortcut)
        .map_err(|e| AppError::Config(format!("Failed to register shortcut: {}", e)))?;

    settings.recording_shortcut = shortcut_str;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
pub fn set_language(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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
) -> Result<(), AppError> {
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

    // On macOS, start/stop the audio stream to control the orange microphone indicator
    #[cfg(target_os = "macos")]
    {
        let tx = state.audio_cmd_tx.lock();
        if is_recording {
            let _ = tx.send(AudioCommand::StartStream);
        } else {
            let _ = tx.send(AudioCommand::StopStream);
        }
    }

    // If starting recording and pill is hidden, force show it so the user can see recording status
    if is_recording {
        let is_hidden = !state.settings.lock().show_pill;
        if is_hidden {
            if let Some(pill) = app.get_webview_window("pill") {
                let _ = pill.show();
            }
        }
    }

    let _ = app.emit("recording-toggled", is_recording);

    if !is_recording {
        stop_and_transcribe(app.clone());
    }
}

pub fn stop_and_transcribe(app: tauri::AppHandle) {
    let state = app.state::<AppState>();

    {
        let mut transcribing = state.is_transcribing.lock();
        if *transcribing {
            warn!("Transcription already in progress, ignoring stop request.");
            return;
        }
        *transcribing = true;
        state.is_cancelled.store(false, Ordering::SeqCst);
    }

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
        let use_gpu = settings.device == "cuda" || settings.device == "metal";

        info!(
            "Transcription started: model={}, device={}, audio_duration={:.2}s",
            model_name,
            &settings.device,
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

        // Validate model name
        if let Err(e) = validate_model_name(&model_name) {
            error!("{}", e);
            let _ = app_handle.emit("status-update", format!("{}", e));
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

        // Auto-download missing model using the shared download function
        if !model_path.exists() && !dev_path.exists() {
            let size_display = get_model_size_display(&model_name);
            let _ = app_handle.emit(
                "status-update",
                format!("Downloading {} model ({})...", model_name, size_display),
            );

            match download_model_to_path(&model_name, &model_dir) {
                Ok(_) => {
                    let _ = app_handle.emit("status-update", format!("{} model ready.", model_name));
                }
                Err(e) => {
                    error!("{}", e);
                    let _ = app_handle.emit("status-update", format!("{}", e));
                    *state.is_transcribing.lock() = false;
                    let _ = app_handle.emit("transcribing-toggled", false);
                    return;
                }
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
    let audio_duration_seconds = audio_data.len() as f64 / 16000.0;
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
                // Record usage statistics
                stats::record_transcription(app, trimmed, audio_duration_seconds);
            }
        }
        Err(e) => {
            error!("Transcription error: {}", e);
            let _ = app.emit("status-update", "Transcription error.");
        }
    }
}

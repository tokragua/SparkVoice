mod settings;
mod whisper;

use crate::settings::{load_settings, save_settings, AppSettings};
use crate::whisper::{capture_audio, list_input_devices, transcribe, WhisperState};
use serde::{Deserialize, Serialize};
use whisper_rs::WhisperContext;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelMetadata {
    pub name: String,
    pub size: String,
    pub description: String,
}

const AVAILABLE_MODELS: &[(&str, &str, &str)] = &[
    ("tiny", "75 MiB", "Fastest, least accurate, multi-language"),
    ("tiny.en", "75 MiB", "Fastest, least accurate, English only"),
    (
        "base",
        "142 MiB",
        "Fast, reasonably accurate, multi-language",
    ),
    (
        "base.en",
        "142 MiB",
        "Fast, reasonably accurate, English only",
    ),
    ("small", "466 MiB", "Good balance, multi-language"),
    ("small.en", "466 MiB", "Good balance, English only"),
    ("small.en-tdrz", "465 MiB", "Small English with Tinydrz"),
    ("medium", "1.5 GiB", "High accuracy, multi-language"),
    ("medium.en", "1.5 GiB", "High accuracy, English only"),
    ("large-v1", "2.9 GiB", "Very high accuracy, multi-language"),
    (
        "large-v2",
        "2.9 GiB",
        "Very high accuracy, multi-language (v2)",
    ),
    ("large-v2-q5_0", "1.1 GiB", "Quantized large-v2"),
    ("large-v3", "2.9 GiB", "State of the art, multi-language"),
    ("large-v3-q5_0", "1.1 GiB", "Quantized large-v3"),
    ("large-v3-turbo", "1.5 GiB", "Fast large-v3, multi-language"),
    ("large-v3-turbo-q5_0", "547 MiB", "Quantized large-v3-turbo"),
];
use enigo::{Enigo, Keyboard, Settings};
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
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
    let tx = state.device_tx.lock().unwrap();
    tx.send(Some(device.clone())).map_err(|e| e.to_string())?;

    // Save to settings
    let mut settings = state.settings.lock().unwrap();
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
    state.settings.lock().unwrap().clone()
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

    let mut settings = state.settings.lock().unwrap();
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
    let mut settings = state.settings.lock().unwrap();
    settings.selected_language = lang;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn add_language(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
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
    let mut settings = state.settings.lock().unwrap();
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
    let mut settings = state.settings.lock().unwrap();
    settings.device = device;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn toggle_recording(app: tauri::AppHandle, state: tauri::State<'_, AppState>) {
    let is_recording = {
        let mut ws = state.whisper_state.lock().unwrap();
        ws.is_recording = !ws.is_recording;

        if ws.is_recording {
            // Set max samples based on settings
            let settings = state.settings.lock().unwrap();
            ws.max_samples = Some(settings.max_recording_seconds as usize * 16000);
            ws.audio_buffer.clear(); // Clear buffer on start
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
    let mut transcribing = state.is_transcribing.lock().unwrap();
    if *transcribing {
        warn!("Transcription already in progress, ignoring stop request.");
        return;
    }
    *transcribing = true;
    state.is_cancelled.store(false, Ordering::SeqCst);
    drop(transcribing);

    // Notify pill UI that processing started
    let _ = app.emit("transcribing-toggled", true);

    let mut ws = state.whisper_state.lock().unwrap();
    let audio_data = ws.audio_buffer.clone();
    ws.audio_buffer.clear();
    ws.is_recording = false;
    drop(ws);

    let tx = state.typer_tx.clone();
    let settings = state.settings.lock().unwrap().clone();
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

        let model_filename = format!("ggml-{}.bin", model_name);
        let model_dir = app_handle.path().app_data_dir().unwrap_or_default();
        if !model_dir.exists() {
            let _ = std::fs::create_dir_all(&model_dir);
        }
        let model_path = model_dir.join(&model_filename);
        let dev_path = std::path::PathBuf::from("src-tauri").join(&model_filename);

        if !model_path.exists() && !dev_path.exists() {
            // Try to download it if truly missing
            info!(
                "Required model {} missing. Attempting download...",
                model_filename
            );
            let _ = app_handle.emit(
                "status-update",
                format!("Downloading {} model (75MB)...", model_name),
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
                                info!("Download complete: {:?}", model_path);
                                let _ = app_handle
                                    .emit("status-update", format!("{} model ready.", model_name));
                            } else {
                                error!("Failed to write model file at {:?}", model_path);
                            }
                        }
                        Err(e) => {
                            error!("Failed to create model file at {:?}: {}", model_path, e);
                            let _ =
                                app_handle.emit("status-update", "Failed to create model file.");
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
            *state.is_transcribing.lock().unwrap() = false;
            let _ = app_handle.emit("transcribing-toggled", false);
            return;
        };

        // Check cache or initialize
        let mut cache = state.model_cache.lock().unwrap();
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
                    *state.is_transcribing.lock().unwrap() = false;
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
        *state.is_transcribing.lock().unwrap() = false;
        let _ = app_handle.emit("transcribing-toggled", false);
    });
}

#[tauri::command]
fn set_max_recording_duration(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    duration: u32,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    settings.max_recording_seconds = duration;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_launch_on_startup(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
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
        .map(|(name, size, desc)| ModelMetadata {
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
    let mut settings = state.settings.lock().unwrap();
    settings.selected_model = model;
    save_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
fn delete_model(app: tauri::AppHandle, model: String) -> Result<(), String> {
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

        info!("Downloading model {}...", model);
        let _ = app_clone.emit("model-download-status", format!("Downloading {}...", model));

        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
            model
        );
        match reqwest::blocking::get(url) {
            Ok(mut response) if response.status().is_success() => {
                if let Ok(mut f) = std::fs::File::create(&model_path) {
                    if let Ok(_) = std::io::copy(&mut response, &mut f) {
                        info!("Download complete: {:?}", model_path);
                        let _ = app_clone.emit("model-download-status", "ready".to_string());
                    } else {
                        let _ = app_clone
                            .emit("model-download-status", "error: write failed".to_string());
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
                        let settings = state.settings.lock().unwrap();
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
            *state.device_tx.lock().unwrap() = device_tx;

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
                let mut ws = whisper_state_for_amp.lock().unwrap();
                let amp = ws.current_amplitude;
                ws.current_amplitude = 0.0; // Reset for peak detection
                drop(ws);

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
                let state = app.state::<AppState>();
                let settings = state.settings.lock().unwrap();
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
                    let mut settings = state.settings.lock().unwrap();
                    settings.pill_x = pos.x as f32;
                    settings.pill_y = pos.y as f32;
                    save_settings(app, &settings);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

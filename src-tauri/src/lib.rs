mod settings;
mod whisper;

use crate::settings::{load_settings, save_settings, AppSettings};
use crate::whisper::{capture_audio, list_input_devices, transcribe, WhisperState};
use serde::{Deserialize, Serialize};

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
use std::sync::{mpsc, Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

struct AppState {
    whisper_state: Arc<Mutex<WhisperState>>,
    settings: Mutex<AppSettings>,
    device_tx: Mutex<mpsc::Sender<Option<String>>>,
    typer_tx: mpsc::Sender<String>,
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
    let mut ws = state.whisper_state.lock().unwrap();
    ws.is_recording = !ws.is_recording;
    let is_recording = ws.is_recording;

    // Notify frontend with absolute state
    let _ = app.emit("recording-toggled", is_recording);

    if !is_recording {
        // Stopped recording, start transcription
        let audio_data = ws.audio_buffer.clone();
        ws.audio_buffer.clear();
        drop(ws); // Release lock during transcription

        let tx = state.typer_tx.clone();
        let settings = state.settings.lock().unwrap().clone();
        let app = app.clone(); // Clone app handle for the spawned thread

        std::thread::spawn(move || {
            let model_name = &settings.selected_model;
            let use_gpu = settings.device == "cuda";

            // Safety check: English-only model with non-English language selected
            if (settings.selected_language != "en" && settings.selected_language != "auto")
                && model_name.ends_with(".en")
            {
                let _ = app.emit(
                    "status-update",
                    format!(
                        "Warning: {} is English-only. Transcription may fail.",
                        model_name
                    ),
                );
            }
            if settings.selected_language == "auto" && model_name.ends_with(".en") {
                let _ = app.emit(
                    "status-update",
                    format!(
                        "Warning: Auto-detect requires a multilingual model. {} is English-only.",
                        model_name
                    ),
                );
            }

            let model_filename = format!("ggml-{}.bin", model_name);

            // Resolve model path in App Data directory for reliability
            let model_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            if !model_dir.exists() {
                let _ = std::fs::create_dir_all(&model_dir);
            }
            let model_path = model_dir.join(&model_filename);

            if !model_path.exists() {
                // If not in App Data, check current/src-tauri for dev compatibility
                let dev_path = std::path::PathBuf::from("src-tauri").join(&model_filename);
                if dev_path.exists() {
                    // Just use the dev path if it exists
                    println!("Using dev model path: {:?}", dev_path);
                    perform_transcription(
                        &app,
                        &tx,
                        &dev_path,
                        &audio_data,
                        &settings.selected_language,
                        &settings.languages,
                        use_gpu,
                    );
                    return;
                }

                // Try to download it if truly missing
                println!(
                    "Required model {} missing. Attempting download...",
                    model_filename
                );
                let _ = app.emit(
                    "status-update",
                    format!("Downloading {} model (75MB)...", model_name),
                );

                let url = format!(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
                    model_name
                );
                match reqwest::blocking::get(url) {
                    Ok(mut response) if response.status().is_success() => {
                        if let Ok(mut f) = std::fs::File::create(&model_path) {
                            if let Ok(_) = std::io::copy(&mut response, &mut f) {
                                println!("Download complete: {:?}", model_path);
                                let _ = app
                                    .emit("status-update", format!("{} model ready.", model_name));
                            } else {
                                eprintln!("Failed to write model file.");
                            }
                        } else {
                            eprintln!("Failed to create model file at {:?}", model_path);
                        }
                    }
                    Ok(response) => eprintln!("Download failed with status: {}", response.status()),
                    Err(e) => eprintln!("Download request error: {}", e),
                }
            }

            if model_path.exists() {
                perform_transcription(
                    &app,
                    &tx,
                    &model_path,
                    &audio_data,
                    &settings.selected_language,
                    &settings.languages,
                    use_gpu,
                );
            } else {
                eprintln!("Model not found at {:?}", model_path);
                let _ = app.emit("status-update", "Model missing. Please check connection.");
            }
        });
    }
}

fn perform_transcription(
    app: &tauri::AppHandle,
    tx: &mpsc::Sender<String>,
    model_path: &std::path::Path,
    audio_data: &[f32],
    lang: &str,
    allowed_langs: &[String],
    use_gpu: bool,
) {
    match transcribe(
        model_path.to_str().unwrap(),
        audio_data,
        lang,
        allowed_langs,
        use_gpu,
    ) {
        Ok(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty()
                && !trimmed.contains("[BLANK_AUDIO]")
                && !trimmed.contains("[SILENCE]")
            {
                println!("Transcribed text: {}", trimmed);
                let _ = tx.send(trimmed.to_string());
            }
        }
        Err(e) => {
            eprintln!("Transcription error: {}", e);
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

        println!("Downloading model {}...", model);
        let _ = app_clone.emit("model-download-status", format!("Downloading {}...", model));

        let url = format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
            model
        );
        match reqwest::blocking::get(url) {
            Ok(mut response) if response.status().is_success() => {
                if let Ok(mut f) = std::fs::File::create(&model_path) {
                    if let Ok(_) = std::io::copy(&mut response, &mut f) {
                        println!("Download complete: {:?}", model_path);
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, _rx) = mpsc::channel::<Option<String>>();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if shortcut.key == Code::F2 {
                            let state = app.state::<AppState>();
                            toggle_recording(app.clone(), state);
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
            });

            let state = app.state::<AppState>();
            let whisper_state = state.whisper_state.clone();

            // Audio Manager Thread
            let (device_tx, device_rx) = mpsc::channel::<Option<String>>();
            *state.device_tx.lock().unwrap() = device_tx;

            let whisper_state_for_audio = whisper_state.clone();
            let initial_device = settings.input_device.clone();

            std::thread::spawn(move || {
                let mut current_stream: Option<cpal::Stream> = None;

                // Initialize with saved device or default
                if let Ok(stream) = capture_audio(whisper_state_for_audio.clone(), initial_device) {
                    current_stream = Some(stream);
                }

                while let Ok(device_name) = device_rx.recv() {
                    current_stream = None; // Drop old stream
                    if let Ok(stream) = capture_audio(whisper_state_for_audio.clone(), device_name)
                    {
                        current_stream = Some(stream);
                    }
                }
            });

            // Stream amplitude to pill UI
            let app_handle = app.handle().clone();
            let whisper_state_for_amp = state.whisper_state.clone();
            std::thread::spawn(move || loop {
                let mut ws = whisper_state_for_amp.lock().unwrap();
                let amp = ws.current_amplitude;
                ws.current_amplitude = 0.0; // Reset for peak detection
                drop(ws);

                let _ = app_handle.emit("audio-amplitude", amp);
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

            // Register F2 shortcut
            if let Err(e) = app
                .global_shortcut()
                .register(Shortcut::new(None, Code::F2))
            {
                println!("Failed to register F2 shortcut: {}", e);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

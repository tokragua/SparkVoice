mod commands;
mod errors;
mod models;
mod network_trigger;
mod settings;
mod stats;
mod whisper;
pub mod db;
pub mod llm;
pub mod migration;

use crate::commands::*;
use crate::models::*;
use crate::settings::{load_settings, save_settings, AppSettings};
use crate::whisper::{capture_audio, WhisperState};

use enigo::{Enigo, Keyboard, Settings};
use log::{error, info, warn};
use parking_lot::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::Instant;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Listener, Manager,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_log::{Target, TargetKind};
use whisper_rs::WhisperContext;

// ── Shared Application State ────────────────────────────────────────────────

pub struct AppState {
    pub whisper_state: Arc<Mutex<WhisperState>>,
    pub settings: Mutex<AppSettings>,
    pub device_tx: Mutex<mpsc::Sender<Option<String>>>,
    pub typer_tx: mpsc::Sender<String>,
    pub is_transcribing: Mutex<bool>,
    pub is_migrating: Arc<AtomicBool>,
    pub is_cancelled: Arc<AtomicBool>,
    /// (model_name, use_gpu, context)
    pub model_cache: Mutex<Option<(String, bool, Arc<WhisperContext>)>>,
    /// Last time pill position was persisted to disk (for debouncing)
    pub pill_save_timer: Mutex<Instant>,
}

// ── Application Entry Point ─────────────────────────────────────────────────

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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app
                .get_webview_window("main")
                .expect("no main window")
                .set_focus();
        }))
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

            // Initialize local SQLite database
            if let Err(e) = db::init_db(app.handle()) {
                log::error!("Failed to initialize database: {}", e);
            }

            // Run historical JSON migration in the background
            let app_handle_for_migration = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = migration::run_historical_migration(&app_handle_for_migration) {
                    log::error!("Historical migration failed: {}", e);
                }
            });

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
                is_migrating: Arc::new(AtomicBool::new(false)),
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
            let mind_map_i = MenuItem::with_id(app, "mind_map", "Mind Map", true, None::<&str>)?;
            let show_pill_i = tauri::menu::CheckMenuItem::with_id(
                app,
                "show_pill",
                "Show Pill",
                true,
                settings.show_pill,
                None::<&str>,
            )?;
            let menu = Menu::with_items(app, &[&show_pill_i, &settings_i, &mind_map_i, &quit_i])?;

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
                    "mind_map" => {
                        if let Some(window) = app.get_webview_window("mind_map") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "show_pill" => {
                        let state = app.state::<AppState>();
                        let mut settings = state.settings.lock();
                        settings.show_pill = !settings.show_pill;
                        let is_showing = settings.show_pill;
                        crate::settings::save_settings(app, &settings);
                        drop(settings);
                        if let Some(pill) = app.get_webview_window("pill") {
                            if is_showing {
                                let _ = pill.show();
                            } else {
                                let _ = pill.hide();
                            }
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

            // Apply saved pill configure
            if let Some(pill) = app.get_webview_window("pill") {
                let _ = pill.set_focusable(false);
                let state = app.state::<AppState>();
                let settings = state.settings.lock();

                // Initialize Knowledge Graph Database
                let _ = db::init_db(app.handle());

                // Run historical migration in background
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(e) = migration::run_historical_migration(&app_handle) {
                        error!("Historical migration failed: {}", e);
                    }
                });

                let _ = pill.set_position(tauri::PhysicalPosition::new(
                    settings.pill_x as i32,
                    settings.pill_y as i32,
                ));
                if !settings.show_pill {
                    let _ = pill.hide();
                }
            }

            // Auto-start network trigger server if enabled
            if settings.network_trigger_enabled {
                network_trigger::start_server(app.handle());
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
                if window.label() == "main" || window.label() == "mind_map" {
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
            get_stats,
            set_network_trigger,
            set_network_trigger_password,
            set_network_trigger_port,
            get_local_ip,
            set_network_trigger_return_text,
            set_transcription_logging,
            get_transcription_logs,
            open_mind_map,
            set_llm_mind_map,
            set_llm_api_url,
            set_llm_model,
            set_llm_node_cap,
            get_mind_map_graph,
            clear_mind_map_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

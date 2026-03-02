use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::AppState;
use crate::commands::stop_and_transcribe;

/// Shared handle to stop the server thread gracefully
struct ServerHandle {
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

static SERVER_STATE: std::sync::OnceLock<std::sync::Mutex<Option<ServerHandle>>> =
    std::sync::OnceLock::new();

fn get_server_state() -> &'static std::sync::Mutex<Option<ServerHandle>> {
    SERVER_STATE.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn start_server(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().clone();

    if !settings.network_trigger_enabled {
        return;
    }

    // Stop any existing server first (waits for thread to finish)
    stop_server();

    let port = settings.network_trigger_port;
    let password = settings.network_trigger_password.clone();
    let app_handle = app.clone();
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let handle = std::thread::spawn(move || {
        let addr = format!("0.0.0.0:{}", port);
        let server = match tiny_http::Server::http(&addr) {
            Ok(s) => {
                info!("Network Trigger API started on {}", addr);
                s
            }
            Err(e) => {
                error!("Failed to start Network Trigger API on {}: {}", addr, e);
                return;
            }
        };

        // Set a short timeout so we can check the running flag periodically
        while running_clone.load(Ordering::SeqCst) {
            let request = match server.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(Some(req)) => req,
                Ok(None) => continue, // timeout, check running flag
                Err(e) => {
                    error!("Network Trigger recv error: {}", e);
                    break;
                }
            };

            // Check Bearer auth if password is set
            if !password.is_empty() {
                let auth_value = request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str() == "Authorization")
                    .map(|h| h.value.as_str().to_string());

                let expected = format!("Bearer {}", password);
                match auth_value {
                    Some(ref val) if val == &expected => {} // Auth OK
                    _ => {
                        let response = tiny_http::Response::from_string(
                            r#"{"error":"Unauthorized"}"#,
                        )
                        .with_status_code(401)
                        .with_header(
                            "Content-Type: application/json"
                                .parse::<tiny_http::Header>()
                                .unwrap(),
                        );
                        let _ = request.respond(response);
                        continue;
                    }
                }
            }

            let method = request.method().to_string();
            let path = request.url().to_string();

            if method != "POST" {
                let response = tiny_http::Response::from_string(
                    r#"{"error":"Method not allowed. Use POST."}"#,
                )
                .with_status_code(405)
                .with_header(
                    "Content-Type: application/json"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                );
                let _ = request.respond(response);
                continue;
            }

            let result = match path.as_str() {
                "/start" => handle_start(&app_handle),
                "/stop" => handle_stop(&app_handle),
                "/toggle" => handle_toggle(&app_handle),
                _ => {
                    let response = tiny_http::Response::from_string(
                        r#"{"error":"Not found. Available endpoints: POST /start, POST /stop, POST /toggle"}"#,
                    )
                    .with_status_code(404)
                    .with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                    let _ = request.respond(response);
                    continue;
                }
            };

            let (status, body) = match result {
                Ok((action, text_opt)) => {
                    let mut json = format!(r#"{{"status":"ok","action":"{}"}}"#, action);
                    if let Some(text) = text_opt {
                        // Build JSON with text field
                        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
                        json = format!(r#"{{"status":"ok","action":"{}","text":"{}"}}"#, action, escaped);
                    }
                    (200, json)
                }
                Err(msg) => (500, format!(r#"{{"error":"{}"}}"#, msg)),
            };

            let response = tiny_http::Response::from_string(body)
                .with_status_code(status)
                .with_header(
                    "Content-Type: application/json"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                );
            let _ = request.respond(response);
        }

        // Server is dropped here, releasing the socket
        drop(server);
        info!("Network Trigger API stopped.");
    });

    {
        let mut state = get_server_state().lock().unwrap();
        *state = Some(ServerHandle {
            running,
            thread: Some(handle),
        });
    }
}

pub fn stop_server() {
    let mut state = get_server_state().lock().unwrap();
    if let Some(mut handle) = state.take() {
        handle.running.store(false, Ordering::SeqCst);
        // Wait for the thread to fully exit and release the socket
        if let Some(thread) = handle.thread.take() {
            let _ = thread.join();
        }
        info!("Network Trigger API shutdown complete.");
    }
}

fn handle_start(app: &AppHandle) -> Result<(String, Option<String>), String> {
    let state = app.state::<AppState>();
    let mut ws = state.whisper_state.lock();
    if ws.is_recording {
        return Ok(("already_recording".into(), None));
    }
    let max_recording_seconds = state.settings.lock().max_recording_seconds;
    ws.is_recording = true;
    ws.max_samples = Some(max_recording_seconds as usize * 16000);
    ws.audio_buffer.clear();
    drop(ws);

    // If starting recording and pill is hidden, force show it
    let is_hidden = !state.settings.lock().show_pill;
    if is_hidden {
        if let Some(pill) = app.get_webview_window("pill") {
            let _ = pill.show();
        }
    }

    let _ = app.emit("recording-toggled", true);
    info!("Network Trigger: recording started");
    Ok(("started".into(), None))
}

fn handle_stop(app: &AppHandle) -> Result<(String, Option<String>), String> {
    let state = app.state::<AppState>();
    let return_text = state.settings.lock().network_trigger_return_text;
    let was_recording = {
        let mut ws = state.whisper_state.lock();
        let was = ws.is_recording;
        ws.is_recording = false;
        was
    };

    if was_recording {
        let _ = app.emit("recording-toggled", false);

        if return_text {
            // Set up a channel to receive the transcribed text
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            let event_id = app.listen("transcribed-text", move |event| {
                // Payload is JSON-serialized string, e.g. "\"hello world\""
                let payload = event.payload().to_string();
                // Strip surrounding quotes from the JSON string
                let text = payload.trim_matches('"').to_string();
                let _ = tx.send(text);
            });

            stop_and_transcribe(app.clone());

            // Wait up to 120 seconds for transcription to complete
            let text = match rx.recv_timeout(std::time::Duration::from_secs(120)) {
                Ok(t) => Some(t),
                Err(_) => {
                    info!("Network Trigger: transcription timed out waiting for text");
                    None
                }
            };
            app.unlisten(event_id);

            info!("Network Trigger: recording stopped (with text)");
            Ok(("stopped".into(), text))
        } else {
            stop_and_transcribe(app.clone());
            info!("Network Trigger: recording stopped");
            Ok(("stopped".into(), None))
        }
    } else {
        Ok(("not_recording".into(), None))
    }
}

fn handle_toggle(app: &AppHandle) -> Result<(String, Option<String>), String> {
    let state = app.state::<AppState>();
    let is_recording = {
        let ws = state.whisper_state.lock();
        ws.is_recording
    };

    if is_recording {
        handle_stop(app)
    } else {
        handle_start(app)
    }
}

/// Get the local network IP address
pub fn get_local_ip() -> String {
    // Try to find a non-loopback IPv4 address by connecting to a public address
    // This doesn't actually send any data, just determines the route
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

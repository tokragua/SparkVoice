use serde::Serialize;
use std::fmt;

/// Typed error enum for all SparkVoice backend operations.
/// Implements `Into<tauri::ipc::InvokeError>` via `Serialize` so Tauri
/// can automatically convert these into frontend error payloads.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "message")]
#[allow(dead_code)]
pub enum AppError {
    /// Invalid or unknown model name
    InvalidModel(String),
    /// File system operation failed
    Io(String),
    /// Model download or network failure
    Download(String),
    /// Hash verification failed after download
    IntegrityCheck(String),
    /// Whisper engine initialization or transcription error
    Whisper(String),
    /// Settings or shortcut configuration error
    Config(String),
    /// Audio device error
    AudioDevice(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::InvalidModel(msg) => write!(f, "Invalid model: {}", msg),
            AppError::Io(msg) => write!(f, "I/O error: {}", msg),
            AppError::Download(msg) => write!(f, "Download error: {}", msg),
            AppError::IntegrityCheck(msg) => write!(f, "Integrity check failed: {}", msg),
            AppError::Whisper(msg) => write!(f, "Whisper error: {}", msg),
            AppError::Config(msg) => write!(f, "Config error: {}", msg),
            AppError::AudioDevice(msg) => write!(f, "Audio device error: {}", msg),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

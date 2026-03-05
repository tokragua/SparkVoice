use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::llm;
use crate::settings;
use crate::stats::TranscriptionLogEntry;

/// Scans the transcription_logs directory, reads JSON files, and processes
/// any that haven't been ingested into the SQLite graph database yet.
pub fn run_historical_migration(app: &AppHandle) -> Result<()> {
    let settings = settings::load_settings(app);

    if !settings.llm_mind_map_enabled {
        log::info!("LLM Mind Map is disabled. Skipping historical migration.");
        return Ok(());
    }

    let state = app.state::<crate::AppState>();
    // Set migrating flag
    state.is_migrating.store(true, Ordering::SeqCst);

    // Ensure we reset the flag when done, even if we return early
    let _migrating_guard = scopeguard::guard(state.is_migrating.clone(), |flag| {
        flag.store(false, Ordering::SeqCst);
    });

    let log_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("transcription_logs");

    if !log_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(log_dir)?;

    for entry_result in entries {
        let entry = entry_result?;
        let path = entry.path();

        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            // Check current progress for this file in SQLite
            let already_indexed = match db::get_log_parsing_index(app, &filename) {
                Ok(idx) => idx,
                Err(e) => {
                    log::error!("Error checking progress for {}: {}", filename, e);
                    0
                }
            };

            // Read the JSON file
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to read {}: {}", filename, e);
                    continue;
                }
            };

            let log_entries: Vec<TranscriptionLogEntry> = match serde_json::from_str(&content) {
                Ok(logs) => logs,
                Err(e) => {
                    log::error!("Failed to parse JSON in {}: {}", filename, e);
                    continue;
                }
            };

            // Handle legacy "Done" files (already in parsed_logs but index is 0)
            if already_indexed == 0 && db::has_log_file_been_parsed(app, &filename).unwrap_or(false) {
                log::info!("File {} was previously marked as fully parsed. Updating index to {} and skipping.", filename, log_entries.len());
                let _ = db::update_log_parsing_index(app, &filename, log_entries.len());
                continue;
            }

            if already_indexed >= log_entries.len() {
                log::debug!("File {} already up to date ({} entries), skipping.", filename, already_indexed);
                continue;
            }

            if already_indexed > 0 {
                log::info!("Resuming indexing for {}: starting from entry {}, total {}", filename, already_indexed, log_entries.len());
            } else {
                log::info!("Processing fresh log file: {}", filename);
            }

            let new_entries = &log_entries[already_indexed..];
            let chunks = new_entries.chunks(5);
            let total_chunks = chunks.len();

            log::info!("Migrating {} ({} remaining entries, {} chunks)", filename, new_entries.len(), total_chunks);

            let mut current_file_index = already_indexed;

            for (i, chunk) in chunks.enumerate() {
                let mut chunk_text = String::new();
                for entry in chunk {
                    chunk_text.push_str(&entry.text);
                    chunk_text.push_str(" ");
                }

                if chunk_text.trim().is_empty() {
                    current_file_index += chunk.len();
                    let _ = db::update_log_parsing_index(app, &filename, current_file_index);
                    continue;
                }

                log::info!("  - Processing chunk {}/{} (index {}..{})", i + 1, total_chunks, current_file_index, current_file_index + chunk.len());
                
                // Implement simple retry (3x)
                let mut success = false;
                for attempt in 1..=3 {
                    match llm::extract_knowledge(&settings, &chunk_text) {
                        Ok(triplets) => {
                            if let Err(e) = db::upsert_triplets(app, &triplets, &filename) {
                                log::error!("    Failed to save triplets for chunk {} of {}: {}", i + 1, filename, e);
                            } else {
                                success = true;
                                // Update progress in DB after each successful chunk
                                current_file_index += chunk.len();
                                if let Err(e) = db::update_log_parsing_index(app, &filename, current_file_index) {
                                    log::error!("    Failed to update indexing progress for {}: {}", filename, e);
                                }
                            }
                            break;
                        }
                        Err(e) => {
                            log::warn!("    LLM extraction failed for chunk {} (attempt {}/3): {}", i + 1, attempt, e);
                            if attempt < 3 {
                                thread::sleep(Duration::from_secs(2));
                            }
                        }
                    }
                }
                
                if !success {
                    log::error!("    Aborted chunk {}/{} after max retries. Stopping file processing for now.", i + 1, total_chunks);
                    break;
                }
            }
        }
    }

    log::info!("Historical JSON migration check complete.");
    Ok(())
}

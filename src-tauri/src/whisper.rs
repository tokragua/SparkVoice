use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{error, info, warn};
use std::sync::{Arc, Mutex};
use tauri::Emitter;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

pub struct WhisperState {
    pub is_recording: bool,
    pub audio_buffer: Vec<f32>,
    pub current_amplitude: f32,
    pub max_samples: Option<usize>,
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let devices = host.input_devices().unwrap();
    devices.map(|d| d.name().unwrap_or_default()).collect()
}

pub fn capture_audio(
    app_handle: tauri::AppHandle,
    state: Arc<Mutex<WhisperState>>,
    device_name: Option<String>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = if let Some(name) = device_name {
        host.input_devices()?
            .filter_map(|d| d.name().ok().map(|n| (d, n)))
            .find(|(_, n)| n == &name)
            .map(|(d, _)| d)
            .ok_or_else(|| anyhow::anyhow!("Device not found"))?
    } else {
        host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No input device found"))?
    };

    let config = device.default_input_config()?;
    let sample_format = config.sample_format();
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;

    info!(
        "Capturing audio: {}Hz, {} channels, format {:?}",
        sample_rate, channels, sample_format
    );

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| process_audio(data, &state, sample_rate, channels, &app_handle),
            |err| error!("Audio capture error: {}", err),
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _| {
                let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                process_audio(&f32_data, &state, sample_rate, channels, &app_handle)
            },
            |err| error!("Audio capture error: {}", err),
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config.into(),
            move |data: &[u16], _| {
                let f32_data: Vec<f32> = data
                    .iter()
                    .map(|&s| (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
                    .collect();
                process_audio(&f32_data, &state, sample_rate, channels, &app_handle)
            },
            |err| error!("Audio capture error: {}", err),
            None,
        )?,
        _ => return Err(anyhow::anyhow!("Unsupported sample format")),
    };

    stream.play()?;
    Ok(stream)
}

fn process_audio(
    data: &[f32],
    state: &Arc<Mutex<WhisperState>>,
    sample_rate: f32,
    channels: usize,
    app_handle: &tauri::AppHandle,
) {
    let mut s = state.lock().unwrap();

    // Calculate RMS for visualization
    let len = data.len();
    if len > 0 {
        let sum: f32 = data.iter().map(|&sample| sample * sample).sum();
        let rms = (sum / len as f32).sqrt();

        // Peak detection: keep the highest value since it was last reset
        if rms > s.current_amplitude {
            s.current_amplitude = rms;
        }
    }

    if s.is_recording {
        // 1. Mono mixing
        let mono_data: Vec<f32> = if channels > 1 {
            data.chunks_exact(channels)
                .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                .collect()
        } else {
            data.to_vec()
        };

        // 2. Resampling to 16kHz
        if sample_rate == 16000.0 {
            s.audio_buffer.extend_from_slice(&mono_data);
        } else {
            let ratio = sample_rate / 16000.0;
            let mut i = 0.0;
            while (i as usize) < mono_data.len() {
                let idx = i as usize;
                let frac = i - idx as f32;

                let sample = if idx + 1 < mono_data.len() {
                    mono_data[idx] * (1.0 - frac) + mono_data[idx + 1] * frac
                } else {
                    mono_data[idx]
                };

                s.audio_buffer.push(sample);
                i += ratio;
            }
        }

        // Safety: Dynamic Recording Limit
        let limit = s.max_samples.unwrap_or(480_000); // Default 30s if not set
        if s.audio_buffer.len() >= limit {
            s.is_recording = false;
            let seconds = limit / 16000;
            warn!(
                "Recording limit reached ({}s). Stopping automatically.",
                seconds
            );
            let _ = app_handle.emit("recording-auto-stopped", ());
        }
    }
}

pub fn transcribe(
    ctx: &WhisperContext,
    audio_data: &[f32],
    lang: &str,
    allowed_langs: &[String],
) -> Result<String> {
    let mut state = ctx.create_state()?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // Optimize CPU performance with multi-threading
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let threads = std::cmp::min(threads, 8) as i32; // Cap at 8 threads to avoid contention
    params.set_n_threads(threads);

    if lang == "auto" {
        params.set_language(None);
    } else {
        params.set_language(Some(lang));
    }
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    info!("Processing transcription with {} CPU threads...", threads);
    let start = std::time::Instant::now();
    state.full(params, audio_data)?;
    let duration = start.elapsed();
    info!(
        "Transcription compute finished in {:.2}s",
        duration.as_secs_f32()
    );

    if lang == "auto" && !allowed_langs.is_empty() {
        if let Ok(id) = state.full_lang_id_from_state() {
            if let Some(detected_lang) = whisper_rs::get_lang_str(id) {
                info!("Detected language: {}", detected_lang);
                if !allowed_langs.contains(&detected_lang.to_string()) {
                    let fallback_lang = allowed_langs
                        .iter()
                        .find(|&l| l != "en")
                        .cloned()
                        .unwrap_or_else(|| allowed_langs[0].clone());

                    info!(
                        "Detected language '{}' not allowed. Forcing fallback: {}",
                        detected_lang, fallback_lang
                    );

                    let mut retry_params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                    retry_params.set_n_threads(threads); // Keep same thread count
                    retry_params.set_language(Some(&fallback_lang));
                    retry_params.set_print_special(false);
                    retry_params.set_print_progress(false);
                    retry_params.set_print_realtime(false);
                    retry_params.set_print_timestamps(false);

                    state.full(retry_params, audio_data)?;
                }
            }
        }
    }

    let mut result = String::new();
    let num_segments = state.full_n_segments()?;
    for i in 0..num_segments {
        if let Ok(segment) = state.full_get_segment_text(i) {
            result.push_str(&segment);
        }
    }

    Ok(result)
}
